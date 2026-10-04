"""Annex D alignment (e)(f): CoreVersion carries schema_version + bridge_instance_id, the credential request is a
closed DTO that tolerates the annex `schema_version`, and invoke timestamps/digests have the D.1 formats."""

from __future__ import annotations

from typing import Any

import pytest
from pydantic import ValidationError

from pulso_core_runtime.invoke.models import CoreTaskInvocation
from pulso_core_runtime.invoke.routes import CredentialRequest
from pulso_core_runtime.main import version_info

pytestmark = pytest.mark.runtime


def test_core_version_has_schema_version_and_bridge_instance_id() -> None:
    info = version_info({})()
    assert info["schema_version"] == "1" and info["bridge_instance_id"] == "bridge-1"
    named = version_info({"PULSO_BRIDGE_INSTANCE": "bridge-7"})()
    assert named["bridge_instance_id"] == "bridge-7"


def test_credential_request_is_closed_and_accepts_the_annex_schema_version() -> None:
    base = {"tenant_id": "t1", "role": "constructor", "purpose": "core_task"}
    assert CredentialRequest.model_validate(base)
    assert CredentialRequest.model_validate({**base, "schema_version": "1"})
    for bad in ({**base, "schema_version": "2"}, {**base, "extra": 1}, {**base, "role": ""},
                {k: v for k, v in base.items() if k != "purpose"}):
        with pytest.raises(ValidationError):
            CredentialRequest.model_validate(bad)


def inv(**over: Any) -> dict[str, Any]:
    from l3a.helpers import body
    return body(**over)


def test_invoke_timestamps_must_be_z_suffixed_rfc3339() -> None:
    assert CoreTaskInvocation.model_validate(inv(deadline="2030-01-01T00:00:00Z", cutoff="2029-12-31T00:00:00Z"))
    for field in ("deadline", "cutoff"):
        for bad in ("2030-01-01T00:00:00+00:00", "2030-01-01T00:00:00", "2030-01-01", "soon"):
            with pytest.raises(ValidationError):
                CoreTaskInvocation.model_validate(inv(**{field: bad}))


def test_invoke_request_digest_is_lowercase_hex_sha256() -> None:
    assert CoreTaskInvocation.model_validate(inv(request_digest="a" * 64))
    for bad in ("A" * 64, "a" * 63, "g" * 64, ""):
        with pytest.raises(ValidationError):
            CoreTaskInvocation.model_validate(inv(request_digest=bad))
