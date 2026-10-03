"""Alias read (CAP-08) and authoring dry-run (CAP-16 L2): implemented by the runtime, so every assertion is live.

A target that does not implement a route fails these tests (501 / unregistered path are no longer skipped)."""

from __future__ import annotations

from conformance.conftest import assert_error, assert_valid
from conformance.worlds import World


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
    assert resp.status_code == 200, resp.text
    body = resp.json()
    assert_valid("AliasState", body)
    assert body["agent_id"] == world.arm_agent and body["alias"] == "prod"


def test_alias_read_of_an_unknown_agent_is_404_alias_unknown(world: World) -> None:
    resp = alias(world, "no-such-agent", "prod")
    assert_error(resp, 404, "pulso:alias_unknown")


def test_alias_name_outside_staging_prod_is_422(world: World) -> None:
    resp = alias(world, world.arm_agent, "canary")
    assert_error(resp, 422, "pulso:invalid_request")


def test_dry_run_body_tenant_must_equal_the_claim(world: World) -> None:
    resp = dry_run(world, dry_body(world, tenant_id=world.other_tenant))
    assert_error(resp, 403, "pulso:tenant_mismatch")


def test_dry_run_request_is_a_closed_dto(world: World) -> None:
    resp = dry_run(world, dry_body(world, surprise=1))
    assert_error(resp, 422, "pulso:invalid_request")


def test_dry_run_with_violations_is_http_200_without_a_candidate_hash(world: World) -> None:
    resp = dry_run(world, dry_body(world, changes=[{"kind": "no-such-kind", "content": {}, "docs": {}}]))
    assert resp.status_code == 200, resp.text
    body = resp.json()
    assert_valid("CoreAuthoringDryRun", body)
    assert body["violations"] and body["candidate_hash"] is None, "200 with violations is NOT success"
    assert body.get("proposal_created", False) is False


def test_dry_run_never_creates_a_proposal(world: World) -> None:
    resp = dry_run(world, dry_body(world))
    assert resp.status_code == 200, resp.text
    assert_valid("CoreAuthoringDryRun", resp.json())
    assert resp.json().get("proposal_created", False) is False


def test_dry_run_of_the_base_release_alone_is_valid_with_a_candidate_hash(world: World) -> None:
    resp = dry_run(world, dry_body(world, base_release_id=world.arm_release))
    assert resp.status_code == 200, resp.text
    body = resp.json()
    assert_valid("CoreAuthoringDryRun", body)
    assert body["valid"] is True and not body["violations"], body
    assert len(body["candidate_hash"]) == 64 and body["release_id_preview"] == "rel-" + body["candidate_hash"][:16]
    assert body.get("proposal_created", False) is False


def test_dry_run_with_an_unknown_base_release_is_404_base_release_unknown(world: World) -> None:
    assert_error(dry_run(world, dry_body(world, base_release_id="rel-nope")), 404, "pulso:base_release_unknown")
