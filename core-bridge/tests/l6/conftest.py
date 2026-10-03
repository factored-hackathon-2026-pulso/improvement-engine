"""L6 fixtures: throwaway PG16 (PULSO_TEST_PG_ADMIN), least-privilege `exporter_ro` role, ingest fixture double.

Doubles: ingest fixture (platform-sim/ingest_fixture, in-process ASGI via TestClient), FakeClock. Core PG is real."""

from __future__ import annotations

import os
import uuid
from collections.abc import Callable, Iterator
from dataclasses import dataclass
from datetime import UTC, datetime, timedelta
from pathlib import Path
from typing import Any

import psycopg
import pytest
from fastapi.testclient import TestClient
from ingest_fixture.app import IngestState, create_app

from pulso_core_runtime.exporter import CoreReader, Exporter, ExporterConfig, ExporterState

RO_ROLE = "exporter_ro"
RO_PASSWORD = "ro-test-only"


def _with_db(admin_dsn: str, name: str) -> str:
    base, _, _ = admin_dsn.rpartition("/")
    return f"{base}/{name}"


@dataclass(frozen=True)
class L6Pg:
    runtime: str       # superuser DSN (seeding / tampering only)
    runtime_ro: str    # exporter_ro DSN (what the exporter uses)
    runtime_db: str
    eval_db: str
    eval: str
    admin: str


class FakeClock:
    def __init__(self) -> None:
        self.now = datetime(2026, 10, 3, 12, 0, 0, tzinfo=UTC)

    def __call__(self) -> datetime:
        return self.now

    def advance(self, seconds: float) -> None:
        self.now += timedelta(seconds=seconds)


@pytest.fixture
def pg() -> Iterator[L6Pg]:
    admin_dsn = os.environ.get("PULSO_TEST_PG_ADMIN")
    if not admin_dsn:
        if os.environ.get("PULSO_REQUIRE_POSTGRES") == "1":
            pytest.fail("PULSO_TEST_PG_ADMIN not set")
        pytest.skip("PULSO_TEST_PG_ADMIN not set (real Postgres 16 required)")
    tag = uuid.uuid4().hex[:10]
    rt, ev = f"l6_{tag}_rt", f"l6_{tag}_ev"
    admin: Any = psycopg.connect(admin_dsn, autocommit=True, connect_timeout=3)
    with admin:
        exists = admin.execute("SELECT 1 FROM pg_roles WHERE rolname=%s", (RO_ROLE,)).fetchone()
        if not exists:
            admin.execute(f"CREATE ROLE {RO_ROLE} LOGIN PASSWORD '{RO_PASSWORD}' NOSUPERUSER NOCREATEDB "
                          "NOCREATEROLE NOBYPASSRLS")
            admin.execute(f"ALTER ROLE {RO_ROLE} SET default_transaction_read_only = on")
        for n in (rt, ev):
            admin.execute(f'CREATE DATABASE "{n}"')
            admin.execute(f'REVOKE CONNECT ON DATABASE "{n}" FROM PUBLIC')
        admin.execute(f'GRANT CONNECT ON DATABASE "{rt}" TO {RO_ROLE}')
        from agent_core.adapters.postgres_uow import apply_schema
        from agent_core.registry import apply_registry_schema

        for n, reg in ((rt, True), (ev, False)):
            with psycopg.connect(_with_db(admin_dsn, n)) as conn:
                apply_schema(conn, None)
                if reg:
                    apply_registry_schema(conn, None)
        with psycopg.connect(_with_db(admin_dsn, rt)) as conn:
            conn.execute(f"GRANT SELECT ON audit_events, reg_events, outbox TO {RO_ROLE}")
        host = _with_db(admin_dsn, rt).split("@", 1)[1].rsplit("/", 1)[0]
        try:
            yield L6Pg(runtime=_with_db(admin_dsn, rt), eval=_with_db(admin_dsn, ev),
                       runtime_ro=f"postgresql://{RO_ROLE}:{RO_PASSWORD}@{host}/{rt}", runtime_db=rt, eval_db=ev,
                       admin=admin_dsn)
        finally:
            for n in (rt, ev):
                admin.execute(f'DROP DATABASE IF EXISTS "{n}" WITH (FORCE)')


@dataclass
class Rig:
    pg: L6Pg
    ingest: IngestState
    client: TestClient
    clock: FakeClock
    tmp: Path
    make: Callable[..., Exporter]


@pytest.fixture
def rig(pg: L6Pg, tmp_path: Path) -> Iterator[Rig]:
    ingest = IngestState()
    client = TestClient(create_app(ingest), base_url="http://ingest.fixture")
    clock = FakeClock()
    made: list[Exporter] = []

    def make(state_path: Path | None = None, **cfg_over: Any) -> Exporter:
        cfg = ExporterConfig(tenant_id="tenant-1", instance="core-a", expected_runtime_db=pg.runtime_db,
                             expected_eval_db=pg.eval_db, binding_ref="binding-1", **cfg_over)
        ex = Exporter(cfg, CoreReader(pg.runtime_ro, cfg), ExporterState(state_path or tmp_path / "state.sqlite"),
                      client, clock=clock, sleep=lambda s: clock.advance(s))
        made.append(ex)
        return ex

    try:
        yield Rig(pg, ingest, client, clock, tmp_path, make)
    finally:
        for ex in made:
            ex.close()
        client.close()
