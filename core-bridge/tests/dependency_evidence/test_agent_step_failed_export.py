"""Dependency evidence (agreement CX-0073/0075): the task receipt keeps its current outcomes; ONLY a correlated,
schema-valid audit event `agent_step` with `kind=failed` and `error_kind` classifies a model-dependency failure.
Real pinned Core + real PG16 + the REAL exporter against the ingest fixture double; the gateway is the contract
double (`tests/llm/gateway_double.py`) behind the real `HttpLLMGateway` and our metering/guard wrappers.

Also (re)generates the SANITIZED fixtures under tests/fixtures/dependency_evidence when
PULSO_REGEN_DEPENDENCY_EVIDENCE=1 and checks the checked-in ones still have the same shape."""

from __future__ import annotations

import json
import os
from pathlib import Path
from typing import Any

import httpx
import psycopg
import pytest
from fastapi.testclient import TestClient
from ingest_fixture.app import IngestState, create_app
from integration.conftest import SCOUT_OUTPUT, WORLD, Composed
from integration.test_llm_stack import _wire_http_gateway
from integration.test_scout import _post, _scout
from llm.gateway_double import GatewayDouble, failure, ok, raising

from pulso_core_runtime.exporter import (
    CoreReader,
    Exporter,
    ExporterConfig,
    ExporterState,
)
from pulso_core_runtime.store.receipts import ReceiptStore

pytestmark = [pytest.mark.integration, pytest.mark.pg]
FIXTURES = Path(__file__).resolve().parents[1] / "fixtures" / "dependency_evidence"
RO = "exporter_ro"
BAD_FINAL = {"kind": "final", "output": {"not": "the scout schema"}}
ZERO = "0" * 64


def _db(dsn: str) -> str:
    return dsn.rsplit("/", 1)[1]


def _exporter(c: Composed, tmp: Path) -> tuple[Exporter, IngestState]:
    admin: Any = psycopg.connect(c.pg.admin, autocommit=True)
    with admin:
        if not admin.execute("SELECT 1 FROM pg_roles WHERE rolname=%s", (RO,)).fetchone():
            admin.execute(f"CREATE ROLE {RO} LOGIN PASSWORD 'ro-test-only' NOSUPERUSER")
            admin.execute(f"ALTER ROLE {RO} SET default_transaction_read_only = on")
        admin.execute(f'REVOKE CONNECT ON DATABASE "{_db(c.pg.runtime)}" FROM PUBLIC')
        admin.execute(f'REVOKE CONNECT ON DATABASE "{_db(c.pg.eval)}" FROM PUBLIC')
        admin.execute(f'GRANT CONNECT ON DATABASE "{_db(c.pg.runtime)}" TO {RO}')
    with psycopg.connect(c.pg.runtime, autocommit=True) as conn:
        conn.execute(f"GRANT SELECT ON audit_events, reg_events, outbox TO {RO}")
    host = c.pg.runtime.split("@", 1)[1].rsplit("/", 1)[0]
    cfg = ExporterConfig(tenant_id="t1", instance="core-a", expected_runtime_db=_db(c.pg.runtime),
                         expected_eval_db=_db(c.pg.eval), binding_ref="binding-1")
    ingest = IngestState()
    client = TestClient(create_app(ingest), base_url="http://ingest.fixture")
    ro = f"postgresql://{RO}:ro-test-only@{host}/{_db(c.pg.runtime)}"
    return Exporter(cfg, CoreReader(ro, cfg), ExporterState(tmp / "state.sqlite"), client), ingest


def _chain(c: Composed, run_id: str) -> list[dict[str, Any]]:
    with psycopg.connect(c.pg.runtime) as conn:
        rows = conn.execute("SELECT seq, type, prev_hash, hash, event_json FROM audit_events WHERE run_id=%s "
                            "ORDER BY seq", (run_id,)).fetchall()
    return [{"seq": r[0], "type": r[1], "prev_hash": r[2], "hash": r[3], "event": json.loads(r[4])} for r in rows]


def _run(c: Composed, double: GatewayDouble, logical: str, tmp: Path) -> dict[str, Any]:
    _wire_http_gateway(c, double)
    key, payload = _scout(c, logical=logical)
    receipt = _post(c, key, payload).json()
    run_id = receipt["core_run_id"]
    assert run_id, receipt  # the receipt always exposes core_run_id so the platform can correlate
    chain = _chain(c, run_id)
    ex, ingest = _exporter(c, tmp)
    try:
        report = ex.poll_once()
    finally:
        ex.close()
    observations = [e for b in ingest.batches for e in b["events"] if e.get("source_run_ref") == run_id and e.get("source_sequence") is not None]
    return {"receipt": receipt, "chain": chain, "observations": observations, "report": report, "run_id": run_id}


def _failed_steps(chain: list[dict[str, Any]]) -> list[dict[str, Any]]:
    return [e for e in chain if e["type"] == "agent_step" and e["event"]["payload"]["kind"] == "failed"]


OUTAGES = [
    ("unavailable", raising(httpx.ConnectError("down"))),
    ("timeout", raising(httpx.ReadTimeout("slow"))),
    ("rate_limited", failure(429, "rate_limited")),
    ("invalid_output", failure(502, "invalid_output", 100, 50)),
    ("refused", failure(502, "refused", 100, 0)),
]


@pytest.mark.skipif(not WORLD.is_dir(), reason="agent-core-assets world absent")
@pytest.mark.parametrize(("error_kind", "script"), OUTAGES, ids=[o[0] for o in OUTAGES])
def test_gateway_failure_is_exported_as_a_correlated_agent_step_failed(
        composed: Composed, tmp_path: Path, error_kind: str, script: Any) -> None:
    got = _run(composed, GatewayDouble(script), f"dep-{error_kind}", tmp_path)
    failed = _failed_steps(got["chain"])
    assert len(failed) == 1, [e["type"] for e in got["chain"]]
    ev = failed[0]
    payload = ev["event"]["payload"]
    assert payload["error_kind"] == error_kind and payload["step"] == 1 and payload["node_id"]
    assert ev["event"]["run_id"] == got["run_id"] and ev["event"]["type"] == "agent_step"
    # the exporter carried exactly that event: run, seq, hash, schema, in chain order, gap semantics unchanged
    mine = {o["source_sequence"]: o for o in got["observations"]}
    assert sorted(mine) == [e["seq"] for e in got["chain"]] and got["report"].partial_reasons == []
    obs = mine[ev["seq"]]
    assert (obs["kind"], obs["level"], obs["source_run_ref"]) == ("core_event", "engine_event", got["run_id"])
    assert obs["native_event_id"] == ev["event"]["event_id"] and obs["source_event_digest"] == ev["hash"]
    assert obs["source_schema_ref"] and obs["source_event_ref"] and obs["coverage_marker"]
    for prev, cur in zip(got["chain"], got["chain"][1:], strict=False):
        assert cur["prev_hash"] == prev["hash"] and cur["seq"] == prev["seq"] + 1
    # the receipt keeps its CURRENT outcomes (no new outcome); our own spend evidence is the model_call_ledger
    assert got["receipt"]["state"] == "terminal_failed" and "dependency_unavailable" not in str(got["receipt"])
    rows = ReceiptStore(composed.pg.runtime).ledger_rows("t1", "j1", "scout", 1)
    assert rows and rows[0]["outcome"] == error_kind


@pytest.mark.skipif(not WORLD.is_dir(), reason="agent-core-assets world absent")
def test_a_legitimate_low_confidence_run_has_no_failed_step(composed: Composed, tmp_path: Path) -> None:
    got = _run(composed, GatewayDouble(ok(output=BAD_FINAL)), "dep-lowconf", tmp_path)
    assert _failed_steps(got["chain"]) == []
    assert got["receipt"]["state"] == "terminal_failed"
    assert {r["outcome"] for r in ReceiptStore(composed.pg.runtime).ledger_rows("t1", "j1", "scout", 1)} == {"ok"}


# --- sanitized fixtures --------------------------------------------------------------------------------------


def _sanitize(got: dict[str, Any], case: str) -> dict[str, Any]:
    """Structure only: hashes, ids and timestamps replaced by placeholders; no payload text, no attributes."""
    chain = got["chain"]
    failed = _failed_steps(chain)
    steps = [e for e in chain if e["type"] == "agent_step"]
    step = failed[0] if failed else (steps[0] if steps else None)
    by_seq = {o["source_sequence"]: o for o in got["observations"]}
    out: dict[str, Any] = {
        "case": case, "run_id": "run-A",
        "task_receipt": {k: got["receipt"].get(k) for k in ("state", "outcome", "reason")} | {"core_run_id": "run-A"},
        "chain": [{"seq": e["seq"], "type": e["type"], "hash": ZERO, "prev_hash": ZERO if e["seq"] else None}
                  for e in chain],
        "agent_step_failed_present": bool(failed)}
    if step is not None:
        p = step["event"]["payload"]
        keep = {k: v for k, v in p.items() if k in ("node_id", "step", "kind", "error_kind")}
        out["agent_step_event"] = {
            "run_id": "run-A", "seq": step["seq"], "event_id": f"run-A-e{step['seq']}", "type": "agent_step",
            "schema_version": step["event"].get("schema_version"), "hash": ZERO, "payload": keep | {"latency_ms": 0}}
        o = by_seq[step["seq"]]
        masked = {"source_event_digest": ZERO, "source_run_ref": "run-A", "observed_at": "<ts>",
                  "native_event_id": f"run-A-e{step['seq']}"}
        out["observation"] = {k: masked.get(k, "<ref>" if k.endswith("_ref") and isinstance(o[k], dict) else o[k])
                              for k in o}
    return out


def _shape(value: Any) -> Any:
    if isinstance(value, dict):
        return {k: _shape(v) for k, v in sorted(value.items())}
    if isinstance(value, list):
        return [_shape(v) for v in value]
    return type(value).__name__


@pytest.mark.skipif(not WORLD.is_dir(), reason="agent-core-assets world absent")
@pytest.mark.parametrize(("case", "script"), [
    ("outage_unavailable", raising(httpx.ConnectError("down"))),
    ("low_confidence_gave_up", ok(output=BAD_FINAL))])
def test_sanitized_fixtures_match_the_real_wire_shape(composed: Composed, tmp_path: Path, case: str,
                                                      script: Any) -> None:
    fresh = _sanitize(_run(composed, GatewayDouble(script), f"fx-{case}", tmp_path), case)
    path = FIXTURES / f"{case}.json"
    if os.environ.get("PULSO_REGEN_DEPENDENCY_EVIDENCE") == "1":
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps(fresh, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    stored = json.loads(path.read_text(encoding="utf-8"))
    assert _shape(stored) == _shape(fresh)
    assert SCOUT_OUTPUT["hypotheses"][0]["statement"] not in json.dumps(stored)  # no content
