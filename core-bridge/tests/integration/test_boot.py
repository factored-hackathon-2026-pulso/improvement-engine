"""First RED of the integration package: the composed app boots with all seven factories, real L3b tools, the
evaluation routes registered in the right order, and truthful `doubles[]`."""

from __future__ import annotations

import pytest

from integration.conftest import Composed

pytestmark = [pytest.mark.integration, pytest.mark.pg]


def test_composed_app_boots_with_seven_factories_and_real_tools(composed: Composed) -> None:
    from pulso_core_runtime.factories import FACTORY_NAMES, PulsoAuthz
    from pulso_core_runtime.tools.dispatcher import PulsoToolDispatcher

    assert len(FACTORY_NAMES) == 7 and composed.exit_code == 0
    assert type(composed.ports.tools) is PulsoToolDispatcher  # not the old stand-in
    assert isinstance(composed.ports.authz, PulsoAuthz)
    assert composed.client.get("/readyz").json() == {"status": "ready"}


def test_version_reports_only_true_doubles(composed: Composed) -> None:
    body = composed.client.get("/internal/v1/version", headers=composed.headers("version_probe")).json()
    assert not any(d.startswith("tools:") for d in body["doubles"])
    for expected in ("transcript:", "grant-active:"):
        assert any(d.startswith(expected) for d in body["doubles"]), body["doubles"]
    # the bank client and the artifact port are real HTTP clients now (tested against the loopback broker)
    assert not any(d.startswith(("evaluation-sandbox:", "arm-artifact-port:")) for d in body["doubles"])
    assert body["runtime_profile"] == "agent_core_real" and body["pulso_sha"] == "integ"


def test_arms_static_routes_are_registered_before_arm_id(composed: Composed) -> None:
    from pulso_core_runtime.internal.app import ROUTES

    paths = [(r.method, r.path) for r in ROUTES]
    assert paths.index(("POST", "/evaluation/arms/run")) < paths.index(("POST", "/evaluation/arms/{arm_id}/run"))
    assert paths.index(("GET", "/evaluation/arms/by-key/{key}")) < paths.index(("GET", "/evaluation/arms/{arm_id}"))
    r = composed.client.get("/internal/v1/evaluation/arms/by-key/nope",
                            headers=composed.headers("evaluation_arm_read"))
    assert r.status_code == 404 and r.json()["code"] == "pulso:not_found"  # handled, not 501


def test_startup_applied_bridge_and_eval_migrations(composed: Composed) -> None:
    import psycopg
    with psycopg.connect(composed.pg.runtime) as conn:
        names = {r[0] for r in conn.execute(
            "SELECT table_name FROM information_schema.tables WHERE table_schema='pulso_bridge'")}
    assert {"jti_seen", "eval_admissions", "arm_executions", "invocation_contexts"} <= names, names
