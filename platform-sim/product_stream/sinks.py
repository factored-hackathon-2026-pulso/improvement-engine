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

    def close(self) -> None:
        self.con.close()
