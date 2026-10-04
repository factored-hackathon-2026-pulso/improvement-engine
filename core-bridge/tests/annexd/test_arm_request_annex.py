"""Annex D.4 (b)(c)(f) for arms: annex names, deprecated aliases (one release), Idempotency-Key header, deadline format.
Real PG16 for the runner; the header merge is a route concern and runs on a stub runner."""

from __future__ import annotations

from typing import Any

import pytest
from fastapi.testclient import TestClient

from l5.test_arms import FakeBank, req, runner
from l5.test_evaluate_path import World
from l5.world import AGENT
from pulso_core_runtime.evaluation.arms import ArmDenied

pytestmark = [pytest.mark.runtime, pytest.mark.pg]

LEGACY = {"mode", "seed_manifest_ref", "agent_id"}


def annex_req(key: str = "k1", **over: Any) -> dict[str, Any]:
    """Annex D.4 names: no mode / agent_id / seed_manifest_ref."""
    base = {k: v for k, v in req(key).items() if k not in LEGACY}
    return {**base, "execution_profile": "evolution_task", "sandbox_session_ref": None,
            "deadline": "2030-01-01T00:00:00Z", **over}


def test_annex_named_request_runs_without_the_deprecated_fields(pg) -> None:  # type: ignore[no-untyped-def]
    w = World(pg)
    rep = runner(w).run(annex_req(), tenant_id="t1").report or {}
    assert rep["status"] == "completed", rep


def test_deprecated_aliases_and_annex_names_are_the_same_request(pg) -> None:  # type: ignore[no-untyped-def]
    w = World(pg)
    r = runner(w)
    first = r.run(req(), tenant_id="t1").report  # legacy: mode=native, agent_id
    jobs = w.storage.jobs
    again = r.run(annex_req(deadline=None, agent_id=AGENT), tenant_id="t1")  # same key, same meaning in annex names -> replay
    assert again.report == first and w.storage.jobs == jobs


def test_attention_profile_uses_sandbox_session_ref_like_the_old_seed_manifest_ref(pg) -> None:  # type: ignore[no-untyped-def]
    w = World(pg)
    bank = FakeBank()
    rep = runner(w, bank).run(annex_req(execution_profile="attention_stateful_complementary",
                                        sandbox_session_ref="seed-1"), tenant_id="t1").report or {}
    assert rep["status"] == "completed" and bank.opened >= 1, rep


def test_attention_profile_without_a_sandbox_ref_is_sandbox_required(pg) -> None:  # type: ignore[no-untyped-def]
    w = World(pg)
    with pytest.raises(ArmDenied) as e:
        runner(w, FakeBank()).run(annex_req(execution_profile="attention_stateful_complementary"), tenant_id="t1")
    assert (e.value.code, e.value.status) == ("sandbox_required", 409)


@pytest.mark.parametrize("over", [
    {"mode": "stateful_attention"},                       # alias contradicts execution_profile=evolution_task
    {"execution_profile": "nope"},                         # not an annex profile
    {"sandbox_session_ref": "a", "seed_manifest_ref": "b"},  # alias contradicts its annex name
    {"deadline": "2030-01-01T00:00:00+00:00"},             # not Z-suffixed
    {"deadline": "2030-01-01T00:00:00"},                   # naive
    {"deadline": "tomorrow"},
])
def test_contradictions_and_bad_formats_are_422(pg, over: dict[str, Any]) -> None:  # type: ignore[no-untyped-def]
    w = World(pg)
    with pytest.raises(ArmDenied) as e:
        runner(w, FakeBank()).run(annex_req(**over), tenant_id="t1")
    assert (e.value.code, e.value.status) == ("invalid_request", 422)


def test_agent_id_alias_must_match_the_target_agent(pg) -> None:  # type: ignore[no-untyped-def]
    w = World(pg)
    rep = runner(w).run(annex_req(agent_id="other-agent"), tenant_id="t1").report or {}
    assert rep["status"] == "failed_infra" and rep["detail"] == "target_agent_mismatch"
    assert AGENT != "other-agent"


def test_a_retry_with_a_later_deadline_replays_the_same_report(pg) -> None:  # type: ignore[no-untyped-def]
    """`deadline` is a per-attempt bound, not identity: a Rust retry recomputes it and must not get a 409."""
    w = World(pg)
    r = runner(w)
    first = r.run(annex_req(deadline="2030-01-01T00:00:00Z"), tenant_id="t1").report
    jobs = w.storage.jobs
    again = r.run(annex_req(deadline="2030-01-01T00:05:00Z"), tenant_id="t1")
    assert again.report == first and w.storage.jobs == jobs
    no_deadline = r.run(annex_req(deadline=None), tenant_id="t1")
    assert no_deadline.report == first and w.storage.jobs == jobs


def test_a_deadline_in_the_past_is_format_checked_only(pg) -> None:  # type: ignore[no-untyped-def]
    """Annex D.4 gives arms no deadline-expiry error: a well-formed past deadline is accepted (ADR 0011)."""
    w = World(pg)
    rep = runner(w).run(annex_req(deadline="2000-01-01T00:00:00Z"), tenant_id="t1").report or {}
    assert rep["status"] == "completed", rep


def test_omitting_agent_id_and_sending_the_derived_one_are_the_same_request(pg) -> None:  # type: ignore[no-untyped-def]
    w = World(pg)
    r = runner(w)
    first = r.run(annex_req(), tenant_id="t1").report  # no agent_id: derived from the target
    jobs = w.storage.jobs
    again = r.run(annex_req(agent_id=AGENT), tenant_id="t1")  # same key, derived value spelled out
    assert again.report == first and w.storage.jobs == jobs


def test_a_request_carrying_agent_id_is_replayed_by_one_without_it(pg) -> None:  # type: ignore[no-untyped-def]
    w = World(pg)
    r = runner(w)
    first = r.run(annex_req(agent_id=AGENT), tenant_id="t1").report
    jobs = w.storage.jobs
    again = r.run(annex_req(), tenant_id="t1")
    assert again.report == first and w.storage.jobs == jobs


# -- the Idempotency-Key header (route level) -------------------------------------------------------------------
class StubArms:
    def __init__(self) -> None:
        self.seen: list[dict[str, Any]] = []

    def run(self, raw: dict[str, Any], *, tenant_id: str) -> Any:
        self.seen.append(raw)
        raise ArmDenied("stop_here", 418)


def _client(arms: StubArms) -> TestClient:
    from annexd.test_service_jwt_claims import _bearer, _token  # noqa: F401
    from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey

    from pulso_core_runtime.evaluation.routes import EvaluationDeps, build_handlers
    from pulso_core_runtime.internal.app import build_internal_app
    from pulso_core_runtime.internal.auth import InMemoryJtiStore, ServiceJwtVerifier, ServiceKey

    import annexd.test_service_jwt_claims as m
    deps = EvaluationDeps(runtime=None, arms=arms, broker=None, budgets=None)  # type: ignore[arg-type]
    keys = {"cp1": ServiceKey("control-api", "core-bridge", m.CP.public_key())}
    del Ed25519PrivateKey
    app = build_internal_app(ServiceJwtVerifier(keys, InMemoryJtiStore()), version_info=lambda: {},
                             handlers=build_handlers(deps))
    return TestClient(app, raise_server_exceptions=False)


def _post(c: TestClient, body: dict[str, Any], key: str | None) -> Any:
    from annexd.test_service_jwt_claims import _bearer, _token
    h = _bearer(_token("evaluation_arm_run"))
    if key is not None:
        h["Idempotency-Key"] = key
    return c.post("/evaluation/arms/run", json=body, headers=h)


def test_header_alone_is_the_key() -> None:
    arms = StubArms()
    _post(_client(arms), {"case_ref": "c"}, "hdr-1")
    assert arms.seen == [{"case_ref": "c", "idempotency_key": "hdr-1"}]


def test_body_alone_still_works_for_back_compat() -> None:
    arms = StubArms()
    _post(_client(arms), {"idempotency_key": "body-1"}, None)
    assert arms.seen == [{"idempotency_key": "body-1"}]


def test_header_and_body_must_agree() -> None:
    arms = StubArms()
    c = _client(arms)
    ok = _post(c, {"idempotency_key": "same"}, "same")
    assert ok.status_code == 418 and len(arms.seen) == 1
    bad = _post(c, {"idempotency_key": "one"}, "two")
    assert bad.status_code == 422 and bad.json()["code"] == "pulso:invalid_request"
    assert bad.json()["details"] == {"fields": ["Idempotency-Key"]} and len(arms.seen) == 1


def test_an_invalid_header_is_422() -> None:
    arms = StubArms()
    r = _post(_client(arms), {"case_ref": "c"}, "has space")
    assert r.status_code == 422 and arms.seen == []
