"""core-keygen: generate throw-away Ed25519 test keys into the core-keys volume (never into the repo).

Idempotent: existing key files are kept (a second run changes nothing). Files (names, not values, are documented):
  identity.json / staff.json         public keys the runtime verifies (formats of agent_core.adapters.identity_keys)
  bridge-{identity,staff,callback,executor}.json   the bridge's private signers (credentials.issuer.load_signer format);
                                     the executor key (A03 iii, executor -> lab-broker) is a distinct keypair, never the callback key
  lab-broker-trust.json              PUBLIC half of the executor key, in the fixture-key format of the lab-broker double
  control-api-core-bridge.key        seed of the control-api -> core-bridge service key (iss control-api); control-api-trust.json
                                     = PUBLIC half of the callback key, for the control-api side of binding callbacks
  service.json                       public service keys (internal.auth.load_service_keys format)
  exporter-control-api.key / exporter-lab-broker.key   exporter private seeds (b64url, one line)
Runs as root inside the image, then hands ownership to the runtime uid (10001).

Local human issuer (plan 17.3.2, LOCAL stack only): when PULSO_HUMAN_STAFF_FRAGMENT names a file
`{"principal_keys": {"local-sim-human-*": <public key>}}` (PUBLIC keys only, written by local_identity.keys), it is merged into
staff.json next to the bridge bot key. The merge also applies to an already generated set and is idempotent. A kid that is not
a `local-sim-human-` kid, or that collides with an existing key, is refused (exit 2). identity.json never gets a human key.
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


HUMAN_KID_PREFIX = "local-sim-human-"


def merge_human_fragment(out: Path, fragment: Path) -> int:
    """Adds the public human kid(s) to staff.json. 0 ok / unchanged, 2 refused (names the problem, never a key)."""
    try:
        keys = json.loads(fragment.read_text(encoding="ascii"))["principal_keys"]
        assert isinstance(keys, dict) and keys
    except (OSError, ValueError, KeyError, AssertionError):
        print(f"core-keygen: human fragment unreadable ({fragment.name})", file=sys.stderr)
        return 2
    staff_path = out / "staff.json"
    staff = json.loads(staff_path.read_text(encoding="ascii"))
    current = staff["principal_keys"]
    changed = False
    for kid, pub in keys.items():
        if not (isinstance(kid, str) and kid.startswith(HUMAN_KID_PREFIX) and isinstance(pub, str) and pub):
            print("core-keygen: human fragment kid must start with local-sim-human-", file=sys.stderr)
            return 2
        if kid in current and current[kid] != pub:
            print("core-keygen: human fragment kid collides with an existing staff key", file=sys.stderr)
            return 2
        if pub in current.values() and current.get(kid) != pub:
            print("core-keygen: human fragment key equals an existing staff key", file=sys.stderr)
            return 2
        if kid not in current:
            current[kid] = pub
            changed = True
    if changed:
        write(staff_path, json.dumps(staff))
    return 0


def main(out: Path) -> int:
    out.mkdir(parents=True, exist_ok=True)
    fragment = os.environ.get("PULSO_HUMAN_STAFF_FRAGMENT", "").strip()
    marker = out / "service.json"
    if marker.exists():
        print("core-keygen: keys already present, nothing written")
        return merge_human_fragment(out, Path(fragment)) if fragment else 0
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
        service[f"exporter-{aud}"] = {"iss": "core-bridge", "aud": aud, "key": pub}
    seed, pub = new_pair()  # control-api -> core-bridge (A03 class i): the seed stays readable for the platform side
    write(out / "control-api-core-bridge.key", seed + "\n")
    service["control-api-core-bridge"] = {"iss": "control-api", "aud": "core-bridge", "key": pub}
    cb_kid, _, cb_pub = signers["bridge-callback"]  # binding callbacks (runtime -> control-api): PUBLIC half only
    write(out / "control-api-trust.json", json.dumps({"keys": {cb_kid: {"iss": "core-bridge", "aud": "control-api", "key": cb_pub}}}), 0o644)
    seed, pub = new_pair()  # smoke probe (core-bridge audience): used by local/core/smoke.ps1 -Exec only
    write(out / "smoke-probe.key", seed + "\n")
    service["smoke-probe"] = {"iss": "pulso-smoke", "aud": "core-bridge", "key": pub}
    write(marker, json.dumps({"keys": service}))  # written last: marks a complete set
    state = Path(os.environ.get("PULSO_EXPORTER_STATE_DIR", "/var/lib/exporter"))
    if state.is_dir():
        os.chown(state, UID, UID)
    if fragment:
        rc = merge_human_fragment(out, Path(fragment))
        if rc:
            return rc
    print("core-keygen: wrote", len(list(out.iterdir())), "files")
    return 0


if __name__ == "__main__":
    sys.exit(main(Path(os.environ.get("PULSO_KEYS_DIR", "/run/pulso-keys"))))
