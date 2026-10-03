"""Real Postgres 16 fixtures (Podman `pulso-dev`). Set PULSO_TEST_PG_ADMIN, e.g.
postgresql://postgres:<pw>@127.0.0.1:55416/postgres; without it PG tests skip (or fail with
PULSO_REQUIRE_POSTGRES=1). Each test gets throw-away databases, dropped afterwards."""

from __future__ import annotations

import os
import uuid
from collections.abc import Callable, Iterator
from dataclasses import dataclass
from typing import Any

import psycopg
import pytest


@dataclass(frozen=True)
class PgDbs:
    runtime: str
    eval: str
    other: str  # a third database, handy to simulate "wrong" DSNs
    admin: str


def _with_db(admin_dsn: str, name: str) -> str:
    base, _, _ = admin_dsn.rpartition("/")
    return f"{base}/{name}"


@pytest.fixture
def pg() -> Iterator[PgDbs]:
    admin_dsn = os.environ.get("PULSO_TEST_PG_ADMIN")
    if not admin_dsn:
        if os.environ.get("PULSO_REQUIRE_POSTGRES") == "1":
            pytest.fail("PULSO_TEST_PG_ADMIN not set")
        pytest.skip("PULSO_TEST_PG_ADMIN not set (real Postgres 16 required)")
    tag = uuid.uuid4().hex[:10]
    names = [f"t_{tag}_{k}" for k in ("rt", "ev", "ot")]
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
            yield PgDbs(*(_with_db(admin_dsn, n) for n in names), admin=admin_dsn)
        finally:
            for n in names:
                admin.execute(f'DROP DATABASE IF EXISTS "{n}" WITH (FORCE)')


Dsn = Callable[[str], str]
