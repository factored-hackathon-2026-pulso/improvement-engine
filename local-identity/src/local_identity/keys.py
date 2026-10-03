"""Ed25519 key helpers, JWS signing and the local keyset generator (`python -m local_identity.keys gen DIR`).

Key files are `{"kid": str, "key": b64url(32-byte seed)}`; public key files are `{"keys": {kid: {iss, aud, key}}}`
(service JWT verifiers) or `{"principal_keys": {kid: b64url(public)}}` (Core staff verifier fragment). Errors
name the file, never the value."""

from __future__ import annotations

import base64
import contextlib
import json
import os
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from cryptography.hazmat.primitives.serialization import Encoding, NoEncryption, PrivateFormat, PublicFormat

from local_identity import HUMAN_KID_PREFIX, SESSION_KID_PREFIX


class KeyFileError(ValueError):
    """A key file is unreadable or malformed; the message names the file only."""


def b64url_encode(raw: bytes) -> str:
    return base64.urlsafe_b64encode(raw).rstrip(b"=").decode("ascii")


def b64url_decode(text: str) -> bytes:
    return base64.urlsafe_b64decode(text + "=" * (-len(text) % 4))


def seed_of(key: Ed25519PrivateKey) -> bytes:
    return key.private_bytes(Encoding.Raw, PrivateFormat.Raw, NoEncryption())


def pubkey_of_seed(seed: bytes) -> bytes:
    return Ed25519PrivateKey.from_private_bytes(seed).public_key().public_bytes(Encoding.Raw, PublicFormat.Raw)


def pubkey_of(key: Ed25519PrivateKey) -> bytes:
    return key.public_key().public_bytes(Encoding.Raw, PublicFormat.Raw)


def jws_compact(key: Ed25519PrivateKey, *, kid: str, typ: str, payload: bytes) -> str:
    """Compact EdDSA JWS with the exact header `{alg, kid, typ}`."""
    head = b64url_encode(json.dumps({"alg": "EdDSA", "kid": kid, "typ": typ}, separators=(",", ":")).encode())
    body = b64url_encode(payload)
    return f"{head}.{body}.{b64url_encode(key.sign(f'{head}.{body}'.encode('ascii')))}"


class Signer:
    """One Ed25519 key with its `kid`. Never prints the key."""

    def __init__(self, kid: str, key: Ed25519PrivateKey) -> None:
        self.kid, self._key = kid, key

    def sign(self, *, typ: str, payload: bytes) -> str:
        return jws_compact(self._key, kid=self.kid, typ=typ, payload=payload)

    @property
    def public_key(self) -> bytes:
        return pubkey_of(self._key)

    def __repr__(self) -> str:
        return f"Signer(kid={self.kid!r})"


def load_signer(path: Path) -> Signer:
    try:
        data = json.loads(path.read_bytes())
        seed = b64url_decode(data["key"])
        if len(seed) != 32 or not isinstance(data["kid"], str) or not data["kid"]:
            raise ValueError
        return Signer(data["kid"], Ed25519PrivateKey.from_private_bytes(seed))
    except (OSError, KeyError, TypeError, ValueError):
        raise KeyFileError(f"signer file unreadable ({path.name})") from None


def _write_json(path: Path, value: Any, *, secret: bool = False) -> None:
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    if secret:
        with contextlib.suppress(OSError):  # Windows / read-only mounts: best effort only
            os.chmod(path, 0o600)


@dataclass(frozen=True)
class Keyset:
    dir: Path
    human_kid: str
    session_kid: str
    control_kid: str


def generate_keyset(directory: Path, *, tenant_id: str = "local-tenant") -> Keyset:
    """Three distinct keypairs (control-api service, session assertion, Core human staff) plus the
    public-side files each consumer needs. The directory must be an ignored local secret dir."""
    directory.mkdir(parents=True, exist_ok=True)
    control, session, human = (Ed25519PrivateKey.generate() for _ in range(3))
    control_kid, session_kid, human_kid = "control-api-human-issuer-1", f"{SESSION_KID_PREFIX}1", f"{HUMAN_KID_PREFIX}1"
    for name, kid, key in (
        ("control-api-signer.json", control_kid, control),
        ("session-signer.json", session_kid, session),
        ("human-staff-signer.json", human_kid, human),
    ):
        _write_json(directory / name, {"kid": kid, "key": b64url_encode(seed_of(key))}, secret=True)
    _write_json(
        directory / "human-issuer-service-keys.json",
        {
            "keys": {
                control_kid: {"iss": "control-api", "aud": "human-issuer", "key": b64url_encode(pubkey_of(control))}
            }
        },
    )
    _write_json(
        directory / "control-api-session-verifier-keys.json",
        {
            "keys": {
                session_kid: {"iss": "human-issuer", "aud": "control-api", "key": b64url_encode(pubkey_of(session))}
            }
        },
    )
    _write_json(
        directory / "core-staff-keys.human-fragment.json",
        {"principal_keys": {human_kid: b64url_encode(pubkey_of(human))}},
    )
    _write_json(
        directory / "identities.json",
        {
            "identities": {
                "local-supervisor": {"tenant_id": tenant_id, "roles": ["constructor", "aprobador"]},
                "local-admin": {"tenant_id": tenant_id, "roles": ["constructor", "aprobador", "admin"]},
                "local-builder-human": {"tenant_id": tenant_id, "roles": ["constructor"]},
            }
        },
    )
    return Keyset(directory, human_kid, session_kid, control_kid)


def main(argv: list[str] | None = None) -> int:
    args = sys.argv[1:] if argv is None else argv
    if len(args) < 2 or args[0] != "gen":
        print("usage: python -m local_identity.keys gen <secret-dir> [tenant_id]", file=sys.stderr)
        return 2
    keys = generate_keyset(Path(args[1]), tenant_id=args[2] if len(args) > 2 else "local-tenant")
    print(f"keyset written to {keys.dir} (kids: {keys.control_kid}, {keys.session_kid}, {keys.human_kid})")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
