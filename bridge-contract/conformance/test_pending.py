"""Routes marked `x-status: pending-implementation` in the contract (alias read, authoring dry-run).

These tests specify the V3 CAP-08/16/17 behaviour. While a target has not implemented a route (it answers 501
`pulso:not_implemented`, or 404 for an unregistered path) the test SKIPS with that reason; once the route exists the
assertions are live. Regenerate the contract (`python bridge-contract/gen.py`) when the runtime's final shapes land."""

from __future__ import annotations

import pytest

from conformance.conftest import assert_error, assert_valid
from conformance.worlds import World

pytestmark = pytest.mark.pending_route


def _skip_if_absent(resp) -> None:  # type: ignore[no-untyped-def]
    if resp.status_code == 501:
        pytest.skip("pending-implementation: route authenticated but not implemented (501)")
    if resp.status_code == 404 and resp.json().get("code") == "pulso:not_found":
        pytest.skip("pending-implementation: route path is not registered (404 not_found)")


def alias(world: World, agent: str, name: str, **kw):  # type: ignore[no-untyped-def]
    return world.api.call("GET", f"/core-state/aliases/{agent}/{name}", purpose="alias_read", **kw)


def dry_run(world: World, body: dict, **kw):  # type: ignore[no-untyped-def]
    return world.api.call("POST", "/core-authoring/dry-run", purpose="authoring_dry_run", body=body, **kw)


def dry_body(world: World, **over) -> dict:  # type: ignore[no-untyped-def]
    body = {"schema_version": "1", "tenant_id": world.tenant, "agent_id": world.arm_agent, "base_release_id": None,
            "changes": []}
    body.update(over)
    return body


def test_alias_read_returns_a_valid_alias_state(world: World) -> None:
    resp = alias(world, world.arm_agent, "prod")
    _skip_if_absent(resp)
    assert resp.status_code == 200, resp.text
    body = resp.json()
    assert_valid("AliasState", body)
    assert body["agent_id"] == world.arm_agent and body["alias"] == "prod"


def test_alias_read_of_an_unknown_agent_is_404_alias_unknown(world: World) -> None:
    resp = alias(world, "no-such-agent", "prod")
    _skip_if_absent(resp)
    assert_error(resp, 404, "pulso:alias_unknown")


def test_alias_name_outside_staging_prod_is_422(world: World) -> None:
    resp = alias(world, world.arm_agent, "canary")
    _skip_if_absent(resp)
    assert_error(resp, 422, "pulso:invalid_request")


def test_dry_run_body_tenant_must_equal_the_claim(world: World) -> None:
    resp = dry_run(world, dry_body(world, tenant_id=world.other_tenant))
    _skip_if_absent(resp)
    assert_error(resp, 403, "pulso:tenant_mismatch")


def test_dry_run_request_is_a_closed_dto(world: World) -> None:
    resp = dry_run(world, dry_body(world, surprise=1))
    _skip_if_absent(resp)
    assert_error(resp, 422, "pulso:invalid_request")


def test_dry_run_with_violations_is_http_200_without_a_candidate_hash(world: World) -> None:
    resp = dry_run(world, dry_body(world, changes=[{"kind": "no-such-kind", "content": {}, "docs": {}}]))
    _skip_if_absent(resp)
    assert resp.status_code == 200, resp.text
    body = resp.json()
    assert_valid("CoreAuthoringDryRun", body)
    assert body["violations"] and body["candidate_hash"] is None, "200 with violations is NOT success"
    assert body.get("proposal_created", False) is False


def test_dry_run_never_creates_a_proposal(world: World) -> None:
    resp = dry_run(world, dry_body(world))
    _skip_if_absent(resp)
    assert resp.status_code == 200, resp.text
    assert_valid("CoreAuthoringDryRun", resp.json())
    assert resp.json().get("proposal_created", False) is False
