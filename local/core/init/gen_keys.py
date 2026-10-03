"""core-keygen: generate throw-away Ed25519 test keys into the core-keys volume (never into the repo).

Idempotent: existing key files are kept (a second run changes nothing). Files (names, not values, are documented):
  identity.json / staff.json         public keys the runtime verifies (formats of agent_core.adapters.identity_keys)
  bridge-{identity,staff,callback,executor}.json   the bridge's private signers (credentials.issuer.load_signer format);
                                     the executor key (A03 iii, executor -> lab-broker) is a distinct keypair, never the callback key
  lab-broker-trust.json              PUBLIC half of the executor key, in the fixture-key format of the lab-broker double
  service.json                       public service keys (internal.auth.load_service_keys format)
  exporter-control-api.key / exporter-lab-broker.key   exporter private seeds (b64url, one line)
Runs as root inside the image, then hands ownership to the runtime uid (10001).
"""

from __future__ import annotations

import base64
import json
import os
import sys
from pathlib import Path

from cryptography.hazmat.primitives import serialization
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey

UID = 10001


def b64u(raw: bytes) -> str:
    return base64.urlsafe_b64encode(raw).rstrip(b"=").decode()


def new_pair() -> tuple[str, str]:
    key = Ed25519PrivateKey.generate()
    seed = key.private_bytes(serialization.Encoding.Raw, serialization.PrivateFormat.Raw, serialization.NoEncryption())
    pub = key.public_key().public_bytes(serialization.Encoding.Raw, serialization.PublicFormat.Raw)
    return b64u(seed), b64u(pub)


def write(path: Path, text: str, mode: int = 0o640) -> None:
    path.write_text(text, encoding="ascii")
    os.chmod(path, mode)
    os.chown(path, UID, UID)


def main(out: Path) -> int:
    out.mkdir(parents=True, exist_ok=True)
    marker = out / "service.json"
    if marker.exists():
        print("core-keygen: keys already present, nothing written")
        return 0
    signers: dict[str, tuple[str, str, str]] = {}
    for name in ("bridge-identity", "bridge-staff", "bridge-callback", "bridge-executor"):
        seed, pub = new_pair()
        kid = f"{name}-local"
        signers[name] = (kid, seed, pub)
        write(out / f"{name}.json", json.dumps({"kid": kid, "key": seed}))
    write(out / "identity.json", json.dumps({
        "principal_keys": {signers["bridge-identity"][0]: signers["bridge-identity"][2]},
        "delegation_keys": {signers["bridge-callback"][0]: signers["bridge-callback"][2]}}))
    write(out / "staff.json", json.dumps({"principal_keys": {signers["bridge-staff"][0]: signers["bridge-staff"][2]}}))
    ex_kid, _, ex_pub = signers["bridge-executor"]
    write(out / "lab-broker-trust.json", json.dumps({"keys": {ex_kid: {"iss": "core-bridge", "aud": "lab-broker", "key": ex_pub}}}), 0o644)  # public only
    service: dict[str, dict[str, str]] = {}
    for aud in ("control-api", "lab-broker"):
        seed, pub = new_pair()
        write(out / f"exporter-{aud}.key", seed + "\n")
        service[f"exporter-{aud}"] = {"iss": "pulso-exporter", "aud": aud, "key": pub}
    seed, pub = new_pair()  # smoke probe (core-bridge audience): used by local/core/smoke.ps1 -Exec only
    write(out / "smoke-probe.key", seed + "\n")
    service["smoke-probe"] = {"iss": "pulso-smoke", "aud": "core-bridge", "key": pub}
    write(marker, json.dumps({"keys": service}))  # written last: marks a complete set
    state = Path(os.environ.get("PULSO_EXPORTER_STATE_DIR", "/var/lib/exporter"))
    if state.is_dir():
        os.chown(state, UID, UID)
    print("core-keygen: wrote", len(list(out.iterdir())), "files")
    return 0


if __name__ == "__main__":
    sys.exit(main(Path(os.environ.get("PULSO_KEYS_DIR", "/run/pulso-keys"))))
