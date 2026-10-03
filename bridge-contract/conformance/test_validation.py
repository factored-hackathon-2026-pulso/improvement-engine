"""Request validation, size limits, version and credential routes. None of these need seeded Core state: every case is
decided before a receipt or an effect exists (zero-effect refusals)."""

from __future__ import annotations

import pytest

from conformance.conftest import assert_error, assert_valid
from conformance.kit import (
    CONTRACT,
    binding_ref,
    idempotency_key,
    request_digest,
    schema_errors,
)
from conformance.worlds import World

LIM = CONTRACT["limits"]


def scout(world: World, logical: str = "v", **over):  # type: ignore[no-untyped-def]
    return world.invocation("scout", "job-val", logical, **over)


# -- size limits -------------------------------------------------------------------------------------------------
def test_body_over_the_cap_is_413_payload_too_large(world: World) -> None:
    raw = b'{"pad":"' + b"x" * (LIM["max_body_bytes"] + 10) + b'"}'
    resp = world.api.call("POST", "/core-tasks/invoke", purpose="core_task_invoke", raw=raw,
                          headers={"Idempotency-Key": "0" * 64})
    assert_error(resp, 413, "pulso:payload_too_large", retryable=False)


def test_input_over_256_kib_is_413_input_too_large_with_zero_effects(world: World) -> None:
    body = scout(world, "big", input={"briefing_ref": "x" * (LIM["max_input_bytes"] + 1024)})
    assert len(str(body)) < LIM["max_body_bytes"]
    resp = world.invoke(body)
    assert_error(resp, 413, "pulso:input_too_large", retryable=False)


# -- DTO validation (extra=forbid, closed stage list, catalogue rules) --------------------------------------------
@pytest.mark.parametrize("raw", [b"{", b"[]", b"null", b'"x"'], ids=["broken-json", "array", "null", "string"])
def test_non_object_bodies_are_422_invalid_request(world: World, raw: bytes) -> None:
    resp = world.api.call("POST", "/core-tasks/invoke", purpose="core_task_invoke", raw=raw,
                          headers={"Idempotency-Key": "0" * 64})
    assert_error(resp, 422, "pulso:invalid_request")


def test_unknown_field_is_422_and_names_the_field(world: World) -> None:
    body = scout(world, "extra", surprise=1)
    resp = world.invoke(body)
    err = assert_error(resp, 422, "pulso:invalid_request")
    assert "surprise" in err["details"]["fields"]


def test_missing_required_field_is_422(world: World) -> None:
    body = scout(world, "missing")
    del body["logical_key"]
    resp = world.invoke(body, key="0" * 64)
    err = assert_error(resp, 422, "pulso:invalid_request")
    assert "logical_key" in err["details"]["fields"]


def test_invocation_schema_accepts_what_the_builder_makes_and_rejects_drift(world: World) -> None:
    good = scout(world, "schema")
    assert not schema_errors("CoreTaskInvocation", good)
    for bad in ({**good, "surprise": 1}, {**good, "attempt": 0}, {**good, "agent_version": "1.0"},
                {**good, "stage": "nope"}, {**good, "agent_id": "pulso-writer"}, {**good, "input": {"zzz": 1}},
                {**good, "registry_mutation_commitment": {"mode": "write", "operations": []}}):
        assert schema_errors("CoreTaskInvocation", bad), bad


def test_unknown_stage_is_400_stage_unknown(world: World) -> None:
    body = {**scout(world, "stage"), "stage": "nope"}
    resp = world.invoke(body, key=idempotency_key(world.tenant, body["job_id"], "nope", 1, "stage"))
    err = assert_error(resp, 400, "pulso:stage_unknown")
    assert err["details"] == {"stage": "nope"}


def test_stage_agent_mismatch_is_422_before_any_receipt(world: World) -> None:
    body = scout(world, "mismatch", agent_id="pulso-writer")
    err = assert_error(world.invoke(body), 422, "pulso:stage_agent_mismatch")
    assert err["details"] == {"stage": "scout", "agent_id": "pulso-writer"}


def test_input_slot_outside_the_stage_catalogue_is_400(world: World) -> None:
    body = scout(world, "slot", input={"briefing_ref": "a", "not_a_slot": 1})
    err = assert_error(world.invoke(body), 400, "pulso:unknown_input_slot")
    assert err["details"]["slots"] == ["not_a_slot"]


def test_commitment_on_a_non_writer_stage_is_422(world: World) -> None:
    body = scout(world, "commit", registry_mutation_commitment={"mode": "write", "operations": []})
    assert_error(world.invoke(body), 422, "pulso:invalid_request")


def test_idempotency_key_must_follow_the_formula(world: World) -> None:
    body = scout(world, "idem")
    wrong = world.invoke(body, key="not-the-formula")
    assert assert_error(wrong, 422, "pulso:invalid_request")["details"]["fields"] == ["Idempotency-Key"]
    absent = world.api.call("POST", "/core-tasks/invoke", purpose="core_task_invoke", body=body, job_id=body["job_id"])
    assert assert_error(absent, 422, "pulso:invalid_request")["details"]["fields"] == ["Idempotency-Key"]


def test_request_digest_must_match_when_present(world: World) -> None:
    body = {**scout(world, "digest"), "request_digest": "0" * 64}
    err = assert_error(world.invoke(body), 422, "pulso:invalid_request")
    assert err["details"]["fields"] == ["request_digest"]
    assert request_digest(scout(world, "digest")) != "0" * 64


def test_body_tenant_must_equal_the_signed_tenant(world: World) -> None:
    body = scout(world, "tenant")
    resp = world.invoke(body, tenant=world.other_tenant)
    assert_error(resp, 403, "pulso:tenant_mismatch")


# -- version -------------------------------------------------------------------------------------------------------
def test_version_reports_the_pinned_core_and_validates(world: World) -> None:
    resp = world.api.call("GET", "/version", purpose="version_probe")
    assert resp.status_code == 200, resp.text
    body = resp.json()
    assert_valid("CoreVersion", body)
    assert body["agent_core_sha"] == CONTRACT["pin"]["agent_core_sha"]
    assert body["contracts_version"] == CONTRACT["pin"]["contracts_version"]


# -- credentials ---------------------------------------------------------------------------------------------------
@pytest.mark.needs("credentials")
@pytest.mark.parametrize("policy", CONTRACT["credential_policy"], ids=lambda p: f"{p['purpose']}-{p['role']}")
def test_credential_issue_succeeds_for_policy_rows(world: World, policy: dict) -> None:
    resp = world.api.call("POST", "/core-credentials/issue", purpose="credential_issue",
                          body={"tenant_id": world.tenant, "role": policy["role"], "purpose": policy["purpose"]})
    assert resp.status_code == 200, resp.text
    assert resp.headers.get("cache-control") == "no-store"
    body = resp.json()
    assert_valid("CoreCredentialIssue", body)
    head = __import__("json").loads(__import__("base64").urlsafe_b64decode(body["jws"].split(".")[0] + "=="))
    assert head["typ"] == "principal+jws" and head["alg"] == "EdDSA" and head["kid"] == body["kid"]


@pytest.mark.needs("credentials")
@pytest.mark.parametrize("role,purpose", [("approver", "registry_write"), ("constructor", "approve"),
                                          ("human", "registry_write"), ("admin", "core_task")])
def test_credential_issue_refuses_everything_outside_the_policy(world: World, role: str, purpose: str) -> None:
    resp = world.api.call("POST", "/core-credentials/issue", purpose="credential_issue",
                          body={"tenant_id": world.tenant, "role": role, "purpose": purpose})
    assert_error(resp, 403, "pulso:credential_not_issuable", retryable=False)


@pytest.mark.needs("credentials")
def test_credential_issue_for_another_tenant_is_tenant_mismatch(world: World) -> None:
    resp = world.api.call("POST", "/core-credentials/issue", purpose="credential_issue",
                          body={"tenant_id": world.other_tenant, "role": "constructor", "purpose": "registry_write"})
    assert_error(resp, 403, "pulso:tenant_mismatch")


@pytest.mark.needs("credentials")
@pytest.mark.parametrize("body", [{"tenant_id": "t1", "role": "constructor"},
                                  {"tenant_id": "t1", "role": "constructor", "purpose": "core_task", "x": 1}],
                         ids=["missing-purpose", "extra-field"])
def test_credential_request_is_closed(world: World, body: dict) -> None:
    body = {**body, "tenant_id": world.tenant}
    assert_error(world.api.call("POST", "/core-credentials/issue", purpose="credential_issue", body=body),
                 422, "pulso:invalid_request")


def test_binding_ref_is_derivable_from_tenant_and_key(world: World) -> None:
    """Rust may compute the invocation's binding_ref before dispatch (admissions bind to it)."""
    key = idempotency_key("t", "j", "scout", 1, "k")
    assert binding_ref("t", key) == __import__("hashlib").sha256(f"t|{key}".encode()).hexdigest()
