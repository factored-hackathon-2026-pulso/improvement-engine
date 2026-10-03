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
