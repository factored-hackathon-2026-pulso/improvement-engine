"""Runs INSIDE the runtime container (smoke -Exec): mints a short-lived service JWT with the throw-away smoke key from the
core-keys volume and prints the JSON body of GET /internal/v1/version. Prints no key material."""

from __future__ import annotations

import base64
import json
import time
import urllib.request
import uuid
from pathlib import Path

from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey


def b64u(raw: bytes) -> str:
    return base64.urlsafe_b64encode(raw).rstrip(b"=").decode()


def main() -> None:
    seed = base64.urlsafe_b64decode(Path("/run/pulso-keys/smoke-probe.key").read_text().strip() + "==")
    key = Ed25519PrivateKey.from_private_bytes(seed)
    now = int(time.time())
    header = b64u(json.dumps({"alg": "EdDSA", "kid": "smoke-probe", "typ": "JWT"}, separators=(",", ":")).encode())
    body = b64u(json.dumps({"iss": "pulso-smoke", "aud": "core-bridge", "sub": "smoke", "purpose": "version_probe",
                            "tenant_id": "tenant-local", "scope": "version_probe", "jti": uuid.uuid4().hex,
                            "iat": now, "exp": now + 60}, separators=(",", ":")).encode())
    token = f"{header}.{body}.{b64u(key.sign(f'{header}.{body}'.encode()))}"
    req = urllib.request.Request("http://127.0.0.1:8000/internal/v1/version", headers={"Authorization": f"Bearer {token}"})
    print(urllib.request.urlopen(req, timeout=5).read().decode())


if __name__ == "__main__":
    main()
