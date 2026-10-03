"""Local-only guards: profile, kid prefixes, key separation, remote indicators, contract scan."""

from __future__ import annotations

import io
import json
from pathlib import Path
from typing import Any

import pytest
from conftest import Env, env_for

from local_identity.config import ConfigError, load_config
from local_identity.contract import scan_remote_config
from local_identity.keys import b64url_decode, b64url_encode, generate_keyset, pubkey_of_seed
from local_identity.main import run


def test_refuses_nonlocal_profile(env: Env) -> None:
    for bad in ("staging", "prod", "", "Local"):
        with pytest.raises(ConfigError) as err:
            load_config(env_for(env.dir, LOCAL_IDENTITY_PROFILE=bad))
        assert err.value.code == "local_identity:profile_not_local"
    with pytest.raises(ConfigError):
        load_config({k: v for k, v in env_for(env.dir).items() if k != "LOCAL_IDENTITY_PROFILE"})


@pytest.mark.parametrize(
    "marker",
    [
        {"AWS_EXECUTION_ENV": "AWS_ECS_FARGATE"},
        {"ECS_CONTAINER_METADATA_URI_V4": "http://169.254.170.2/v4/x"},
        {"PULSO_ENV": "staging"},
        {"PULSO_ENV": "production"},
    ],
)
def test_refuses_remote_runtime_indicators(env: Env, marker: dict[str, str]) -> None:
    with pytest.raises(ConfigError) as err:
        load_config(env_for(env.dir, **marker))
    assert err.value.code == "local_identity:remote_environment"


def test_human_kid_must_be_a_local_test_kid(env: Env) -> None:
    path = env.dir / "human-staff-signer.json"
    data = json.loads(path.read_text())
    data["kid"] = "staff-2026-01"
    path.write_text(json.dumps(data))
    with pytest.raises(ConfigError) as err:
        load_config(env_for(env.dir))
    assert err.value.code == "local_identity:kid_not_local"


def test_human_staff_key_must_differ_from_session_and_bot_keys(env: Env, tmp_path: Path) -> None:
    session = json.loads((env.dir / "session-signer.json").read_text())
    human = json.loads((env.dir / "human-staff-signer.json").read_text())
    (env.dir / "human-staff-signer.json").write_text(json.dumps({"kid": human["kid"], "key": session["key"]}))
    with pytest.raises(ConfigError) as err:
        load_config(env_for(env.dir))
    assert err.value.code == "local_identity:key_reuse"
    (env.dir / "human-staff-signer.json").write_text(json.dumps(human))
    bot = tmp_path / "bot-keys.json"
    bot.write_text(json.dumps({"keys": {"st1": b64url_encode(pubkey_of_seed(b64url_decode(human["key"])))}}))
    with pytest.raises(ConfigError) as err2:
        load_config(env_for(env.dir, LOCAL_IDENTITY_BOT_PUBLIC_KEYS=str(bot)))
    assert err2.value.code == "local_identity:key_reuse"


@pytest.mark.parametrize(
    "bad",
    [
        {"pulso-constructor:t": {"tenant_id": "t", "roles": ["constructor"]}},
        {"h": {"tenant_id": "t", "roles": ["root"]}},
        {"h": {"tenant_id": "t", "roles": []}},
        {"h": {"tenant_id": "t", "roles": ["constructor"], "actor": "bot"}},
    ],
)
def test_identity_allowlist_rejects_bot_shaped_and_unknown_roles(env: Env, bad: dict[str, Any]) -> None:
    (env.dir / "identities.json").write_text(json.dumps({"identities": bad}))
    with pytest.raises(ConfigError) as err:
        load_config(env_for(env.dir))
    assert err.value.code == "local_identity:identities_invalid"


def test_service_keys_must_be_bound_to_human_issuer_audience(env: Env) -> None:
    path = env.dir / "human-issuer-service-keys.json"
    data = json.loads(path.read_text())
    for entry in data["keys"].values():
        entry["aud"] = "core-bridge"
    path.write_text(json.dumps(data))
    with pytest.raises(ConfigError) as err:
        load_config(env_for(env.dir))
    assert err.value.code == "local_identity:service_keys_invalid"


def test_main_exit_codes(env: Env) -> None:
    served: list[dict[str, Any]] = []
    assert run(env=env_for(env.dir), serve=lambda app, **kw: served.append(kw), stderr=io.StringIO()) == 0
    assert served and served[0]["port"] == 8083
    err = io.StringIO()
    assert run(env=env_for(env.dir, LOCAL_IDENTITY_PROFILE="prod"), serve=lambda *a, **k: None, stderr=err) == 2
    assert "local_identity:profile_not_local" in err.getvalue()


def test_secret_never_in_error_text(env: Env) -> None:
    seed = json.loads((env.dir / "human-staff-signer.json").read_text())["key"]
    (env.dir / "human-staff-signer.json").write_text('{"kid": "local-sim-human-1", "key": "' + seed[:-2] + '"}')
    with pytest.raises(ConfigError) as err:
        load_config(env_for(env.dir))
    assert seed[:-2] not in str(err.value)


# --- remote-config contract scan (CAP-63) -------------------------------------------------------------------


def test_scan_flags_test_kid_and_simulated_flag_in_remote_config(tmp_path: Path) -> None:
    (tmp_path / "terraform").mkdir()
    (tmp_path / "terraform" / "staging.tfvars").write_text('staff_kids = ["local-sim-human-1"]\n')
    (tmp_path / "infra").mkdir()
    (tmp_path / "infra" / "ecs-task.json").write_text('{"env": [{"name": "X", "value": "auth.simulated=true"}]}')
    (tmp_path / "deploy-prod.yaml").write_text('principal: {"auth": {"simulated": true}}\n')
    findings = scan_remote_config(tmp_path)
    assert {f.path.name for f in findings} == {"staging.tfvars", "ecs-task.json", "deploy-prod.yaml"}


def test_scan_ignores_local_files_and_clean_remote_config(tmp_path: Path) -> None:
    (tmp_path / "local").mkdir()
    (tmp_path / "local" / "compose.yaml").write_text("kid: local-sim-human-1\n")
    (tmp_path / "terraform").mkdir()
    (tmp_path / "terraform" / "main.tf").write_text('resource "x" "y" {}\n')
    assert scan_remote_config(tmp_path) == []


def test_repository_remote_config_is_clean() -> None:
    assert scan_remote_config(Path(__file__).resolve().parents[2]) == []


def test_keyset_files_are_distinct_and_fragment_public_only(tmp_path: Path) -> None:
    keys = generate_keyset(tmp_path / "k", tenant_id="t1")
    seeds = [
        json.loads((keys.dir / n).read_text())["key"]
        for n in ("control-api-signer.json", "session-signer.json", "human-staff-signer.json")
    ]
    assert len(set(seeds)) == 3
    frag = (keys.dir / "core-staff-keys.human-fragment.json").read_text()
    assert list(json.loads(frag)["principal_keys"]) == [keys.human_kid]
    assert keys.human_kid.startswith("local-sim-human-")
    assert not any(seed in frag for seed in seeds)
