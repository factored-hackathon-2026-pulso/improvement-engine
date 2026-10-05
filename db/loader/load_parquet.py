"""Parquet -> Postgres COPY loader for schemas raw / augmented / product (generic over db/catalog.py).

Rules: refuses evaluator tables and unknown columns, owns the lineage columns, idempotent per _batch_id (delete the
batch then COPY, one transaction), never prints or raises row values. DSN comes from PULSO_PG_LOADER_DSN only.

    python db/loader/load_parquet.py --schema raw --table e0_case --file case.parquet [--batch-id B]
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Callable, Iterable, Iterator

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import catalog  # noqa: E402

BATCH_ROWS = 5000


class LoaderError(Exception):
    pass


class LoaderRefusal(LoaderError):
    """Policy refusal; messages carry table/column names only."""


@dataclass(frozen=True)
class Plan:
    schema: str
    table: str
    columns: tuple[str, ...]
    delete_sql: str
    copy_sql: str


@dataclass(frozen=True)
class Result:
    batch_id: str
    rows: int


def build_plan(schema: str, table: str, file_columns: Iterable[str], batch_id: str, source_file: str) -> Plan:
    if schema not in catalog.SCHEMAS:
        raise LoaderRefusal(f"schema not loadable: {schema!r}")
    bare = table.split("_", 1)[1] if table.startswith(("e0_", "bank_")) else table
    if table in catalog.FORBIDDEN_TABLES or bare in catalog.FORBIDDEN_TABLES:
        raise LoaderRefusal(f"table is never loaded: {table!r}")
    if table not in catalog.SCHEMAS[schema]:
        raise LoaderRefusal(f"unknown table {schema}.{table}")
    known = {n for n, _ in catalog.SCHEMAS[schema][table]}
    cols = list(file_columns)
    lineage = [x for x in cols if x in catalog.LINEAGE_NAMES]
    if lineage:
        raise LoaderRefusal(f"lineage columns are set by the loader: {lineage}")
    unknown = [x for x in cols if x not in known]
    if unknown:
        raise LoaderRefusal(f"unknown columns for {schema}.{table}: {unknown}")
    all_cols = [*cols, "_batch_id", "_source_file"]
    return Plan(schema, table, tuple(cols),
                f"DELETE FROM {schema}.{table} WHERE _batch_id = %s",
                f"COPY {schema}.{table} ({', '.join(all_cols)}) FROM STDIN")


def default_batch_id(path: str) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()[:16]


def _adapt(value: Any, pgtype: str) -> Any:
    if value is None:
        return None
    if pgtype == "jsonb" and not isinstance(value, str):
        return json.dumps(value, default=str)
    return value


def _parquet_reader(path: str, columns: list[str] | None) -> Iterator[list[tuple]]:
    import pyarrow.parquet as pq  # lazy: unit tests need no pyarrow

    pf = pq.ParquetFile(path)
    for batch in pf.iter_batches(batch_size=BATCH_ROWS, columns=columns):
        cols = [batch.column(i).to_pylist() for i in range(batch.num_columns)]
        yield list(zip(*cols))


def _parquet_columns(path: str) -> list[str]:
    import pyarrow.parquet as pq

    return list(pq.ParquetFile(path).schema_arrow.names)


def load_file(conn: Any, schema: str, table: str, path: str, *, batch_id: str | None = None,
              file_columns: list[str] | None = None,
              reader: Callable[[str, list[str] | None], Iterable[list[tuple]]] = _parquet_reader) -> Result:
    cols = file_columns if file_columns is not None else _parquet_columns(path)
    bid = batch_id or default_batch_id(path)
    source = os.path.basename(path)
    plan = build_plan(schema, table, cols, bid, source)
    types = dict(catalog.SCHEMAS[schema][table])
    pgtypes = [types[c] for c in plan.columns]
    total = 0
    try:
        with conn.transaction():
            with conn.cursor() as cur:
                cur.execute(plan.delete_sql, (bid,))
                with cur.copy(plan.copy_sql) as cp:
                    for rows in reader(path, list(plan.columns)):
                        for row in rows:
                            cp.write_row([*(_adapt(v, t) for v, t in zip(row, pgtypes)), bid, source])
                            total += 1
    except LoaderError:
        raise
    except Exception as exc:  # never forward the driver/reader message: it may quote a row value
        raise LoaderError(f"load failed for {schema}.{table} ({type(exc).__name__})") from None
    return Result(bid, total)


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--schema", required=True)
    ap.add_argument("--table", required=True)
    ap.add_argument("--file", required=True)
    ap.add_argument("--batch-id")
    a = ap.parse_args(argv)
    dsn = os.environ.get("PULSO_PG_LOADER_DSN")
    if not dsn:
        print("PULSO_PG_LOADER_DSN is not set", file=sys.stderr)
        return 2
    import psycopg

    try:
        with psycopg.connect(dsn) as conn:
            res = load_file(conn, a.schema, a.table, a.file, batch_id=a.batch_id)
    except LoaderError as e:
        print(f"refused/failed: {e}", file=sys.stderr)
        return 1
    print(f"loaded {res.rows} rows into {a.schema}.{a.table} batch {res.batch_id}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
