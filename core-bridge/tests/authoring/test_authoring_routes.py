"""First RED of the R2 bridge gaps: `GET /core-state/aliases/{agent_id}/{alias}` (CAP-08) and
`POST /core-authoring/dry-run` (CAP-16 L2) on the isolated `/internal/v1` app, over Core's REAL registry
code (in-memory store seeded with the pinned `registry-demo` release). Doubles: in-memory store, FakeClock/FakeIds."""

from __future__ import annotations

import copy
import hashlib
import json
import time
import uuid
from datetime import UTC, datetime
from typing import Any

import pytest
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from fastapi.testclient import TestClient

from pulso_core_runtime.authoring.routes import AuthoringDeps, register
from pulso_core_runtime.authoring.service import AuthoringService
from pulso_core_runtime.internal.app import build_internal_app
from pulso_core_runtime.internal.auth import (
    InMemoryJtiStore,
    ServiceJwtVerifier,
    ServiceKey,
    sign_service_jwt,
)

pytestmark = pytest.mark.runtime

CP = Ed25519PrivateKey.generate()
INFO = {"agent_core_sha": "x", "contracts_version": "1.3.0", "pulso_sha": "p", "image_digest": "d",
        "runtime_profile": "agent_core_real", "doubles": []}
AGENT = "atencion"


def _app(store: Any, service: Any, profile: str = "agent_core_real") -> TestClient:
    handlers: dict[str, Any] = {}
    register(handlers, AuthoringDeps(AuthoringService(store, service, runtime_profile=profile)))
    keys = {"cp1": ServiceKey("control-api", "core-bridge", CP.public_key())}
    app = build_internal_app(ServiceJwtVerifier(keys, InMemoryJtiStore()), version_info=lambda: INFO,
                             handlers=handlers)
    return TestClient(app, raise_server_exceptions=False)


def _token(purpose: str, tenant: str = "t1") -> str:
    now = int(time.time())
    return sign_service_jwt(CP, kid="cp1", claims={
        "iss": "control-api", "aud": "core-bridge", "sub": "worker:1", "tenant_id": tenant, "purpose": purpose,
        "job_id": "j1", "iat": now, "exp": now + 60, "jti": uuid.uuid4().hex})


class World:
    def __init__(self) -> None:
        from agent_core.registry.memory import InMemoryRegistryStore
        from agent_core.registry.service import RegistryService
        from testing.fakes.clock import FakeClock
        from testing.fakes.ids import FakeIds
        from tests.registry.service_world import FakeEvaluator, seed_demo

        self.store = InMemoryRegistryStore()
        self.base = seed_demo(self.store)
        self.service = RegistryService(self.store, FakeEvaluator(), FakeClock(), FakeIds())
        self.client = _app(self.store, self.service)

    def alias(self, agent: str, alias: str, tenant: str = "t1") -> Any:
        return self.client.get(f"/core-state/aliases/{agent}/{alias}",
                               headers={"Authorization": f"Bearer {_token('alias_read', tenant)}"})

    def dry(self, body: dict[str, Any], tenant: str = "t1") -> Any:
        return self.client.post("/core-authoring/dry-run", json=body,
                                headers={"Authorization": f"Bearer {_token('authoring_dry_run', tenant)}"})

    def counts(self) -> tuple[int, int, int]:
        with self.store.transaction() as tx:
            created = tx.count_created_after("auto_detect", datetime(2000, 1, 1, tzinfo=UTC))
            return len(tx.events()), len(tx.list_versions("prompt", "p/resumen_radicado")), created


def _draft(version: str = "1.1.0", **over: Any) -> dict[str, Any]:
    from tests.registry.helpers import prompt_draft

    return prompt_draft(version=version, **over).model_dump(mode="json")


def _req(world: World, changes: list[dict[str, Any]] | None = None, **over: Any) -> dict[str, Any]:
    return {"schema_version": "1", "tenant_id": "t1", "agent_id": AGENT, "base_release_id": world.base,
            "changes": changes if changes is not None else [_draft()], **over}


def _digest(body: dict[str, Any]) -> str:
    from agent_core.domain import canonical_bytes

    # independent recomputation of plan D.1: sha256(JCS(functional body without request_digest))
    return hashlib.sha256(canonical_bytes({k: v for k, v in body.items() if k != "request_digest"})).hexdigest()


# ---- alias read (CAP-08) -------------------------------------------------------------------------------

def test_alias_read_returns_observed_alias_without_writes() -> None:
    w = World()
    before = w.counts()
    r = w.alias(AGENT, "staging")
    assert r.status_code == 200
    body = r.json()
    assert body["schema_version"] == "1" and body["agent_id"] == AGENT and body["alias"] == "staging"
    assert body["release_id"] == w.base and body["status"] == "active" and body["source"] == "core_store"
    assert body["observed_at"].endswith("Z") and body["runtime_profile"] == "agent_core_real"
    assert w.counts() == before


def test_alias_unknown_is_typed_and_indistinguishable_for_agent_or_alias() -> None:
    w = World()
    unknown_agent, unset_alias = w.alias("no-such-agent", "staging"), w.alias("otro", "prod")
    for r in (unknown_agent, unset_alias):
        assert r.status_code == 404 and r.json()["code"] == "pulso:alias_unknown" and r.json()["details"] == {}
    assert unknown_agent.json() == unset_alias.json() | {"trace_id": unknown_agent.json()["trace_id"]}


def test_alias_name_outside_staging_prod_is_invalid_request() -> None:
    r = World().alias(AGENT, "canary")
    assert r.status_code == 422 and r.json()["code"] == "pulso:invalid_request"


def test_alias_reports_revoked_status() -> None:
    w = World()
    with w.store.transaction() as tx:  # Core's revoke needs a promoted-away alias; flip the status directly
        tx.set_release_status(w.base, "revoked")
    assert w.alias(AGENT, "staging").json()["status"] == "revoked"


def test_alias_requires_alias_read_purpose() -> None:
    w = World()
    r = w.client.get(f"/core-state/aliases/{AGENT}/staging",
                     headers={"Authorization": f"Bearer {_token('core_task_read')}"})
    assert r.status_code == 403 and r.json()["code"] == "pulso:auth_denied"


# ---- authoring dry-run (CAP-16 L2) ---------------------------------------------------------------------

def test_dry_run_valid_matches_freeze_candidate_hash_and_writes_nothing() -> None:
    from agent_core.registry.models import Origin
    from tests.registry.helpers import human, prompt_draft

    w = World()
    before = w.counts()
    r = w.dry(_req(w))
    assert r.status_code == 200, r.text
    out = r.json()
    assert out["valid"] is True and out["violations"] == [] and out["proposal_created"] is False
    assert len(out["candidate_hash"]) == 64 and out["release_id_preview"] == "rel-" + out["candidate_hash"][:16]
    assert len(out["release_hash"]) == 64
    assert w.counts() == before  # no proposal, event, version or quota consumed
    # Parity: the REAL freeze of the same draft produces the same candidate_hash (CC-08).
    ana = human()
    p = w.service.create_proposal(ana, AGENT, Origin.manual, "t")
    w.service.put_draft(ana, p.proposal_id, [prompt_draft(version="1.1.0")], expected_rev=0)
    view = w.service.freeze(ana, p.proposal_id)
    assert view.candidate_hash == out["candidate_hash"] and view.release_id_preview == out["release_id_preview"]
    assert out["new_versions"] == [v.model_dump(mode="json") for v in view.new_versions]
    assert out["auto_bumped"] == [v.model_dump(mode="json") for v in view.auto_bumped]
    assert out["content_hashes"] and all(len(h) == 64 for h in out["content_hashes"].values())


def test_dry_run_is_deterministic_idempotent_and_binds_request_digest() -> None:
    w = World()
    body = _req(w)
    a, b = w.dry(body).json(), w.dry(copy.deepcopy(body)).json()
    assert a == b and a["request_digest"] == _digest(body)
    reordered = {k: body[k] for k in reversed(list(body))}
    assert w.dry(reordered).json() == a  # key order is irrelevant (JCS)
    assert w.dry(_req(w, [_draft("1.2.0")])).json()["request_digest"] != a["request_digest"]


def test_dry_run_supplied_digest_must_match() -> None:
    w = World()
    body = _req(w)
    assert w.dry({**body, "request_digest": _digest(body)}).status_code == 200
    bad = w.dry({**body, "request_digest": "0" * 64})
    assert bad.status_code == 422 and bad.json()["code"] == "pulso:invalid_request"
    assert bad.json()["details"] == {"fields": ["request_digest"]}


def test_dry_run_violation_is_200_not_success_with_core_rule_names() -> None:
    w = World()
    r = w.dry(_req(w, [_draft("0.9.0")]))  # not greater than the base version
    out = r.json()
    assert r.status_code == 200 and out["valid"] is False and out["candidate_hash"] is None
    assert out["release_id_preview"] is None
    v = out["violations"][0]
    assert v["rule"] == "REG-VERSION" and set(v) == {"rule", "path", "flow", "node_id", "message"}


def test_dry_run_unreferenced_new_entity_and_schema_problem() -> None:
    from agent_core.registry.models import EntityDraft
    from tests.registry.helpers import docs

    w = World()
    orphan = EntityDraft(kind="prompt", content={
        "id": "p/nadie", "version": "1.0.0", "locales": {"es": "x", "pt": "y"},
        "model_profile": "perfil-generacion@1.0.0"}, docs=docs()).model_dump(mode="json")
    out = w.dry(_req(w, [orphan])).json()
    assert out["valid"] is False and {v["rule"] for v in out["violations"]} == {"REG-UNREFERENCED"}
    bad_shape = {"kind": "prompt", "content": {"no_id": "secret-value"}, "docs": _draft()["docs"]}
    out = w.dry(_req(w, [bad_shape])).json()
    assert out["valid"] is False and out["violations"][0]["rule"] == "REG-SCHEMA"
    assert "secret-value" not in json.dumps(out)  # never echoes input values


def test_dry_run_limits_50_changes_262144_bytes_and_200_nodes() -> None:
    w = World()
    out = w.dry(_req(w, [_draft(f"1.{i + 1}.0") for i in range(51)])).json()
    assert out["valid"] is False and out["violations"][0]["rule"] == "REG-LIMIT"
    out = w.dry(_req(w, [_draft(text="x" * 262_200)])).json()
    assert out["valid"] is False and any(v["rule"] == "REG-LIMIT" for v in out["violations"])
    at_limit = w.dry(_req(w, [_draft(f"1.{i + 1}.0") for i in range(50)])).json()  # 50 is allowed by the limit
    assert all(v["rule"] != "REG-LIMIT" for v in at_limit["violations"])


def test_dry_run_node_limit_is_enforced_by_core_validation_with_the_configured_limit() -> None:
    """Core's default is 200 nodes (`Limits.max_flow_nodes`); a >200-node reachable flow is awkward to author, so the
    wiring is proven with the same code path and a tight limit, and the default is asserted separately."""
    from agent_core.registry.models import EntityDraft
    from agent_core.registry.validation import DEFAULT_LIMITS, Limits
    from tests.registry.helpers import docs

    assert DEFAULT_LIMITS.max_flow_nodes == 200 and DEFAULT_LIMITS.max_changes == 50
    assert DEFAULT_LIMITS.max_entity_bytes == 262_144
    w = World()
    with w.store.transaction() as tx:
        ref = next(r for r in tx.release_refs(w.base) if r.kind == "flow")
        flow = json.loads(tx.blobs.get(tx.get_version(ref).content_hash))
    flow["version"] = "9.0.0"
    d = EntityDraft(kind="flow", content=flow, docs=docs()).model_dump(mode="json")
    handlers: dict[str, Any] = {}
    register(handlers, AuthoringDeps(AuthoringService(w.store, w.service, runtime_profile="x",
                                                      limits=Limits(max_flow_nodes=5))))
    keys = {"cp1": ServiceKey("control-api", "core-bridge", CP.public_key())}
    client = TestClient(build_internal_app(ServiceJwtVerifier(keys, InMemoryJtiStore()), version_info=lambda: INFO,
                                           handlers=handlers), raise_server_exceptions=False)
    out = client.post("/core-authoring/dry-run", json=_req(w, [d]),
                      headers={"Authorization": f"Bearer {_token('authoring_dry_run')}"}).json()
    assert out["valid"] is False and any(v["rule"] == "REG-LIMIT" for v in out["violations"])


def test_dry_run_denies_release_settings_kind() -> None:
    w = World()
    before = w.counts()
    settings = {"kind": "release_settings", "content": {"interrupts": []}, "docs": _draft()["docs"]}
    r = w.dry(_req(w, [_draft(), settings]))
    assert r.status_code == 422 and r.json()["code"] == "pulso:release_settings_not_allowed"
    assert w.counts() == before


def test_dry_run_platform_guardrail_edit_is_a_violation() -> None:
    from agent_core.registry.models import EntityDraft
    from tests.registry.helpers import docs, suite_content

    w = World()
    suite = suite_content(thresholds={"platform_pii_leak": 0})
    d = EntityDraft(kind="eval_suite", content=suite, docs=docs()).model_dump(mode="json")
    out = w.dry(_req(w, [_draft(), d])).json()
    assert out["valid"] is False and any(v["rule"] == "REG-PLATFORM-EDIT" for v in out["violations"])


def test_dry_run_request_validation_and_scoping() -> None:
    w = World()
    assert w.dry({**_req(w), "extra": 1}).json()["code"] == "pulso:invalid_request"
    assert w.dry({k: v for k, v in _req(w).items() if k != "changes"}).status_code == 422
    r = w.dry(_req(w), tenant="t2")  # body tenant != verified claim
    assert r.status_code == 403 and r.json()["code"] == "pulso:tenant_mismatch"
    r = w.dry(_req(w, base_release_id="rel-nope"))
    assert r.status_code == 404 and r.json()["code"] == "pulso:base_release_unknown"
    assert w.dry(_req(w, changes=[])).status_code == 200  # empty draft: the base alone is a candidate


def test_dry_run_new_agent_without_base_is_reported_not_crashing() -> None:
    w = World()
    out = w.dry(_req(w, base_release_id=None, agent_id="nuevo")).json()
    assert out["valid"] is False and out["candidate_hash"] is None


def test_dry_run_store_outage_is_503_retryable() -> None:
    w = World()

    class Down:
        def transaction(self) -> Any:
            raise OSError("db down")

    client = _app(Down(), w.service)
    r = client.post("/core-authoring/dry-run", json=_req(w),
                    headers={"Authorization": f"Bearer {_token('authoring_dry_run')}"})
    assert r.status_code == 503 and r.json()["code"] == "pulso:dry_run_unavailable" and r.json()["retryable"]


def test_real_responses_validate_against_the_bridge_mock_schemas() -> None:
    """Mock parity: the bodies of the real routes satisfy `platform-sim/bridge_mock/schemas` (the shared contract)."""
    from pathlib import Path

    from jsonschema import Draft202012Validator

    schemas = Path(__file__).resolve().parents[3] / "platform-sim" / "bridge_mock" / "schemas"

    def check(name: str, instance: Any) -> None:
        Draft202012Validator(json.loads((schemas / f"{name}.schema.json").read_text(encoding="utf-8"))).validate(instance)

    w = World()
    check("AliasState", w.alias(AGENT, "staging").json())
    check("BridgeError", w.alias("nobody", "staging").json())
    check("CoreAuthoringDryRunRequest", _req(w))
    check("CoreAuthoringDryRun", w.dry(_req(w)).json())
    check("CoreAuthoringDryRun", w.dry(_req(w, [_draft("0.9.0")])).json())
