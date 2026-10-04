import contextlib
import json

import pytest

import load_parquet as lp


class FakeCopy:
    def __init__(self, sink):
        self.sink = sink

    def write_row(self, row):
        self.sink.append(tuple(row))

    def __enter__(self):
        return self

    def __exit__(self, *a):
        return False


class FakeCursor:
    def __init__(self, conn):
        self.conn = conn

    def execute(self, sql, params=None):
        self.conn.log.append(("execute", sql, params))

    def copy(self, sql):
        self.conn.log.append(("copy", sql, None))
        return FakeCopy(self.conn.rows)

    def __enter__(self):
        return self

    def __exit__(self, *a):
        return False


class FakeConn:
    def __init__(self):
        self.log, self.rows = [], []

    def cursor(self):
        return FakeCursor(self)

    @contextlib.contextmanager
    def transaction(self):
        self.log.append(("begin", "", None))
        yield
        self.log.append(("commit", "", None))


def reader(rows):
    return lambda path, columns: iter([rows])


def test_refuses_evaluator_tables():
    for t in ("labels", "timeline", "e0_labels", "e0_timeline", "pseudonym_map"):
        with pytest.raises(lp.LoaderRefusal):
            lp.build_plan("raw", t, ["case_id"], "b1", "f.parquet")


def test_refuses_unknown_table_and_schema():
    with pytest.raises(lp.LoaderRefusal):
        lp.build_plan("raw", "nope", ["x"], "b", "f")
    with pytest.raises(lp.LoaderRefusal):
        lp.build_plan("pulso", "e0_case", ["case_id"], "b", "f")


def test_refuses_unknown_columns_naming_the_column_only():
    with pytest.raises(lp.LoaderRefusal) as e:
        lp.build_plan("raw", "e0_case", ["case_id", "ssn"], "b", "f")
    assert "ssn" in str(e.value)


def test_refuses_lineage_columns_in_file():
    with pytest.raises(lp.LoaderRefusal):
        lp.build_plan("raw", "e0_case", ["case_id", "_batch_id"], "b", "f")


def test_plan_copy_lists_columns_and_lineage():
    p = lp.build_plan("raw", "e0_case", ["case_id", "priority"], "b1", "x.parquet")
    assert p.delete_sql == "DELETE FROM raw.e0_case WHERE _batch_id = %s"
    assert p.copy_sql == "COPY raw.e0_case (case_id, priority, _batch_id, _source_file) FROM STDIN"


def test_load_is_one_transaction_delete_then_copy_and_counts():
    conn = FakeConn()
    res = lp.load_file(conn, "raw", "e0_case", "dir/x.parquet", batch_id="b1",
                       file_columns=["case_id", "priority"], reader=reader([("c1", "high"), ("c2", "low")]))
    assert res.rows == 2 and res.batch_id == "b1"
    assert [k for k, *_ in conn.log] == ["begin", "execute", "copy", "commit"]
    assert conn.log[1][2] == ("b1",)
    assert conn.rows == [("c1", "high", "b1", "x.parquet"), ("c2", "low", "b1", "x.parquet")]


def test_rerun_same_batch_deletes_first_so_idempotent():
    conn = FakeConn()
    for _ in range(2):
        lp.load_file(conn, "raw", "e0_case", "x.parquet", batch_id="b1", file_columns=["case_id"],
                     reader=reader([("c1",)]))
    execs = [l for l in conn.log if l[0] == "execute"]
    assert len(execs) == 2 and all(l[1].startswith("DELETE") for l in execs)


def test_jsonb_values_serialised_and_none_kept():
    conn = FakeConn()
    lp.load_file(conn, "raw", "e0_turn", "t.parquet", batch_id="b", file_columns=["turn_id", "evidence_ids"],
                 reader=reader([("t1", ["a", "b"]), ("t2", None)]))
    assert conn.rows[0][1] == json.dumps(["a", "b"])
    assert conn.rows[1][1] is None


def test_never_prints_row_values(capsys):
    lp.load_file(FakeConn(), "raw", "e0_turn", "t.parquet", batch_id="b", file_columns=["turn_id", "text"],
                 reader=reader([("t1", "SYNTH-VALUE-123")]))
    out = capsys.readouterr()
    assert "SYNTH-VALUE-123" not in out.out + out.err


def test_error_message_has_no_values():
    def bad_reader(path, columns):
        raise ValueError("bad value SYNTH-VALUE-123")

    with pytest.raises(lp.LoaderError) as e:
        lp.load_file(FakeConn(), "raw", "e0_turn", "t.parquet", batch_id="b", file_columns=["turn_id"],
                     reader=bad_reader)
    assert "SYNTH-VALUE-123" not in str(e.value) and "SYNTH-VALUE-123" not in repr(e.value.__cause__ or "")


def test_default_batch_id_is_content_hash(tmp_path):
    f, g = tmp_path / "a.parquet", tmp_path / "b.parquet"
    f.write_bytes(b"abc")
    g.write_bytes(b"abd")
    assert lp.default_batch_id(str(f)) == lp.default_batch_id(str(f))
    assert lp.default_batch_id(str(f)) != lp.default_batch_id(str(g))


def test_generic_over_product_schema():
    p = lp.build_plan("product", "event_log", ["sequence", "event_id"], "b", "f")
    assert p.copy_sql.startswith("COPY product.event_log (sequence, event_id")
