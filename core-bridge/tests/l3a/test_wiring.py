"""`build_l3` composes handlers from signer files and applies the bridge migrations; errors name files only."""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any

import pytest
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey

from pulso_core_runtime.internal.auth import b64url_encode
from pulso_core_runtime.invoke.wiring import build_l3
from pulso_core_runtime.store.migrations import l3_ready

from .conftest import PgDbs

pytestmark = [pytest.mark.l3a, pytest.mark.pg]


def _signer_file(path: Path, kid: str) -> None:
    path.write_text(json.dumps({"kid": kid, "key": b64url_encode(Ed25519PrivateKey.generate().private_bytes_raw())}))


def test_build_l3_registers_the_three_routes_and_migrates(pg: PgDbs, tmp_path: Path) -> None:
    from pulso_core_runtime.internal.store import ensure_schema
    ensure_schema(pg.runtime)
    for name, kid in (("i", "id1"), ("s", "st1"), ("c", "cb1")):
        _signer_file(tmp_path / f"{name}.json", kid)
    env = {"PULSO_BRIDGE_IDENTITY_SIGNER": str(tmp_path / "i.json"), "PULSO_BRIDGE_STAFF_SIGNER": str(tmp_path / "s.json"),
           "PULSO_BRIDGE_CALLBACK_SIGNER": str(tmp_path / "c.json"), "PULSO_CONTROL_API_URL": "http://control.test"}
    l3 = build_l3(env, dsn=pg.runtime, registry=object(), app_getter=lambda: None)
    assert set(l3.handlers) == {"POST /core-tasks/invoke", "GET /core-tasks/{task_id}", "POST /core-credentials/issue"}
    assert l3_ready(pg.runtime)


def test_missing_signer_file_error_names_file_not_content(pg: PgDbs, tmp_path: Path) -> None:
    env: dict[str, Any] = {"PULSO_BRIDGE_IDENTITY_SIGNER": str(tmp_path / "absent.json")}
    with pytest.raises(ValueError, match="absent.json"):
        build_l3(env, dsn=pg.runtime, registry=object(), app_getter=lambda: None, migrate=False)
