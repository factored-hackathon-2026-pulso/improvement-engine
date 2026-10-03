"""CAP-57 / N-11: old binary vs new schema and the reverse, on a real PG16 (ADR 0022 s3 of the pin is policy only;
this is the executable half). The OLD pin is the previous reference checkout + venv (789d6c8); the NEW pin is the
current one (894fa65, PR #29: run_idempotency reservation + runs.change_xid). Skipped (and said so) when the old toolchain is not on this machine.

    PULSO_OLD_CORE_CHECKOUT / PULSO_OLD_CORE_PYTHON override the defaults below."""

from __future__ import annotations

import json
import os
import subprocess
import sys
import uuid
from collections.abc import Iterator
from pathlib import Path

import psycopg
import pytest
from runtime.conftest import PgDbs, pg  # noqa: F401  (fixture re-export: skip/fail semantics of PULSO_TEST_PG_ADMIN)

pytestmark = [pytest.mark.integration, pytest.mark.pg]

ROOT = Path(__file__).resolve().parents[2]
WORKER = ROOT / "scripts" / "expand_contract_worker.py"
OLD_CHECKOUT = Path(os.environ.get("PULSO_OLD_CORE_CHECKOUT", r"D:\.codex\factored\references\agent-core-789d6c8"))
OLD_PY = Path(os.environ.get("PULSO_OLD_CORE_PYTHON",
                             str(Path(os.environ.get("TEMP", ".")) / "pulso-wire-venv-789d6c8" / "Scripts" / "python.exe")))
OLD_SHA = "789d6c89b2fca90fc10e2abf157da51dc81c5d51"
NEW_CHECKOUT = Path(os.environ.get("PULSO_CORE_CHECKOUT", r"D:\.codex\factored\references\agent-core-894fa65"))
NEW_SHA = "894fa65575d83420523f33ec1c6919b8965f7ebe"
Toolchains = dict[str, tuple[Path, Path]]


def _head(path: Path) -> str:
    return subprocess.run(["git", "-C", str(path), "rev-parse", "HEAD"], capture_output=True, text=True).stdout.strip()


@pytest.fixture(scope="module")
def toolchains() -> Toolchains:
    if not OLD_PY.exists() or _head(OLD_CHECKOUT) != OLD_SHA:
        pytest.skip(f"old pin toolchain not available (need {OLD_PY} and a checkout at {OLD_SHA[:7]})")
    if _head(NEW_CHECKOUT) != NEW_SHA:
        pytest.skip(f"new pin checkout not available (set PULSO_CORE_CHECKOUT to a checkout at {NEW_SHA[:7]})")
    return {"old": (OLD_PY, OLD_CHECKOUT), "new": (Path(sys.executable), NEW_CHECKOUT)}


@pytest.fixture
def dsn(pg: PgDbs) -> Iterator[str]:  # noqa: F811
    name = f"ec_{uuid.uuid4().hex[:10]}"
    with psycopg.connect(pg.admin, autocommit=True) as admin:
        admin.execute(f'CREATE DATABASE "{name}"')
    yield pg.admin.rpartition("/")[0] + "/" + name
    with psycopg.connect(pg.admin, autocommit=True) as admin:
        admin.execute(f'DROP DATABASE IF EXISTS "{name}" WITH (FORCE)')


def _run(tc: Toolchains, which: str, *args: str) -> dict:
    py, checkout = tc[which]
    env = {**os.environ, "PULSO_CORE_CHECKOUT": str(checkout), "PYTHONPATH": str(checkout)}
    r = subprocess.run([str(py), "-W", "ignore", str(WORKER), *args], capture_output=True, text=True, env=env,
                       timeout=300)
    assert r.returncode == 0, f"{which} {args[0]} failed: {r.stderr[-800:]}"
    return json.loads(r.stdout.strip().splitlines()[-1])


def _migrate(tc: Toolchains, which: str, dsn: str, **extra_env: str) -> None:
    py, checkout = tc[which]
    exe = py.with_name("agentcore.exe") if py.with_name("agentcore.exe").exists() else py.with_name("agentcore")
    env = {k: v for k, v in os.environ.items() if k != "AGENTCORE_BLOB_BUCKET"}  # never inherit an S3 bucket
    r = subprocess.run([str(exe), "migrate", "--dsn", dsn], capture_output=True, text=True,
                       env={**env, "PYTHONPATH": str(checkout), **extra_env})
    assert r.returncode == 0, f"{which} migrate failed: {r.stdout[-300:]} {r.stderr[-300:]}"


def test_only_the_adapters_schema_changed_and_only_by_expansion(toolchains: Toolchains) -> None:
    """Static half. PR #29 touches ONE script: `change_xid` column + index, `reserved_until`, and a relaxed NOT NULL."""
    for rel in ("agent_core/adapters/sql/audit_events.sql", "agent_core/registry/postgres/schema.sql"):
        assert (toolchains["old"][1] / rel).read_bytes() == (toolchains["new"][1] / rel).read_bytes(), rel
    rel = "agent_core/adapters/sql/schema.sql"
    old, new = ((toolchains[w][1] / rel).read_text(encoding="utf-8") for w in ("old", "new"))
    assert old != new and "change_xid" in new and "reserved_until" in new
    assert "change_xid" not in old and "reserved_until" not in old
    assert "DROP TABLE" not in new and "DROP COLUMN" not in new


def test_new_schema_with_old_binary_and_back(toolchains: Toolchains, dsn: str) -> None:
    tc = toolchains
    _migrate(tc, "new", dsn)
    fp_new = _run(tc, "new", "fingerprint", dsn)["fingerprint"]
    # OLD binary against the schema the NEW binary created: migrate is a no-op, then it seeds and operates.
    _migrate(tc, "old", dsn)
    assert _run(tc, "old", "fingerprint", dsn)["fingerprint"] == fp_new
    assert _run(tc, "old", "engine", dsn)["ping"] is True
    assert _run(tc, "old", "seed", dsn)["releases"]
    old_view = _run(tc, "old", "exercise", dsn)
    # NEW binary reads what the OLD one wrote (a release imported by the old code) and writes on top.
    new_view = _run(tc, "new", "exercise", dsn)
    assert (new_view["release_id"], new_view["status"], new_view["entities"]) == \
           (old_view["release_id"], old_view["status"], old_view["entities"])
    assert _run(tc, "new", "engine", dsn)["ping"] is True
    # ... and the OLD one keeps working after the new one wrote.
    assert _run(tc, "old", "exercise", dsn)["proposal_state"] == "draft"
    _migrate(tc, "new", dsn)
    assert _run(tc, "new", "fingerprint", dsn)["fingerprint"] == fp_new


def test_old_schema_with_new_binary(toolchains: Toolchains, dsn: str) -> None:
    tc = toolchains
    _migrate(tc, "old", dsn)
    fp_old = _run(tc, "old", "fingerprint", dsn)["fingerprint"]
    _migrate(tc, "new", dsn)
    assert _run(tc, "new", "fingerprint", dsn)["fingerprint"] != fp_old  # 894fa65 expands the schema (change_xid, ...)
    _run(tc, "new", "seed", dsn)
    assert _run(tc, "new", "exercise", dsn)["proposal_state"] == "draft"
    # ROLLBACK HAZARD (data, not schema): 894fa65 seeds `locked: true` on the demo interrupt (`Interrupt.locked`,
    # extra=forbid in 789d6c8), so the old binary can no longer read that release. Rolling back is only safe while no
    # release written by the new pin carries a locked interrupt (our four pulso-evolution releases have no interrupts).
    py, checkout = tc["old"]
    env = {**os.environ, "PULSO_CORE_CHECKOUT": str(checkout), "PYTHONPATH": str(checkout)}
    r = subprocess.run([str(py), "-W", "ignore", str(WORKER), "exercise", dsn], capture_output=True, text=True,
                       env=env, timeout=300)
    assert r.returncode != 0 and "interrupts.0.locked" in r.stderr


def test_blob_bucket_migrate_is_the_one_non_expand_step_and_the_old_binary_survives_it(toolchains: Toolchains,
                                                                                       dsn: str) -> None:
    """The ONLY schema difference between the pins is python-embedded, not in a `.sql` script: `agentcore migrate` with
    `AGENTCORE_BLOB_BUCKET` set runs `ALTER TABLE ... DROP CONSTRAINT` (ADR 0022 s3 vetoes DROP CONSTRAINT; upstream
    findings PR27-04). We never set the bucket, but this pins what happens if infra does: the fingerprint changes (so the
    ADR claim "no schema change to undo" holds only without the bucket) and the old binary still operates on it. Blobs
    written to S3 afterwards would not be readable by the old binary (rollback hazard, not testable without S3)."""
    tc = toolchains
    _migrate(tc, "new", dsn)
    plain = _run(tc, "new", "fingerprint", dsn)["fingerprint"]
    _migrate(tc, "new", dsn, AGENTCORE_BLOB_BUCKET="unused-bucket-migrate-never-calls-s3")
    detached = _run(tc, "new", "fingerprint", dsn)["fingerprint"]
    assert detached != plain, "the bucket-enabled migrate no longer changes the schema: update ADR 0008"
    _migrate(tc, "old", dsn)  # the old binary's migrate does not re-add the FK on an existing table
    assert _run(tc, "old", "fingerprint", dsn)["fingerprint"] == detached
    assert _run(tc, "old", "seed", dsn)["releases"]
    assert _run(tc, "old", "exercise", dsn)["proposal_state"] == "draft"
