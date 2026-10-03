"""core-keygen: the bridge needs a SEPARATE executor keypair (A03 iii) and the lab-broker double trusts its public half."""

from __future__ import annotations

import base64
import json
import os
import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "init"))

import gen_keys  # noqa: E402


@pytest.fixture()
def keys(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> Path:
    monkeypatch.setattr(os, "chown", lambda *a, **k: None, raising=False)
    monkeypatch.setenv("PULSO_EXPORTER_STATE_DIR", str(tmp_path / "nostate"))
    out = tmp_path / "keys"
    assert gen_keys.main(out) == 0
    return out


def _load(path: Path) -> dict:
    return json.loads(path.read_text(encoding="ascii"))


def test_emits_executor_signer_with_distinct_kid_and_seed(keys: Path) -> None:
    ex, cb = _load(keys / "bridge-executor.json"), _load(keys / "bridge-callback.json")
    assert set(ex) == {"kid", "key"}
    assert ex["kid"] != cb["kid"] and ex["key"] != cb["key"]
    assert len(base64.urlsafe_b64decode(ex["key"] + "=" * (-len(ex["key"]) % 4))) == 32


def test_lab_broker_trust_holds_only_the_executor_public_key(keys: Path) -> None:
    from cryptography.hazmat.primitives import serialization
    from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey

    ex = _load(keys / "bridge-executor.json")
    trust = _load(keys / "lab-broker-trust.json")
    assert list(trust["keys"]) == [ex["kid"]]
    entry = trust["keys"][ex["kid"]]
    assert (entry["iss"], entry["aud"]) == ("core-bridge", "lab-broker")
    seed = base64.urlsafe_b64decode(ex["key"] + "=" * (-len(ex["key"]) % 4))
    pub = Ed25519PrivateKey.from_private_bytes(seed).public_key().public_bytes(
        serialization.Encoding.Raw, serialization.PublicFormat.Raw)
    assert entry["key"] == base64.urlsafe_b64encode(pub).rstrip(b"=").decode()
    assert ex["key"] not in (keys / "lab-broker-trust.json").read_text(encoding="ascii")


def test_executor_key_is_not_a_delegation_key(keys: Path) -> None:
    ex = _load(keys / "bridge-executor.json")
    ident = _load(keys / "identity.json")
    assert ex["kid"] not in ident["delegation_keys"] and ex["kid"] not in ident["principal_keys"]


def test_second_run_changes_nothing(keys: Path) -> None:
    before = {p.name: p.read_bytes() for p in keys.iterdir()}
    assert gen_keys.main(keys) == 0
    assert {p.name: p.read_bytes() for p in keys.iterdir()} == before


def test_exporter_keys_are_registered_with_the_issuer_the_exporter_signs(keys: Path) -> None:
    from pulso_core_runtime.exporter.auth import ISSUER  # the exporter signs iss=core-bridge (A03)

    service = _load(keys / "service.json")["keys"]
    for aud in ("control-api", "lab-broker"):
        assert service[f"exporter-{aud}"]["iss"] == ISSUER
        assert service[f"exporter-{aud}"]["aud"] == aud


def test_control_api_to_core_bridge_key_is_emitted_with_a_readable_seed(keys: Path) -> None:
    entry = _load(keys / "service.json")["keys"]["control-api-core-bridge"]
    assert (entry["iss"], entry["aud"]) == ("control-api", "core-bridge")
    seed = (keys / "control-api-core-bridge.key").read_text(encoding="ascii").strip()
    from cryptography.hazmat.primitives import serialization
    from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey

    pub = Ed25519PrivateKey.from_private_bytes(base64.urlsafe_b64decode(seed + "=" * (-len(seed) % 4))).public_key()
    raw = pub.public_bytes(serialization.Encoding.Raw, serialization.PublicFormat.Raw)
    assert entry["key"] == base64.urlsafe_b64encode(raw).rstrip(b"=").decode()


def test_control_api_trust_holds_only_the_callback_public_key(keys: Path) -> None:
    cb = _load(keys / "bridge-callback.json")
    trust = _load(keys / "control-api-trust.json")["keys"]
    assert list(trust) == [cb["kid"]]
    assert (trust[cb["kid"]]["iss"], trust[cb["kid"]]["aud"]) == ("core-bridge", "control-api")
    assert cb["key"] not in (keys / "control-api-trust.json").read_text(encoding="ascii")


# --- local human issuer public key (plan 17.3.2): merged into the LOCAL Core staff verifier set only -----------------
HUMAN_KID = "local-sim-human-1"


def _fragment(tmp_path: Path, kid: str = HUMAN_KID) -> Path:
    from cryptography.hazmat.primitives import serialization
    from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey

    pub = Ed25519PrivateKey.generate().public_key().public_bytes(serialization.Encoding.Raw, serialization.PublicFormat.Raw)
    path = tmp_path / "fragment.json"
    path.write_text(json.dumps({"principal_keys": {kid: base64.urlsafe_b64encode(pub).rstrip(b"=").decode()}}), encoding="ascii")
    return path


def test_staff_set_has_no_human_key_without_a_fragment(keys: Path) -> None:
    assert not any(k.startswith("local-sim-") for k in _load(keys / "staff.json")["principal_keys"])


def test_human_fragment_is_merged_next_to_the_bridge_staff_key(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(os, "chown", lambda *a, **k: None, raising=False)
    monkeypatch.setenv("PULSO_EXPORTER_STATE_DIR", str(tmp_path / "nostate"))
    frag = _fragment(tmp_path)
    monkeypatch.setenv("PULSO_HUMAN_STAFF_FRAGMENT", str(frag))
    out = tmp_path / "keys"
    assert gen_keys.main(out) == 0
    staff = _load(out / "staff.json")["principal_keys"]
    assert set(staff) == {"bridge-staff-local", HUMAN_KID}  # bot key and human key stay distinct entries
    assert staff[HUMAN_KID] == _load(frag)["principal_keys"][HUMAN_KID]
    assert staff["bridge-staff-local"] != staff[HUMAN_KID]
    assert HUMAN_KID not in _load(out / "identity.json")["principal_keys"]  # run principals never trust the human key


def test_merge_is_idempotent_and_also_applies_to_an_already_generated_set(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(os, "chown", lambda *a, **k: None, raising=False)
    monkeypatch.setenv("PULSO_EXPORTER_STATE_DIR", str(tmp_path / "nostate"))
    out = tmp_path / "keys"
    assert gen_keys.main(out) == 0  # set generated without the human key
    monkeypatch.setenv("PULSO_HUMAN_STAFF_FRAGMENT", str(_fragment(tmp_path)))
    assert gen_keys.main(out) == 0
    first = (out / "staff.json").read_text(encoding="ascii")
    assert HUMAN_KID in json.loads(first)["principal_keys"]
    assert gen_keys.main(out) == 0
    assert (out / "staff.json").read_text(encoding="ascii") == first


@pytest.mark.parametrize("kid", ["bridge-staff-local", "prod-human-1", ""])
def test_a_fragment_kid_that_is_not_a_local_sim_human_kid_is_refused(tmp_path: Path, monkeypatch: pytest.MonkeyPatch, kid: str) -> None:
    monkeypatch.setattr(os, "chown", lambda *a, **k: None, raising=False)
    monkeypatch.setenv("PULSO_EXPORTER_STATE_DIR", str(tmp_path / "nostate"))
    monkeypatch.setenv("PULSO_HUMAN_STAFF_FRAGMENT", str(_fragment(tmp_path, kid)))
    assert gen_keys.main(tmp_path / "keys") == 2
