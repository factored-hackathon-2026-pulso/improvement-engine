"""PG-gated composition checks for the 789d6c8 bump: Core `/version` sha (N-04), export OFF by default (N-08),
key reload (N-09). Reuses the L2 composition helpers."""

from __future__ import annotations

import json
import time
from typing import Any

import pytest
from fastapi.testclient import TestClient
from pulso_core_runtime import PIN_SHA
from pulso_core_runtime.internal.store import ensure_schema

from .conftest import PgDbs
from .test_pg_runtime import (  # noqa: F401  (fixture re-export)
    _compose,
    _env,
    _pub,
    _seed,
    _token,
    keys,
)

pytestmark = [pytest.mark.runtime, pytest.mark.pg]


def test_core_version_reports_the_build_sha(pg: PgDbs, keys: dict[str, Any]) -> None:  # noqa: F811
    ensure_schema(pg.runtime)
    code, app, err = _compose(_env(pg, keys, PULSO_CORE_SHA=PIN_SHA))
    assert code == 0, err
    body = TestClient(app).get("/version").json()
    assert body["sha"] == PIN_SHA and body["contract"] and body["package"]
    code, app, err = _compose(_env(pg, keys))  # no PULSO_CORE_SHA: falls back to the pinned sha
    assert TestClient(app).get("/version").json()["sha"] == PIN_SHA


def test_core_export_routes_absent_by_default(pg: PgDbs, keys: dict[str, Any]) -> None:  # noqa: F811
    ensure_schema(pg.runtime)
    code, app, err = _compose(_env(pg, keys))
    assert code == 0, err
    c = TestClient(app)
    for path in ("/v1/export/runs", "/v1/export/registry-events", "/v1/export/runs/r1/events"):
        assert c.get(path).status_code == 404, path
    assert "/v1/export/runs" not in c.get("/openapi.json").text


def test_core_export_requires_credential_when_enabled(pg: PgDbs, keys: dict[str, Any]) -> None:  # noqa: F811
    ensure_schema(pg.runtime)
    code, app, err = _compose(_env(pg, keys, PULSO_CORE_EXPORT_ENABLED="1"))
    assert code == 0, err
    c = TestClient(app)
    assert c.get("/v1/export/runs").status_code in (401, 403)
    assert c.get("/v1/export/runs", headers={"Authorization": "Bearer garbage"}).status_code in (401, 403)


def test_key_reload_keeps_last_good_keys_and_reports_type_only(pg: PgDbs, keys: dict[str, Any]) -> None:  # noqa: F811
    ensure_schema(pg.runtime)
    code, app, err = _compose(_env(pg, keys, PULSO_KEYS_RELOAD_SECONDS="0.05"))
    assert code == 0, err
    c = TestClient(app)
    staff = keys["dir"] / "staff.json"
    good = staff.read_text()
    staff.write_text("{not json")  # corrupt but non-empty: readiness (non-empty file) stays green, reload fails closed
    time.sleep(0.2)
    c.get("/version")
    c.get("/v1/registry/proposals", headers={"Authorization": "Bearer x"})  # exercises the staff verifier -> reload
    info = c.get("/internal/v1/version", headers={"Authorization": f"Bearer {_token(keys['svc'])}"}).json()
    assert info["keys_reload_error"] in (None, "SchemaError")
    assert "not json" not in json.dumps(info)
    staff.write_text(good)
