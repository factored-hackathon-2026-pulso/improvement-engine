"""CAP-53 contract tests for the bridge mock (`runtime_profile=contract_mock`).

First RED: `test_version_is_labelled_contract_mock` fails until `bridge_mock` exists; mutating the label breaks it again."""

from __future__ import annotations

import concurrent.futures as cf
import time

import httpx
import pytest

from bridge_mock import service_jws
from registry_mock import jws as core_jws

from .conftest import validate

NO_EFFECTS = {"lab_queries": 0, "model_calls": 0, "registry_writes": 0}


def same(a: dict, b: dict) -> bool:
    """Replays return the same stored result; only the per-request trace id differs."""
    return {**a, "trace_id": None} == {**b, "trace_id": None}


def err(resp: httpx.Response, status: int, code: str) -> dict:
    assert resp.status_code == status, resp.text
    body = resp.json()
    validate("BridgeError", body)
    assert body["code"] == code, body
    return body


# --- identity / label -----------------------------------------------------------------------------------------

def test_version_is_labelled_contract_mock(bridge) -> None:
    r = bridge.c.get("/internal/v1/version", headers=bridge.headers("version_read"))
    assert r.status_code == 200
    validate("CoreVersion", r.json())
    assert r.json()["runtime_profile"] == "contract_mock"
    assert r.json()["agent_core_sha"] == "789d6c89b2fca90fc10e2abf157da51dc81c5d51"
    assert r.json()["contracts_version"] == "1.3.0"


def test_sim_info_is_labelled(bridge) -> None:
    info = bridge.c.get("/_sim/info").json()
    assert info["runtime_profile"] == "contract_mock" and info["level"] == "mock"


def test_alias_response_carries_the_label(bridge) -> None:
    r = bridge.c.get("/internal/v1/core-state/aliases/pulso-writer/staging", headers=bridge.headers("state_read"))
    assert r.json()["runtime_profile"] == "contract_mock"


# --- service JWT auth -------------------------------------------------------------------------------------------

def _token_for(bridge, case: str) -> str | None:
    return {
        "missing": None,
        "garbage": "not-a-jws",
        "bad_signature": bridge.token("version_read").rsplit(".", 1)[0] + "." + service_jws.b64url_encode(bytes(64)),
        "alg_none": service_jws.sign({"alg": "none", "kid": service_jws.CONTROL_KID, "typ": "JWT"},
                                     service_jws.claims(purpose="version_read")),
        "unknown_kid": bridge.token("version_read", kid="sim-unknown-9"),
        "wrong_typ": bridge.token("version_read", typ="principal+jws"),
        "wrong_audience": bridge.token("version_read", aud="control-api"),
        "wrong_issuer": bridge.token("version_read", iss="somebody"),
        "expired": bridge.token("version_read", ttl=-10),
        "ttl_over_5_min": bridge.token("version_read", ttl=301),
        "wrong_purpose": bridge.token("alias_read"),
    }[case]


@pytest.mark.parametrize("case,status,code", [
    ("missing", 401, "credentials_invalid"), ("garbage", 401, "credentials_invalid"),
    ("bad_signature", 401, "credentials_invalid"), ("alg_none", 401, "credentials_invalid"),
    ("unknown_kid", 401, "credentials_invalid"), ("wrong_typ", 401, "credentials_invalid"),
    ("wrong_audience", 401, "credentials_invalid"), ("wrong_issuer", 401, "credentials_invalid"),
    ("expired", 401, "credentials_invalid"), ("ttl_over_5_min", 401, "credentials_invalid"),
    ("wrong_purpose", 403, "forbidden"),
])
def test_service_jwt_rules(bridge, case, status, code) -> None:
    token = _token_for(bridge, case)
    r = bridge.c.get("/internal/v1/version", headers={"Authorization": f"Bearer {token}"} if token else {})
    err(r, status, code)
    assert r.headers["content-type"].startswith("application/json")


def test_jti_replay_is_rejected(bridge) -> None:
    h = {"Authorization": f"Bearer {bridge.token('version_read')}"}
    assert bridge.c.get("/internal/v1/version", headers=h).status_code == 200
    err(bridge.c.get("/internal/v1/version", headers=h), 401, "credentials_invalid")


def test_token_tenant_must_equal_the_body_tenant(bridge) -> None:
    body = bridge.invocation(tenant="tenant-b")
    r = bridge.c.post("/internal/v1/core-tasks/invoke", json=body, headers=bridge.headers("task_invoke", "tenant-a", "k"))
    err(r, 403, "tenant_mismatch")
    assert bridge.effects() == NO_EFFECTS


# --- invoke: happy path, validation, idempotency ----------------------------------------------------------------

@pytest.mark.parametrize("stage,fact", [("scout", "pulso_hypotheses"), ("verifier", "pulso_verification"),
                                        ("builder_design", "pulso_change_spec"), ("writer", "pulso_writer_receipts")])
def test_invoke_produces_receipt_and_stage_fact(bridge, stage, fact) -> None:
    body = bridge.invocation(stage=stage)
    r = bridge.invoke(body, key=f"k-{stage}")
    assert r.status_code == 200, r.text
    receipt = r.json()
    validate("CoreTaskReceipt", receipt)
    assert receipt["state"] == "terminal_ok" and receipt["outcome"] == "completed"
    assert receipt["runtime_profile"] == "contract_mock" and receipt["request_digest"] == body["request_digest"]
    assert list(receipt["result"]["facts"]) == [fact]
    assert receipt["result"]["facts"][fact]["source_kind"] == ("tool" if stage == "writer" else "agent")
    read = bridge.c.get(f"/internal/v1/core-tasks/{receipt['core_run_id']}", headers=bridge.headers("task_read"))
    assert read.status_code == 200
    validate("ReadRunResult", read.json())
    assert read.json() == receipt["result"]


def test_binding_is_recorded_before_any_effect(bridge) -> None:
    receipt = bridge.invoke().json()
    ledger = bridge.c.get("/_sim/bindings").json()
    assert [b["core_run_id"] for b in ledger] == [receipt["core_run_id"]]
    validate("CoreTaskBinding", ledger[0])
    assert ledger[0]["command_key"] == "k-1" and ledger[0]["tenant_id"] == "tenant-a"
    assert bridge.effects()["model_calls"] == 1


@pytest.mark.parametrize("mutate,status,code", [
    (lambda b: b.update(surprise=1), 400, "invalid_request"),
    (lambda b: b.update(stage="oracle"), 400, "stage_unknown"),
    (lambda b: b.update(attempt=-1), 400, "invalid_request"),
    (lambda b: b.update(request_digest="0" * 64), 400, "request_digest_invalid"),
    (lambda b: b.update(input={"blob": "x" * 262_145}), 413, "input_too_large"),
])
def test_invoke_body_rules(bridge, mutate, status, code) -> None:
    body = bridge.invocation()
    mutate(body)
    if code != "request_digest_invalid":
        body["request_digest"] = service_jws.request_digest(body)
    err(bridge.invoke(body), status, code)
    assert bridge.effects() == NO_EFFECTS


def test_missing_idempotency_key_is_invalid(bridge) -> None:
    r = bridge.c.post("/internal/v1/core-tasks/invoke", json=bridge.invocation(), headers=bridge.headers())
    err(r, 400, "invalid_request")


def test_same_key_same_digest_replays_without_reentering_the_engine(bridge) -> None:
    first = bridge.invoke().json()
    again = bridge.invoke()
    assert again.status_code == 200 and same(again.json(), first)
    assert bridge.effects()["model_calls"] == 1
    assert len(bridge.c.get("/_sim/bindings").json()) == 1


def test_same_key_other_digest_conflicts(bridge) -> None:
    bridge.invoke()
    err(bridge.invoke(bridge.invocation(attempt=1)), 409, "digest_conflict")
    assert bridge.effects()["model_calls"] == 1


def test_same_key_other_tenant_is_an_independent_command(bridge) -> None:
    a = bridge.invoke(key="shared").json()
    b = bridge.invoke(bridge.invocation(tenant="tenant-b"), key="shared", tenant="tenant-b").json()
    assert a["core_run_id"] != b["core_run_id"]
    assert bridge.effects()["model_calls"] == 2


# --- cross-tenant / run_not_found ---------------------------------------------------------------------------------

def test_foreign_tenant_run_is_not_visible(bridge) -> None:
    run = bridge.invoke().json()["core_run_id"]
    r = bridge.c.get(f"/internal/v1/core-tasks/{run}", headers=bridge.headers("task_read", "tenant-b"))
    err(r, 404, "run_not_found")


def test_unknown_run_is_run_not_found(bridge) -> None:
    err(bridge.c.get("/internal/v1/core-tasks/core-run-9999", headers=bridge.headers("task_read")), 404, "run_not_found")


def test_receipt_of_another_job_or_attempt_is_never_returned(bridge) -> None:
    bridge.invoke(bridge.invocation(job="job-1"), key="k-job")
    err(bridge.invoke(bridge.invocation(job="job-2"), key="k-job"), 409, "digest_conflict")
    err(bridge.invoke(bridge.invocation(attempt=1), key="k-job"), 409, "digest_conflict")


# --- death before / after binding -----------------------------------------------------------------------------------

def _die(bridge, mode: str) -> None:
    bridge.sim("fault", mode=mode)
    with pytest.raises(httpx.TransportError):
        bridge.invoke(key="k-die")


def test_death_before_binding_leaves_no_binding_no_effect_and_manual_reconcile(bridge) -> None:
    _die(bridge, "die_before_binding")
    assert bridge.c.get("/_sim/bindings").json() == []
    assert bridge.effects() == NO_EFFECTS
    bridge.sim("restart")
    retry = bridge.invoke(key="k-die")
    assert retry.status_code == 200
    receipt = retry.json()
    validate("CoreTaskReceipt", receipt)
    assert receipt["state"] == "manual_reconcile" and receipt["core_run_id"] is None and receipt["result"] is None
    assert bridge.effects() == NO_EFFECTS  # never re-executed
    assert same(bridge.invoke(key="k-die").json(), receipt)  # stable


def test_death_after_binding_is_unknown_and_the_run_was_never_committed(bridge) -> None:
    _die(bridge, "die_after_binding")
    ledger = bridge.c.get("/_sim/bindings").json()
    assert len(ledger) == 1
    bridge.sim("restart")
    receipt = bridge.invoke(key="k-die").json()
    validate("CoreTaskReceipt", receipt)
    assert receipt["state"] == "unknown" and receipt["core_run_id"] == ledger[0]["core_run_id"]
    assert receipt["result"] is None
    err(bridge.c.get(f"/internal/v1/core-tasks/{receipt['core_run_id']}", headers=bridge.headers("task_read")),
        404, "run_not_found")
    assert bridge.effects() == NO_EFFECTS
    assert len(bridge.c.get("/_sim/bindings").json()) == 1


def test_death_after_core_commit_recovers_the_terminal_result_exactly_once(bridge) -> None:
    _die(bridge, "die_after_commit")
    bridge.sim("restart")
    receipt = bridge.invoke(key="k-die").json()
    validate("CoreTaskReceipt", receipt)
    assert receipt["state"] == "terminal_ok" and receipt["result"]["facts"]
    assert bridge.effects()["model_calls"] == 1
    read = bridge.c.get(f"/internal/v1/core-tasks/{receipt['core_run_id']}", headers=bridge.headers("task_read"))
    assert read.status_code == 200


def test_restart_changes_the_bridge_instance_but_keeps_state(bridge) -> None:
    before = bridge.invoke().json()
    bridge.sim("restart")
    after = bridge.invoke().json()
    assert after["core_run_id"] == before["core_run_id"]
    v = bridge.c.get("/internal/v1/version", headers=bridge.headers("version_read")).json()
    assert v["bridge_instance_id"] != before["bridge_instance_id"]


@pytest.mark.parametrize("script", ["binding_conflict", "digest_mismatch", "command_unknown", "unavailable", "timeout"])
def test_binding_denied_means_zero_effects(bridge, script) -> None:
    bridge.sim("binding", script=[script])
    body = err(bridge.invoke(), 403, "binding_failed")
    assert body["details"]["callback"] == script
    assert bridge.effects() == NO_EFFECTS
    assert bridge.c.get("/_sim/bindings").json() == []
    assert bridge.invoke().json()["state"] == "manual_reconcile"


# --- bridge_busy --------------------------------------------------------------------------------------------------

def test_bridge_busy_is_429_retryable(bridge) -> None:
    bridge.sim("config", max_inflight=1)
    bridge.sim("fault", mode="latency", seconds=1.0)
    with cf.ThreadPoolExecutor(2) as pool:
        slow = pool.submit(bridge.invoke, bridge.invocation(job="slow"), "k-slow")
        time.sleep(0.4)
        busy = bridge.invoke(bridge.invocation(job="fast"), "k-fast")
        assert slow.result().status_code == 200
    body = err(busy, 429, "bridge_busy")
    assert body["retryable"] is True
    assert bridge.invoke(bridge.invocation(job="fast"), "k-fast").status_code == 200  # capacity is back
    assert bridge.effects()["model_calls"] == 2


def test_busy_does_not_block_replays_of_known_commands(bridge) -> None:
    first = bridge.invoke().json()
    bridge.sim("config", max_inflight=0)
    assert same(bridge.invoke().json(), first)
    err(bridge.invoke(bridge.invocation(job="new"), "k-new"), 429, "bridge_busy")


# --- release pin / outcomes -----------------------------------------------------------------------------------------

def test_unknown_release_is_pin_unavailable(bridge) -> None:
    err(bridge.invoke(bridge.invocation(release_id="rel-nope")), 409, "release_pin_unavailable")


def test_revoked_release_and_version_drift(bridge) -> None:
    bridge.sim("releases", release_id="rel-mock-0002", status="revoked", agents={"pulso-scout": "1.0.0"})
    err(bridge.invoke(bridge.invocation(release_id="rel-mock-0002")), 409, "release_revoked")
    err(bridge.invoke(bridge.invocation(agent_version="2.0.0"), key="k-drift"), 409, "release_drift")
    assert bridge.effects() == NO_EFFECTS


def test_failed_run_is_terminal_failed_with_outcome_failed(bridge) -> None:
    bridge.sim("outcomes", script=["failed"])
    receipt = bridge.invoke().json()
    validate("CoreTaskReceipt", receipt)
    assert receipt["state"] == "terminal_failed" and receipt["outcome"] == "failed" and receipt["result"]["facts"] == {}


def test_completed_without_the_required_fact_is_output_missing(bridge) -> None:
    bridge.sim("outcomes", script=["completed_without_fact"])
    err(bridge.invoke(), 422, "output_missing")
    assert bridge.invoke().json()["state"] == "terminal_failed"  # stored: never re-executed
    assert bridge.effects()["model_calls"] == 1


# --- alias / dry-run / credentials -------------------------------------------------------------------------------------

def test_alias_state(bridge) -> None:
    bridge.sim("aliases", agent_id="pulso-writer", alias="staging", release_id="rel-mock-0001")
    r = bridge.c.get("/internal/v1/core-state/aliases/pulso-writer/staging", headers=bridge.headers("state_read"))
    validate("AliasState", r.json())
    assert r.json()["release_id"] == "rel-mock-0001"
    other = bridge.c.get("/internal/v1/core-state/aliases/pulso-writer/prod", headers=bridge.headers("state_read"))
    assert other.json()["release_id"] is None
    err(bridge.c.get("/internal/v1/core-state/aliases/pulso-writer/canary", headers=bridge.headers("state_read")),
        404, "not_found")


def _dry(bridge, changes, tenant="tenant-a"):
    body = {"schema_version": "1", "tenant_id": tenant, "agent_id": "atencion", "base_release_id": "rel-98130317a1003849",
            "changes": changes}
    return bridge.c.post("/internal/v1/core-authoring/dry-run", json=body,
                         headers=bridge.headers("authoring_dry_run", tenant))


def _change(i=0, version="1.1.0"):
    return {"kind": "template", "docs": {"description": "d", "rationale": "r", "changelog": "c"},
            "content": {"id": f"t/x{i}", "version": version, "locales": {"es": "hola"}, "reads": []}}


def test_dry_run_validates_without_creating_a_proposal(bridge) -> None:
    ok = _dry(bridge, [_change()])
    validate("CoreAuthoringDryRun", ok.json())
    assert ok.json()["valid"] is True and ok.json()["candidate_hash"] and ok.json()["proposal_created"] is False
    bad = _dry(bridge, [_change(i) for i in range(51)])
    validate("CoreAuthoringDryRun", bad.json())
    assert bad.json()["valid"] is False and bad.json()["violations"][0]["rule"] == "REG-LIMIT"
    assert bad.json()["candidate_hash"] is None
    assert bridge.effects()["registry_writes"] == 0


def test_dry_run_is_deterministic(bridge) -> None:
    assert _dry(bridge, [_change()]).json() == _dry(bridge, [_change()]).json()


def test_dry_run_rejects_unknown_fields(bridge) -> None:
    body = {"schema_version": "1", "tenant_id": "tenant-a", "agent_id": "a", "base_release_id": None, "changes": [],
            "surprise": 1}
    r = bridge.c.post("/internal/v1/core-authoring/dry-run", json=body,
                      headers=bridge.headers("authoring_dry_run"))
    err(r, 400, "invalid_request")


def test_credential_issue_mints_a_core_principal_jws(bridge) -> None:
    body = {"schema_version": "1", "tenant_id": "tenant-a", "role": "constructor", "purpose": "credential_issue"}
    r = bridge.c.post("/internal/v1/core-credentials/issue", json=body, headers=bridge.headers("credential_issue"))
    assert r.status_code == 200, r.text
    out = r.json()
    validate("CoreCredentialIssue", out)
    principal = core_jws.verify(out["jws"])  # typ=principal+jws, EdDSA, exact header, sim key
    assert principal["roles"] == ["constructor"] and principal["type"] == "builder"
    assert principal["attrs"].get("actor") != "human"
    now = service_jws.parse_iso(bridge.c.get("/_sim/info").json()["now"])
    assert 0 < (service_jws.parse_iso(out["exp"]) - now).total_seconds() <= 900
    assert out["jws"] not in bridge.c.get("/_sim/invocations").text  # never persisted or logged


@pytest.mark.parametrize("role", ["aprobador", "admin", "human"])
def test_credential_issue_refuses_human_and_approval_roles(bridge, role) -> None:
    body = {"schema_version": "1", "tenant_id": "tenant-a", "role": role, "purpose": "credential_issue"}
    r = bridge.c.post("/internal/v1/core-credentials/issue", json=body, headers=bridge.headers("credential_issue"))
    err(r, 403, "forbidden")


def test_credential_issue_needs_its_purpose(bridge) -> None:
    body = {"schema_version": "1", "tenant_id": "tenant-a", "role": "constructor", "purpose": "credential_issue"}
    r = bridge.c.post("/internal/v1/core-credentials/issue", json=body, headers=bridge.headers("task_invoke"))
    err(r, 403, "forbidden")


# --- route table and /_sim ------------------------------------------------------------------------------------------

def test_route_table_is_exactly_the_bridge_contract(bridge) -> None:
    paths = bridge.c.get("/openapi.json").json()["paths"]
    table = sorted(f"{m.upper()} {p}" for p, ops in paths.items() if p.startswith("/internal/v1") for m in ops)
    assert table == sorted([
        "POST /internal/v1/core-tasks/invoke", "GET /internal/v1/core-tasks/{core_run_id}",
        "GET /internal/v1/core-state/aliases/{agent_id}/{alias}", "POST /internal/v1/core-authoring/dry-run",
        "GET /internal/v1/version", "POST /internal/v1/core-credentials/issue"])


def test_unknown_route_and_method_use_the_private_error_shape(bridge) -> None:
    err(bridge.c.get("/internal/v1/nope", headers=bridge.headers()), 404, "not_found")
    err(bridge.c.delete("/internal/v1/version", headers=bridge.headers()), 405, "method_not_allowed")


def test_sim_reset_clears_state_and_armed_faults(bridge) -> None:
    bridge.sim("fault", mode="die_before_binding")
    bridge.sim("reset")
    assert bridge.invoke().status_code == 200
    bridge.sim("reset")
    assert bridge.c.get("/_sim/bindings").json() == []
