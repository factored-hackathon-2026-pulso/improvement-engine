"""Adversarial review round: guard bypasses, parsing robustness, and the Core-ignores-attrs binding contract."""

from __future__ import annotations

import json
import uuid
from pathlib import Path
from typing import Any

import pytest
from conftest import ADMIN, SUPERVISOR, TENANT, Env, env_for, proposal_target, release_target

from local_identity.client import BindingMismatch, assert_bound
from local_identity.config import ConfigError, load_config

CMD_URL = "/internal/v1/human/command-authorizations/issue"
ROOT = Path(__file__).resolve().parents[1]


@pytest.mark.parametrize(
    "marker",
    [
        {"KUBERNETES_SERVICE_HOST": "10.0.0.1"},
        {"AWS_LAMBDA_FUNCTION_NAME": "f"},
        {"AWS_CONTAINER_CREDENTIALS_RELATIVE_URI": "/v2/credentials/x"},
        {"AWS_CONTAINER_CREDENTIALS_FULL_URI": "http://x"},
        {"K_SERVICE": "svc"},
        {"WEBSITE_SITE_NAME": "app"},
        {"PULSO_ENV": " Production "},
        {"PULSO_ENV": "qa"},  # allowlist: only local/dev/test-like values may run the sandbox issuer
        {"PULSO_ENV": "preprod"},
    ],
)
def test_more_remote_indicators_and_pulso_env_allowlist(env: Env, marker: dict[str, str]) -> None:
    with pytest.raises(ConfigError) as err:
        load_config(env_for(env.dir, **marker))
    assert err.value.code == "local_identity:remote_environment"


@pytest.mark.parametrize("value", ["", "local", "dev", "development", "test", " Local "])
def test_pulso_env_local_values_allowed(env: Env, value: str) -> None:
    load_config(env_for(env.dir, PULSO_ENV=value))


def test_default_bind_is_loopback_and_image_does_not_bake_the_profile(env: Env) -> None:
    assert load_config(env_for(env.dir)).host == "127.0.0.1"
    dockerfile = (ROOT / "Dockerfile").read_text()
    # An image that sets the profile itself would make the profile guard meaningless wherever it is deployed.
    assert "LOCAL_IDENTITY_PROFILE" not in dockerfile
    assert "LOCAL_IDENTITY_HOST=0.0.0.0" in dockerfile
    assert "LOCAL_IDENTITY_PROFILE: local" in (ROOT / "compose.fragment.yaml").read_text()


def test_oversized_body_is_rejected_before_parsing(env: Env) -> None:
    h = {"Authorization": f"Bearer {env.client.mint('command_authorization')}"}
    r = env.http.post(CMD_URL, headers=h, content=b"{" + b" " * (70 * 1024) + b"}")
    assert r.status_code == 413 and r.json()["code"] == "pulso:payload_too_large"


def test_huge_numeric_claims_are_401_not_500(env: Env) -> None:
    now_ts = int(env.clock.now.timestamp())
    tok = env.client.signer.sign_claims(
        {
            "iss": "control-api", "aud": "human-issuer", "sub": "w", "tenant_id": TENANT,
            "purpose": "command_authorization", "iat": now_ts, "exp": 10**400, "jti": uuid.uuid4().hex,
        }
    )  # fmt: skip
    r = env.http.post(CMD_URL, headers={"Authorization": f"Bearer {tok}"}, json={})
    assert r.status_code == 401


def test_trace_id_is_only_echoed_when_hex(env: Env) -> None:
    bad = "-".join(["00", 'x"<script>' + "a" * 22, "b" * 16, "01"])
    assert len(bad.split("-")[1]) == 32
    r = env.http.post(CMD_URL, headers={"traceparent": bad}, json={})
    assert "script" not in r.text
    good = "-".join(["00", "ab" * 16, "b" * 16, "01"])
    assert env.http.post(CMD_URL, headers={"traceparent": good}, json={}).json()["trace_id"] == "ab" * 16


def test_digest_not_ambiguous_across_fields(env: Env) -> None:
    """Fields are regex-restricted (no quotes/newlines) so moving content between fields cannot collide."""
    a = env.client.command_authorization(**_cmd(command_ref="c1", challenge_ref="x:y"))
    b = env.client.command_authorization(**_cmd(command_ref="c1:x", challenge_ref="y"))
    assert a.metadata["binding_digest"] != b.metadata["binding_digest"]
    for bad in ('a"b', "a\nb", "a b", "aаb", "a\n"):
        r = env.http.post(
            CMD_URL,
            headers={"Authorization": f"Bearer {env.client.mint('command_authorization')}"},
            json=_cmd(command_ref=bad),
        )
        assert r.status_code == 422, bad


def _cmd(**over: Any) -> dict[str, Any]:
    args: dict[str, Any] = {
        "tenant_id": TENANT, "actor_ref": SUPERVISOR, "command_ref": "cmd-1", "operation": "approve",
        "target": proposal_target(), "challenge_ref": "chal-1", "nonce": uuid.uuid4().hex,
    }  # fmt: skip
    args.update(over)
    return args


def _bound(args: dict[str, Any]) -> dict[str, Any]:
    return {k: v for k, v in args.items() if k != "nonce"}


# --- HumanAuthorizationPort contract: Core ignores binding attrs, so enforcement lives in the port -------------


def test_assert_bound_is_malformed_not_crash_on_hostile_payloads(env: Env) -> None:
    import base64

    args = _cmd()
    for payload in (b'{"attrs": []}', b'{"attrs": "x"}', b"[]", b"null", b'{"id": 1}'):
        body = base64.urlsafe_b64encode(payload).rstrip(b"=").decode()
        with pytest.raises(BindingMismatch):
            assert_bound(f"e30.{body}.sig", **_bound(args))
    with pytest.raises(BindingMismatch):
        assert_bound("not-a-jws", **_bound(args))


def test_assert_bound_checks_roles_step_up_and_expiry_when_asked(env: Env) -> None:
    args = _cmd(operation="revoke", actor_ref=ADMIN, target=release_target())
    out = env.client.command_authorization(**args)
    assert_bound(out.authorization_jws, **_bound(args), now=env.clock.now)
    with pytest.raises(BindingMismatch):  # expired credential
        assert_bound(out.authorization_jws, **_bound(args), now=env.clock.now.replace(year=2027))
    # an approve credential must not satisfy a revoke intention even if the caller lies about the operation
    ap = _cmd()
    approval = env.client.command_authorization(**ap)
    with pytest.raises(BindingMismatch):
        assert_bound(approval.authorization_jws, **_bound({**ap, "operation": "revoke", "target": release_target()}))


def test_tampered_individual_attrs_detected(env: Env) -> None:
    import base64

    args = _cmd()
    out = env.client.command_authorization(**args)
    h, p, s = out.authorization_jws.split(".")
    payload = json.loads(base64.urlsafe_b64decode(p + "=" * (-len(p) % 4)))
    payload["attrs"]["expected_revision"] = "999"  # digest untouched; Core would never notice
    forged = base64.urlsafe_b64encode(json.dumps(payload).encode()).rstrip(b"=").decode()
    with pytest.raises(BindingMismatch):
        assert_bound(f"{h}.{forged}.{s}", **_bound(args))
