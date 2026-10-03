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
    import gc

    from pulso_core_runtime.evaluation.arms import ArmRunner
    from pulso_core_runtime.evaluation.broker_clients import BrokerArtifactPort, BrokerSandboxClient
    from pulso_core_runtime.store.receipts import ReceiptStore

    runners = [o for o in gc.get_objects() if isinstance(o, ArmRunner)
               and getattr(o.artifacts, "_h", None) is not None and o.artifacts._h._base == composed.loop.url]
    assert runners, "composed ArmRunner not found"
    arms = runners[-1]
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
