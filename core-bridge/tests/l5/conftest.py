"""L5 fixtures. Real Postgres 16 via PULSO_TEST_PG_ADMIN (skips without it, fails with PULSO_REQUIRE_POSTGRES=1).

Each test gets throw-away `runtime` (core + registry schema, plus the `pulso_bridge` schema on demand) and
`eval` (core schema only) databases."""

from __future__ import annotations

import os
import uuid
from collections.abc import Iterator
from dataclasses import dataclass
from typing import Any

import psycopg
import pytest


@dataclass(frozen=True)
class PgDbs:
    runtime: str
    eval: str
    admin: str


def _with_db(admin_dsn: str, name: str) -> str:
    return f"{admin_dsn.rpartition('/')[0]}/{name}"


@pytest.fixture
def pg() -> Iterator[PgDbs]:
    admin_dsn = os.environ.get("PULSO_TEST_PG_ADMIN")
    if not admin_dsn:
        if os.environ.get("PULSO_REQUIRE_POSTGRES") == "1":
            pytest.fail("PULSO_TEST_PG_ADMIN not set")
        pytest.skip("PULSO_TEST_PG_ADMIN not set (real Postgres 16 required)")
    tag = uuid.uuid4().hex[:10]
    names = [f"l5_{tag}_rt", f"l5_{tag}_ev"]
    try:
        admin: Any = psycopg.connect(admin_dsn, autocommit=True, connect_timeout=3)
    except psycopg.OperationalError:
        if os.environ.get("PULSO_REQUIRE_POSTGRES") == "1":
            raise
        pytest.skip("Postgres not reachable")
    with admin:
        for n in names:
            admin.execute(f'CREATE DATABASE "{n}"')
        from agent_core.adapters.postgres_uow import apply_schema
        from agent_core.registry import apply_registry_schema

        for n, reg in ((names[0], True), (names[1], False)):
            with psycopg.connect(_with_db(admin_dsn, n)) as conn:
                apply_schema(conn, None)
                if reg:
                    apply_registry_schema(conn, None)
        try:
            yield PgDbs(_with_db(admin_dsn, names[0]), _with_db(admin_dsn, names[1]), admin_dsn)
        finally:
            for n in names:
                admin.execute(f'DROP DATABASE IF EXISTS "{n}" WITH (FORCE)')
