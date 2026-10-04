import re

import pytest

from product_stream.catalog_view import PRODUCT_COLUMNS
from product_stream.generator import ProductStream
from product_stream.sinks import PostgresSink

DSN = "postgresql://user:s3cr3t-pw@db.example.invalid:5432/pulso"


class FakeCur:
    def __init__(self, log):
        self.log = log

    def execute(self, sql, params=None):
        self.log.append((sql, params))

    def executemany(self, sql, seq):
        self.log.append((sql, list(seq)))

    def fetchone(self):
        return (0,)

    def __enter__(self):
        return self

    def __exit__(self, *a):
        return False


class FakeConn:
    def __init__(self):
        self.log = []
        self.commits = 0
        self.closed = False

    def cursor(self):
        return FakeCur(self.log)

    def commit(self):
        self.commits += 1

    def close(self):
        self.closed = True


def sink(monkeypatch, conn):
    monkeypatch.setenv("PULSO_PRODUCT_SIM_PG_DSN", DSN)
    seen = []
    s = PostgresSink(connect=lambda dsn: (seen.append(dsn), conn)[1], run_id="r1")
    return s, seen


def test_dsn_comes_from_env_and_missing_env_fails_without_leak(monkeypatch):
    monkeypatch.delenv("PULSO_PRODUCT_SIM_PG_DSN", raising=False)
    with pytest.raises(RuntimeError) as e:
        PostgresSink(connect=lambda d: FakeConn())
    assert "PULSO_PRODUCT_SIM_PG_DSN" in str(e.value)
    conn = FakeConn()
    s, seen = sink(monkeypatch, conn)
    assert seen == [DSN]
    assert DSN not in repr(s) and "s3cr3t" not in repr(s)


def test_ensure_schema_only_allow_listed_tables_plus_lineage(monkeypatch):
    conn = FakeConn()
    s, _ = sink(monkeypatch, conn)
    s.ensure_schema()
    ddl = " ".join(sql for sql, _ in conn.log)
    for t, cols in PRODUCT_COLUMNS.items():
        assert f"CREATE TABLE IF NOT EXISTS product.{t} " in ddl
    created = set(re.findall(r"CREATE TABLE IF NOT EXISTS product\.(\w+)", ddl))
    assert created == set(PRODUCT_COLUMNS)
    assert "_batch_id" in ddl and "jsonb" in ddl


def test_write_batch_inserts_allow_listed_columns_with_lineage_and_commits(monkeypatch):
    conn = FakeConn()
    s, _ = sink(monkeypatch, conn)
    g = ProductStream(seed=2, horizon_events=100)
    batch = g.next_batch(40)
    s.write_batch(batch)
    assert conn.commits == 1
    inserts = [(sql, p) for sql, p in conn.log if sql.startswith("INSERT INTO product.")]
    assert inserts
    for sql, params in inserts:
        t = re.match(r"INSERT INTO product\.(\w+) \(([^)]*)\)", sql)
        cols = [c.strip() for c in t.group(2).split(",")]
        assert t.group(1) in PRODUCT_COLUMNS
        assert set(cols) <= set(PRODUCT_COLUMNS[t.group(1)]) | {"_batch_id", "_source_file"}
        assert "_batch_id" in cols
    el = next(p for sql, p in inserts if "product.event_log" in sql)
    assert len(el) == 40 and el[0][-2].startswith("product-sim:r1:")


def test_write_batch_refuses_non_allow_listed(monkeypatch):
    s, _ = sink(monkeypatch, FakeConn())
    with pytest.raises(ValueError):
        s.write_batch({"login_accounts": [{"id": "x"}]})
    with pytest.raises(ValueError):
        s.write_batch({"staff": [{"id": "x", "email": "a@b"}]})
