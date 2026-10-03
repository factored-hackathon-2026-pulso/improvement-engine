"""Invoke / read flows: receipt state machine, idempotency, replay, tenant isolation, release pin, binding denial.
Needs a seeded scout release (`needs("invoke")`); the real world scripts the model, an external world supplies refs."""

from __future__ import annotations

import uuid

import pytest

from conformance.conftest import assert_error, assert_valid
from conformance.kit import CONTRACT, binding_ref, idempotency_key
from conformance.worlds import World

pytestmark = pytest.mark.needs("invoke")
SM = CONTRACT["receipt_state_machine"]


def nonce() -> str:
    return uuid.uuid4().hex[:10]


def run_scout(world: World, logical: str | None = None, **over):  # type: ignore[no-untyped-def]
    world.before_scout()
    logical = logical or f"s-{nonce()}"
    body = world.invocation("scout", f"job-{logical}", logical, extract_manifest_ref="ex-1", **over)
    return body, world.invoke(body)


def test_scout_happy_path_returns_a_terminal_ok_receipt_with_a_valid_fact(world: World) -> None:
    body, resp = run_scout(world)
    assert resp.status_code == 200, resp.text
    receipt = resp.json()
    assert_valid("CoreTaskReceipt", receipt)
    assert receipt["state"] == "terminal_ok" and receipt["outcome"] == "completed"
    key = idempotency_key(body["tenant_id"], body["job_id"], "scout", 1, body["logical_key"])
    assert receipt["task_binding_ref"] == binding_ref(body["tenant_id"], key)
    result = receipt["result"]
    assert_valid("ReadRunResult", result)
    fact = result["facts"]["pulso_hypotheses"]
    assert fact["source_kind"] == "agent"
    assert_valid("Fact_pulso_hypotheses", fact["value"])
    assert list(result["facts"]) == ["pulso_hypotheses"], "only the stage's whitelisted fact may leave the bridge"


def test_same_key_and_body_replays_the_stored_receipt_without_a_second_run(world: World) -> None:
    body, first = run_scout(world)
    assert first.status_code == 200
    again = world.invoke(body)
    assert again.status_code == 200
    assert again.json()["core_run_id"] == first.json()["core_run_id"]
    assert again.json()["state"] == "terminal_ok"
    assert_valid("CoreTaskReceipt", again.json())


def test_same_key_with_another_body_is_409_digest_conflict(world: World) -> None:
    body, first = run_scout(world)
    assert first.status_code == 200
    other = {**body, "input": {"briefing_ref": "wiki/other.md"}}
    key = idempotency_key(body["tenant_id"], body["job_id"], "scout", 1, body["logical_key"])
    assert_error(world.invoke(other, key=key), 409, "pulso:digest_conflict", retryable=False)


def test_a_correct_request_digest_is_accepted(world: World) -> None:
    world.before_scout()
    logical = f"d-{nonce()}"
    body = world.invocation("scout", f"job-{logical}", logical, extract_manifest_ref="ex-1")
    resp = world.invoke(body, with_digest=True)
    assert resp.status_code == 200, resp.text


def test_read_by_core_run_id_returns_the_same_terminal_receipt(world: World) -> None:
    _, resp = run_scout(world)
    run_id = resp.json()["core_run_id"]
    got = world.read_task(run_id)
    assert got.status_code == 200, got.text
    assert_valid("CoreTaskReceipt", got.json())
    assert got.json()["core_run_id"] == run_id and got.json()["state"] == "terminal_ok"
    assert got.json()["result"]["output_digest"] == resp.json()["result"]["output_digest"]


def test_read_of_a_foreign_tenants_run_is_404_not_found(world: World) -> None:
    _, resp = run_scout(world)
    foreign = world.read_task(resp.json()["core_run_id"], tenant=world.other_tenant)
    assert_error(foreign, 404, "pulso:not_found", retryable=False)


def test_read_of_an_unknown_id_is_404_not_found(world: World) -> None:
    assert_error(world.read_task("run-does-not-exist"), 404, "pulso:not_found")


@pytest.mark.parametrize("what", ["unknown_release", "wrong_version"])
def test_release_pin_failures_are_409_release_pin_unavailable(world: World, what: str) -> None:
    over = {"release_id": "rel-does-not-exist"} if what == "unknown_release" else {"agent_version": "9.9.9"}
    logical = f"p-{nonce()}"
    body = world.invocation("scout", f"job-{logical}", logical, **over)
    assert_error(world.invoke(body), 409, "pulso:release_pin_unavailable", retryable=False)


def test_a_pin_failure_is_terminal_and_a_retry_with_the_same_key_does_not_re_run(world: World) -> None:
    logical = f"p2-{nonce()}"
    body = world.invocation("scout", f"job-{logical}", logical, release_id="rel-does-not-exist")
    first = world.invoke(body)
    assert first.status_code == 409
    again = world.invoke(body)
    assert again.status_code == 200
    receipt = again.json()
    assert_valid("CoreTaskReceipt", receipt)
    assert receipt["state"] == "terminal_failed" and receipt["reason"] == "release_pin_unavailable"


def test_receipt_state_machine_values_are_the_published_closed_list(world: World) -> None:
    _, resp = run_scout(world)
    assert resp.json()["state"] in SM["states"]
    assert SM["terminal"] == ["terminal_failed", "terminal_ok"]
    assert resp.status_code == (200 if resp.json()["state"] in SM["terminal"] else 202)


@pytest.mark.needs("invoke", "control")
def test_a_denied_binding_never_reaches_the_model_and_is_not_terminal_ok(world: World) -> None:
    world.before_scout()
    before = world.control.model_calls
    world.control.deny_binding()
    try:
        logical = f"b-{nonce()}"
        body = world.invocation("scout", f"job-{logical}", logical, extract_manifest_ref="ex-1")
        resp = world.invoke(body)
    finally:
        world.control.restore()
    assert resp.status_code in (200, 202, 409, 403), resp.text
    if resp.status_code in (200, 202):
        receipt = resp.json()
        assert_valid("CoreTaskReceipt", receipt)
        assert receipt["state"] != "terminal_ok"
        assert receipt["state"] in {"manual_reconcile", "terminal_failed", "unknown", "sent"}
    assert world.control.model_calls == before, "zero model effect before a confirmed binding"
