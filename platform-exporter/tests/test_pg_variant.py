"""Postgres variant against a REAL Postgres 16 (throwaway Podman container, PULSO_TEST_PG_ADMIN).

The platform schema is the simulator DDL rendered for Postgres, loaded with a simulator scenario. The exporter connects
with a least-privilege role (column-level SELECT on the allow-list only), so the database itself also refuses the
credential tables: defence in depth behind the exporter's own pre-query assertion."""

import os
import uuid
from datetime import UTC, datetime, timedelta

import psycopg
import pytest
from fastapi.testclient import TestClient
from ingest_fixture.app import IngestState, create_app
from platform_live import PlatformLiveSim
from platform_live.ddl import render_ddl

from platform_exporter import Exporter, ExporterConfig, ExporterState, PostgresSource
from platform_exporter.policy import ALLOWED_COLUMNS, AccessDenied
from tests.conftest import FakeClock

pytestmark = pytest.mark.pg
ADMIN = os.environ.get("PULSO_TEST_PG_ADMIN")
if not ADMIN:
    pytestmark = [pytest.mark.pg, pytest.mark.skip(reason="PULSO_TEST_PG_ADMIN not set")]

TABLES = ["customers", "staff", "teams", "admin_roster", "cases", "turns", "assignments", "customer_case_slots", "login_accounts",
          "mfa_challenges", "staff_sessions", "analyst_availability", "event_log"]
RO_PASSWORD = "ro-test-only"


def _copy(sim_conn, pg) -> None:
    if sim_conn.execute("SELECT 1 FROM sqlite_master WHERE name='teams'").fetchone():  # planned Product evolution
        pg.execute("CREATE TABLE teams (id TEXT PRIMARY KEY, name TEXT NOT NULL UNIQUE, active BOOLEAN NOT NULL DEFAULT TRUE)")
        pg.execute("CREATE TABLE admin_roster (staff_id TEXT PRIMARY KEY REFERENCES staff(id), since TEXT NOT NULL)")
        pg.execute("ALTER TABLE staff ADD COLUMN team_id TEXT")
        pg.execute("ALTER TABLE staff DROP COLUMN team")
    for table in TABLES:
        if table in ("teams", "admin_roster") and not sim_conn.execute(
                "SELECT 1 FROM sqlite_master WHERE name=?", (table,)).fetchone():
            continue
        cols = [r[0] for r in sim_conn.execute(f"SELECT name FROM pragma_table_info('{table}')")]
        types = dict(pg.execute("SELECT column_name, data_type FROM information_schema.columns WHERE table_name=%s",
                                (table,)).fetchall())
        rows = sim_conn.execute(f"SELECT {', '.join(cols)} FROM {table}").fetchall()
        ph = ", ".join("%s" for _ in cols)
        for row in rows:
            vals = [bool(v) if types.get(c) == "boolean" and v is not None else v for c, v in zip(cols, row, strict=True)]
            pg.execute(f"INSERT INTO {table} ({', '.join(cols)}) VALUES ({ph})", vals)


@pytest.fixture
def pg_rig(tmp_path):
    name = f"plexp_{uuid.uuid4().hex[:10]}"
    admin = psycopg.connect(ADMIN, autocommit=True)
    admin.execute(f"CREATE DATABASE {name}")
    base = ADMIN.rpartition("/")[0]
    owner_dsn = f"{base}/{name}"
    owner = psycopg.connect(owner_dsn, autocommit=True)
    for stmt in render_ddl("postgres"):
        owner.execute(stmt)
    sim = PlatformLiveSim(seed=7, path=str(tmp_path / "sim.db"))
    sim.generate(n_cases=40, faults=True)
    sim.conn.commit()
    _copy(sim.conn, owner)
    role = f"exporter_ro_{uuid.uuid4().hex[:8]}"
    admin.execute(f"CREATE ROLE {role} LOGIN PASSWORD '{RO_PASSWORD}'")
    owner.execute(f"REVOKE ALL ON ALL TABLES IN SCHEMA public FROM {role}")
    owner.execute(f"GRANT USAGE ON SCHEMA public TO {role}")
    for table, cols in ALLOWED_COLUMNS.items():
        have = {r[0] for r in owner.execute("SELECT column_name FROM information_schema.columns WHERE "
                                            "table_name=%s", (table,)).fetchall()}
        owner.execute(f"GRANT SELECT ({', '.join(c for c in cols if c in have)}) ON {table} TO {role}")
    host = base.split("@")[1]
    ro_dsn = f"postgresql://{role}:{RO_PASSWORD}@{host}/{name}"
    ingest = IngestState()
    client = TestClient(create_app(ingest), base_url="http://ingest.fixture")
    clock = FakeClock()
    cfg = ExporterConfig(tenant_id="tenant-1", instance="plat-pg", binding_ref="binding-1", window_seconds=600,
                         gap_grace_seconds=0.0)
    ex = Exporter(cfg, PostgresSource(ro_dsn), ExporterState(tmp_path / "state.sqlite"), client, clock=clock,
                  sleep=lambda s: clock.advance(s))
    try:
        yield sim, ex, ingest, ro_dsn, owner
    finally:
        ex.close()
        sim.conn.close()
        owner.close()
        admin.execute(f"DROP DATABASE {name} WITH (FORCE)")
        admin.execute(f"DROP ROLE {role}")
        admin.close()


def test_pg_role_cannot_read_credential_tables_or_state_columns(pg_rig):
    _sim, _ex, _ing, ro_dsn, _owner = pg_rig
    with psycopg.connect(ro_dsn, autocommit=True) as c:
        for sql in ("SELECT password_hash FROM login_accounts", "SELECT status FROM cases", "SELECT name FROM staff",
                    "SELECT text FROM turns"):
            with pytest.raises(psycopg.errors.InsufficientPrivilege):
                c.execute(sql)
        with pytest.raises(psycopg.errors.ReadOnlySqlTransaction):
            c.execute("SET default_transaction_read_only = on")
            c.execute("INSERT INTO event_log(sequence) VALUES (999999)")  # privilege or read-only: never writes


def test_pg_exporter_pre_query_assertion_is_independent_of_database_grants(pg_rig):
    _sim, ex, _ing, _dsn, _owner = pg_rig
    with pytest.raises(AccessDenied):
        ex.source.table_columns("login_accounts")


def test_pg_poll_matches_simulator_faults_and_paths(pg_rig):
    sim, ex, ingest, _dsn, _owner = pg_rig
    rep = ex.poll_once()
    assert not rep.stopped and not rep.errors, (rep.stopped, rep.errors)
    gap = next(f for f in sim.faults if f["kind"] == "sequence_gap")
    assert (gap["after"] + 1, gap["after"] + gap["size"]) in rep.gaps
    assert rep.unknown_event_types["case.escalated"] == 1
    assert rep.late_events
    total = sim.conn.execute("SELECT COUNT(*) FROM event_log").fetchone()[0]
    known = {e["native_event_id"] for b in ingest.batches for e in b["events"] if e["source_sequence"] is not None
             and e["source_event"].get("kind") != "exporter_finding"}
    assert len(known) + sum(rep.unknown_event_types.values()) == total
    ex.rescan()  # staff/dimension reads with column-level grants only


def test_pg_extract_uses_timestamptz_and_jsonb_and_never_reads_state(pg_rig):
    sim, ex, _ing, _dsn, _owner = pg_rig
    case_id, closed_at = sim.conn.execute("SELECT case_id, event_time FROM event_log WHERE "
                                          "event_type='case.closed' ORDER BY sequence LIMIT 1").fetchone()
    cut = datetime.fromisoformat(closed_at.replace("Z", "+00:00")).astimezone(UTC)
    assert ex.case_extract(cut - timedelta(seconds=1))[case_id].close_reason is None
    assert ex.case_extract(cut + timedelta(seconds=5))[case_id].status == "closed"
