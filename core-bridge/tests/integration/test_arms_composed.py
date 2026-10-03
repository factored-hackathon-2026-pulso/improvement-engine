"""Task arms through the composed app: the REAL BrokerArtifactPort + BrokerSandboxClient over HTTP to the loopback
broker/bank (double), arms route -> ArmRunner -> real engine, atomic ledger, PG16.

Gap closed: `task_builder` / `stateful_attention` used to be refused (`sandbox_required`) and every manifest fetch
failed (`manifest_missing`) because the composition wired `NoArtifactPort` and `sandbox=None`."""

from __future__ import annotations

import hashlib
from typing import Any

import pytest
from agent_core.domain.json import canonical_bytes

from integration.conftest import Composed, composed  # noqa: F401  (fixture re-export)
from integration.test_arm_broker import claims_of
from l5.world import AGENT, suite_with

pytestmark = [pytest.mark.integration, pytest.mark.pg]


def seal(composed_: Composed, ref: str = "art-1") -> None:
    scenarios = [suite_with().scenarios[0].model_dump(mode="json")]
    content = {"scenarios": scenarios, "entries": {s["id"]: {} for s in scenarios}}
    composed_.loop.backend.artifacts[ref] = {
        "schema_version": "1", "encoding": "json", "content": content, "byte_length": 100,
        "artifact": {"id": ref, "media_type": "application/json",
                     "digest": "sha256:" + hashlib.sha256(canonical_bytes(content)).hexdigest()}}


def arm_body(key: str, mode: str, **over: Any) -> dict[str, Any]:
    return {"idempotency_key": key, "binding_ref": "bind-1", "campaign_ref": "camp-1", "case_ref": "case-1",
            "arm": "baseline", "repetition": 0, "seed": 7, "mode": mode, "agent_id": AGENT,
            "target": {"kind": "published_release", "release_id": "rel-demo"}, "scenario_manifest_ref": "art-1",
            "budget_ref": "bud-1", "seed_manifest_ref": "seed-1", **over}


def run_arm(c: Composed, body: dict[str, Any]) -> Any:
    return c.client.post("/internal/v1/evaluation/arms/run", json=body, headers={
        **c.headers("evaluation_arm_run"), "Idempotency-Key": body["idempotency_key"]})


@pytest.mark.parametrize("mode", ["task_builder", "stateful_attention"])
def test_task_arms_run_against_the_loopback_bank_with_a_real_manifest(composed: Composed, mode: str) -> None:
    seal(composed)
    r = run_arm(composed, arm_body("k-" + mode, mode))
    assert r.status_code == 200, r.text
    rep = r.json()
    assert rep["status"] == "completed", rep
    assert rep["initial_state_digest"].startswith("sha256:") and rep["final_state_ref"].startswith("final:bank-")
    backend = composed.loop.backend
    assert backend.sessions and all(s["closed"] == "arm_done" for s in backend.sessions.values())
    opened = [b for r_, b in zip(backend.requests, backend.bodies, strict=True)
              if r_.url.path.endswith("/sandbox/sessions")]
    assert opened == [{"campaign_ref": "camp-1", "case_ref": "case-1", "arm": "baseline", "repetition": 0,
                       "seed_manifest_ref": "seed-1"}]
    artifact_calls = [r_ for r_ in backend.requests if "/artifacts/" in r_.url.path]
    assert claims_of(artifact_calls[0])["scope"] == "artifact_read"
    assert claims_of(artifact_calls[0])["binding_ref"] == "bind-1"  # the port now carries the binding
    sandbox_calls = [claims_of(r_) for r_ in backend.requests if "/sandbox/" in r_.url.path]
    assert sandbox_calls and {c_["scope"] for c_ in sandbox_calls} == {"sandbox"}
    assert {c_["aud"] for c_ in sandbox_calls} == {"lab-broker"} and {c_["tenant_id"] for c_ in sandbox_calls} == {"t1"}
    assert len({c_["jti"] for c_ in sandbox_calls}) == len(sandbox_calls)


def test_composition_wires_real_clients_and_the_shared_ledger(composed: Composed) -> None:
    from pulso_core_runtime.evaluation.broker_clients import BrokerArtifactPort, BrokerSandboxClient
    from pulso_core_runtime.store.receipts import ReceiptStore

    arms = composed.app.state.pulso_arms  # deterministic handle; no gc.get_objects scan
    assert arms.artifacts._h._base == composed.loop.url
    assert isinstance(arms.artifacts, BrokerArtifactPort) and isinstance(arms.sandbox, BrokerSandboxClient)
    assert isinstance(arms.ledger, ReceiptStore)  # atomic capped spend, not the in-process tally


def test_bank_down_is_failed_infra_and_missing_manifest_is_manifest_missing(composed: Composed) -> None:
    seal(composed)
    composed.loop.backend.open_mode = "503"
    assert run_arm(composed, arm_body("k-down", "task_builder")).json()["status"] == "failed_infra"
    composed.loop.backend.open_mode = "ok"
    rep = run_arm(composed, arm_body("k-missing", "task_builder", scenario_manifest_ref="nope")).json()
    assert rep["status"] == "failed_infra" and rep["reason"] == "manifest_missing"
    native = run_arm(composed, arm_body("k-native", "native", seed_manifest_ref=None))  # native: no bank, same manifest
    del native


def test_composed_arm_ledger_charges_atomically_and_enforces_the_cap(composed: Composed) -> None:
    """The ledger the composition hands to the ArmRunner is the real PG `ReceiptStore`: an `EvalBudgetMeter` over
    it, wrapping the composed test gateway, charges `budget_meter` and refuses spend over the cap. (The demo arm
    scenarios of the loopback bank make zero model calls, so the arm route itself cannot move the meter here; the
    live scout path charging the same store is covered in test_scout.)"""
    from decimal import Decimal

    from agent_core.domain import EntityRef, Locale

    from pulso_core_runtime.evaluation.budget import BudgetLimits, EvalBudgetMeter
    from pulso_core_runtime.store.receipts import ReceiptStore

    arms = composed.app.state.pulso_arms
    assert isinstance(arms.ledger, ReceiptStore)
    meter = EvalBudgetMeter(BudgetLimits("bud-x", Decimal("0.0015"), None, None, None), ledger=arms.ledger,
                            tenant_id="t1", job_id="job-ledger-cap")
    gateway = meter.wrap(composed.gateway)
    prompt, loc = EntityRef(id="p", version="1.0.0"), Locale("es")
    inputs = {"goal": "g"}
    gateway.generate(prompt, inputs, loc)  # 0.001 within the 0.0015 cap
    row = ReceiptStore(composed.pg.runtime).meter_get("t1", "job-ledger-cap", "evaluation", 0)
    assert row is not None and row["calls"] == 1 and Decimal(str(row["cost_usd"])) == Decimal("0.001")
    gateway.generate(prompt, inputs, loc)  # would reach 0.002: the ledger refuses and the meter marks exhaustion
    assert meter.exhausted == "cost_usd_max"
    row = ReceiptStore(composed.pg.runtime).meter_get("t1", "job-ledger-cap", "evaluation", 0)
    assert row is not None and row["calls"] == 1  # the refused spend was never applied
