"""Mint LOCAL dev identities for the Pulso dev stack. Run with the agent-core project environment:

    uv run --project <agent-core> python identity.py keys|mint --state-dir <dir>

`keys`  writes identity-keys.json / staff-keys.json (public keys only) and an engine Ed25519 key (private, kept in
        the gitignored state dir, dev only).
`mint`  writes tokens.json with `admin` (registry import, step-up human), `builder` (the engine-signed
        `builder` principal used for POST /v1/runs) and `exporter` (read-only, for the trigger poller). Tokens last 12 h. Nothing is printed.
"""
import argparse
import json
from datetime import timedelta
from pathlib import Path

from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from cryptography.hazmat.primitives.serialization import (Encoding, NoEncryption, PrivateFormat, PublicFormat,
                                                           load_pem_private_key)

from agent_core.adapters.jws_identity import ALG, PRINCIPAL_TYP, b64url_encode
from agent_core.adapters.system_clock import SystemClock
from agent_core.domain import Principal, dumps
from testing.fakes.identity import TestIdentityIssuer, TestStaffIssuer, sign_jws

ENGINE_KID = "pulso-engine-dev-1"
TTL = timedelta(hours=12)


def _pub(key: Ed25519PrivateKey) -> str:
    return b64url_encode(key.public_key().public_bytes(Encoding.Raw, PublicFormat.Raw))


def _engine_key(state: Path) -> Ed25519PrivateKey:
    path = state / "engine-key.pem"
    if path.exists():
        return load_pem_private_key(path.read_bytes(), password=None)
    key = Ed25519PrivateKey.generate()
    path.write_bytes(key.private_bytes(Encoding.PEM, PrivateFormat.PKCS8, NoEncryption()))
    return key


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("cmd", choices=["keys", "mint"])
    ap.add_argument("--state-dir", type=Path, required=True)
    args = ap.parse_args()
    state: Path = args.state_dir
    state.mkdir(parents=True, exist_ok=True)
    clock = SystemClock()
    cust, staff, engine = TestIdentityIssuer(clock), TestStaffIssuer(clock, ttl=TTL), _engine_key(state)
    if args.cmd == "keys":
        principal_keys = {cust.principal_kid: _pub(cust.principal_key), staff.principal_kid: _pub(staff.principal_key),
                          ENGINE_KID: _pub(engine)}
        (state / "identity-keys.json").write_text(json.dumps({
            "principal_keys": principal_keys, "delegation_keys": {cust.delegation_kid: _pub(cust.delegation_key)}},
            indent=2), encoding="utf-8")
        (state / "staff-keys.json").write_text(json.dumps(
            {"principal_keys": {staff.principal_kid: _pub(staff.principal_key),
                                # the registry HTTP API verifies ONLY these keys: trusting the engine kid lets the engine call
                                # /v1/registry as its own `builder` principal (create_proposal, put_draft, validate; B2 writer)
                                ENGINE_KID: _pub(engine)}}, indent=2), encoding="utf-8")
        return 0
    now = clock.now()
    builder = Principal.model_validate({
        "type": "builder", "id": "pulso-engine", "roles": ["constructor"], "attrs": {},
        "auth": {"level": "session", "at": now}, "exp": now + TTL})
    header = {"alg": ALG, "kid": ENGINE_KID, "typ": PRINCIPAL_TYP}
    # ENV1 (G9): read-only `exporter` principal for scripts/triggers/agentcore_poller.py (GET /v1/export/*, staff verifier). Same engine
    # kid (already in staff-keys); no constructor role, so it cannot touch proposals.
    exporter = Principal.model_validate({
        "type": "builder", "id": "pulso-poller", "roles": ["exporter"], "attrs": {},
        "auth": {"level": "session", "at": now}, "exp": now + TTL})
    tokens = {"admin": staff.admin(), "builder": sign_jws(header, dumps(builder).encode("utf-8"), engine),
              "exporter": sign_jws(header, dumps(exporter).encode("utf-8"), engine)}
    (state / "tokens.json").write_text(json.dumps(tokens), encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
