"""Sinks that write allow-listed rows only. SQLite (file) and Postgres (psycopg, DSN from an env var)."""
from __future__ import annotations

import json
import os
import sqlite3

from .catalog_view import PRODUCT_COLUMNS, PRODUCT_TYPES

_SQLITE_TYPE = {"text": "TEXT", "timestamptz": "TEXT", "jsonb": "TEXT", "integer": "INTEGER", "bigint": "INTEGER",
                "double precision": "REAL", "boolean": "INTEGER"}


def validate_batch(rows_by_table: dict[str, list[dict]]) -> None:
    for t, rows in rows_by_table.items():
        if t not in PRODUCT_COLUMNS:
            raise ValueError(f"table not in the product allow-list: {t}")
        allowed = set(PRODUCT_COLUMNS[t])
        for r in rows:
            extra = set(r) - allowed
            if extra:
                raise ValueError(f"columns outside the allow-list for {t}: {sorted(extra)}")


def _val(v):
    if isinstance(v, (dict, list)):
        return json.dumps(v, sort_keys=True, separators=(",", ":"))
    if isinstance(v, bool):
        return int(v)
    return v


class SqliteSink:
    def __init__(self, path: str):
        self.path = path
        self.con = sqlite3.connect(path)

    def ensure_schema(self) -> None:
        for t, cols in PRODUCT_COLUMNS.items():
            defs = []
            for c in cols:
                d = f"{c} {_SQLITE_TYPE[PRODUCT_TYPES[t][c]]}"
                if (t, c) == ("event_log", "sequence"):
                    d += " PRIMARY KEY"
                defs.append(d)
            self.con.execute(f"CREATE TABLE IF NOT EXISTS {t} ({', '.join(defs)})")
        self.con.commit()

    def event_log_count(self) -> int:
        return self.con.execute("SELECT count(*) FROM event_log").fetchone()[0]

    def write_batch(self, rows_by_table: dict[str, list[dict]]) -> None:
        validate_batch(rows_by_table)
        for t, rows in rows_by_table.items():
            for r in rows:
                if t == "customer_case_slots":  # mutable current state: one row per customer
                    self.con.execute("DELETE FROM customer_case_slots WHERE customer_id=?", (r["customer_id"],))
                cols = [c for c in PRODUCT_COLUMNS[t] if c in r]
                self.con.execute(f"INSERT INTO {t} ({','.join(cols)}) VALUES ({','.join('?' * len(cols))})",
                                 [_val(r[c]) for c in cols])
        self.con.commit()

    def reset(self) -> None:
        for t in PRODUCT_COLUMNS:
            self.con.execute(f"DELETE FROM {t}")
        self.con.commit()

    def close(self) -> None:
        self.con.close()


DEFAULT_DSN_ENV = "PULSO_PRODUCT_SIM_PG_DSN"
SOURCE_FILE = "product_stream"


class PostgresSink:
    """Writes the same rows into the `product` schema. The DSN is read from an env var and never printed."""

    def __init__(self, dsn_env: str = DEFAULT_DSN_ENV, connect=None, run_id: str = "run"):
        dsn = os.environ.get(dsn_env)
        if not dsn:
            raise RuntimeError(f"Postgres DSN env var {dsn_env} is not set")
        if connect is None:
            import psycopg  # optional dependency, only for the live path
            connect = psycopg.connect
        self.run_id = run_id
        self._n = 0
        self._con = connect(dsn)

    def __repr__(self) -> str:
        return f"PostgresSink(run_id={self.run_id!r})"

    def ensure_schema(self) -> None:
        with self._con.cursor() as cur:
            missing = []
            for t in PRODUCT_COLUMNS:  # provisioned targets (db/sql) are left alone: the writer role has no CREATE
                cur.execute("SELECT to_regclass(%s)", (f"product.{t}",))
                if not cur.fetchone()[0]:
                    missing.append(t)
            if not missing:
                self._con.commit()
                return
            cur.execute("CREATE SCHEMA IF NOT EXISTS product")
            for t, cols in PRODUCT_COLUMNS.items():
                if t not in missing:
                    continue
                defs = [f"{c} {PRODUCT_TYPES[t][c]}" for c in cols]
                defs += ["_batch_id text", "_source_file text", "_ingested_at timestamptz NOT NULL DEFAULT now()"]
                cur.execute(f"CREATE TABLE IF NOT EXISTS product.{t} ({', '.join(defs)})")
        self._con.commit()

    def event_log_count(self) -> int:
        with self._con.cursor() as cur:
            cur.execute("SELECT count(*) FROM product.event_log")
            return cur.fetchone()[0]

    def write_batch(self, rows_by_table: dict[str, list[dict]]) -> None:
        validate_batch(rows_by_table)
        self._n += 1
        batch_id = f"product-sim:{self.run_id}:{self._n}"
        with self._con.cursor() as cur:
            for t, rows in rows_by_table.items():
                if not rows:
                    continue
                if t == "customer_case_slots":
                    cur.executemany("DELETE FROM product.customer_case_slots WHERE customer_id = %s",
                                    [(r["customer_id"],) for r in rows])
                cols = [c for c in PRODUCT_COLUMNS[t] if all(c in r for r in rows)]
                ph = [("%s::jsonb" if PRODUCT_TYPES[t][c] == "jsonb" else "%s") for c in cols] + ["%s", "%s"]
                sql = (f"INSERT INTO product.{t} ({', '.join(cols)}, _batch_id, _source_file) "
                       f"VALUES ({', '.join(ph)})")
                cur.executemany(sql, [[_pg_val(r[c]) for c in cols] + [batch_id, SOURCE_FILE] for r in rows])
        self._con.commit()

    def reset(self) -> None:
        """Delete only rows written by this simulator (lineage _source_file)."""
        with self._con.cursor() as cur:
            for t in PRODUCT_COLUMNS:
                cur.execute(f"DELETE FROM product.{t} WHERE _source_file = %s", (SOURCE_FILE,))
        self._con.commit()

    def close(self) -> None:
        self._con.close()


def _pg_val(v):
    if isinstance(v, (dict, list)):
        return json.dumps(v, sort_keys=True, separators=(",", ":"))
    return v
