"""Composed app (real `main._compose`, real Core registry on Postgres 16): alias read and authoring dry-run
through `/internal/v1`, with the dry-run `candidate_hash` proven equal to the real `freeze` (CC-08).
Doubles: the pin checkout's FakeEvaluator/FakeClock/FakeIds only to seed releases (documented in test_pg_runtime)."""

from __future__ import annotations

import time
import uuid
from typing import Any

import psycopg
import pytest
from fastapi.testclient import TestClient

from pulso_core_runtime.internal.auth import sign_service_jwt
from pulso_core_runtime.internal.store import ensure_schema

from .conftest import PgDbs
from .test_pg_runtime import (  # noqa: F401  (fixture re-export)
    _compose,
    _env,
    _seed_two_releases,
    keys,
)

pytestmark = [pytest.mark.runtime, pytest.mark.pg]


def _tok(keys: dict[str, Any], purpose: str, tenant: str = "t1") -> dict[str, str]:
    now = int(time.time())
    jws = sign_service_jwt(keys["svc"], kid="cp1", claims={
        "iss": "control-api", "aud": "core-bridge", "sub": "worker:1", "tenant_id": tenant, "purpose": purpose,
        "job_id": "j1", "iat": now, "exp": now + 60, "jti": uuid.uuid4().hex})
    return {"Authorization": f"Bearer {jws}"}


def _tables(dsn: str) -> dict[str, int]:
    with psycopg.connect(dsn) as conn:
        names = [r[0] for r in conn.execute(
            "select table_name from information_schema.tables where table_schema='public' "
            "and table_name like 'reg%' order by 1")]
        return {n: conn.execute(f'select count(*) from "{n}"').fetchone()[0] for n in names}  # type: ignore[index]


def _draft(version: str) -> dict[str, Any]:
    from tests.registry.helpers import prompt_draft

    return prompt_draft(version=version).model_dump(mode="json")


def test_alias_and_dry_run_through_composed_app_match_core(pg: PgDbs, keys: dict[str, Any]) -> None:  # noqa: F811
    ensure_schema(pg.runtime)
    _, r1, r2, (service, _clock), (agent, _admin) = _seed_two_releases(pg.runtime)
    code, app, err = _compose(_env(pg, keys))
    assert code == 0, err
    c = TestClient(app)

    # alias read: staging moved to r2 by the second publish; prod stayed on the seed release
    got = c.get(f"/internal/v1/core-state/aliases/{agent}/staging", headers=_tok(keys, "alias_read"))
    assert got.status_code == 200, got.text
    assert got.json()["release_id"] == r2 and got.json()["status"] == "active" and got.json()["source"] == "core_store"
    prod = c.get(f"/internal/v1/core-state/aliases/{agent}/prod", headers=_tok(keys, "alias_read"))
    assert prod.status_code == 200 and prod.json()["release_id"] not in (r1, r2)
    unknown = c.get("/internal/v1/core-state/aliases/no-such-agent/prod", headers=_tok(keys, "alias_read"))
    assert unknown.status_code == 404 and unknown.json()["code"] == "pulso:alias_unknown"
    # tenant scoping: a token for another tenant never reaches the handler
    other = c.get(f"/internal/v1/core-state/aliases/{agent}/staging", headers=_tok(keys, "alias_read", "t2"))
    assert other.status_code == 403 and other.json()["code"] == "pulso:tenant_mismatch"

    before = _tables(pg.runtime)
    body = {"schema_version": "1", "tenant_id": "t1", "agent_id": agent, "base_release_id": r2,
            "changes": [_draft("1.3.0")]}
    r = c.post("/internal/v1/core-authoring/dry-run", json=body, headers=_tok(keys, "authoring_dry_run"))
    assert r.status_code == 200, r.text
    out = r.json()
    assert out["valid"] is True and out["proposal_created"] is False and out["violations"] == []
    again = c.post("/internal/v1/core-authoring/dry-run", json=body, headers=_tok(keys, "authoring_dry_run")).json()
    assert again == out  # deterministic and idempotent
    assert _tables(pg.runtime) == before  # no proposal, version, event, release or alias write; no quota used

    # CC-08: the REAL freeze of the same draft yields the same candidate hash / release id preview.
    from agent_core.registry.models import EntityDraft, Origin
    from tests.registry.helpers import human

    ana = human()
    p = service.create_proposal(ana, agent, Origin.manual, "t")
    service.put_draft(ana, p.proposal_id, [EntityDraft.model_validate(_draft("1.3.0"))], expected_rev=0)
    view = service.freeze(ana, p.proposal_id)
    assert view.candidate_hash == out["candidate_hash"] and view.release_id_preview == out["release_id_preview"]
    assert out["auto_bumped"] == [v.model_dump(mode="json") for v in view.auto_bumped]

    bad = c.post("/internal/v1/core-authoring/dry-run", json={**body, "changes": [_draft("0.0.1")]},
                 headers=_tok(keys, "authoring_dry_run"))
    assert bad.status_code == 200 and bad.json()["valid"] is False
    assert bad.json()["violations"][0]["rule"] == "REG-VERSION"
    # the 501 placeholders are gone for both routes
    assert c.get(f"/internal/v1/core-state/aliases/{agent}/staging").status_code == 401
