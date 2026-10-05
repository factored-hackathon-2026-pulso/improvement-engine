r"""Contract: a credential minted by the ENGINE (core-client `ServiceIdentity`) is accepted, as a `builder` principal, by the REAL
agent-core `serve` verifier and registry HTTP API (the staff-keys path: `JwsIdentityVerifier` + `registry_extension` with a clock).

Opt-in (skipped, with the reason, when the inputs are missing):
  AGENT_CORE_DIR   a checkout of agent-core (read-only; put on sys.path), whose dependencies the running Python has (its venv)
  PULSO_MINT_EXE   the built example:  cargo build -p core-client --example mint_service_credential

    $env:AGENT_CORE_DIR = 'D:\.codex\factored\tmp\survey\ac-engprod'
    $env:PULSO_MINT_EXE = 'D:\cargo-targets\claude-engprod\debug\examples\mint_service_credential.exe'
    <agent-core venv python> -m unittest scripts/serve-credentials/test_serve_credential_contract.py

The seed is generated per run (random, never a real one); nothing secret is read or printed.
"""
import json
import os
import secrets
import subprocess
import sys
import unittest
from datetime import timedelta
from pathlib import Path

AC = os.environ.get("AGENT_CORE_DIR", "")
EXE = os.environ.get("PULSO_MINT_EXE", "")
KID = "pulso-engine-contract-1"
SKIP = None
if not AC or not Path(AC, "agent_core").is_dir():
    SKIP = "AGENT_CORE_DIR is not set to an agent-core checkout"
elif not EXE or not Path(EXE).is_file():
    SKIP = "PULSO_MINT_EXE is not set to the built mint_service_credential example"
if SKIP is None:
    sys.path.insert(0, AC)
    try:
        from dataclasses import replace

        from fastapi.testclient import TestClient

        from agent_core.adapters.jws_identity import JwsIdentityVerifier, b64url_decode
        from agent_core.api.app import create_app
        from agent_core.registry.http import registry_extension
        from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PublicKey
        from tests.m09.conftest import api_deps
        from tests.registry.helpers import AGENT
        from tests.registry.service_world import World
    except Exception as exc:  # the checkout or its dependencies are not importable from this Python
        SKIP = f"agent-core is not importable from this Python ({type(exc).__name__})"

BODY = lambda: {"agent_id": AGENT, "origin": "auto_detect", "title": "[improvement-engine] contract"}  # noqa: E731


def mint(seed: str, now: int, *extra: str) -> dict:
    env = {k: v for k, v in os.environ.items() if not k.startswith(("PULSO_", "AGENTCORE_"))}
    env.update(PULSO_SERVICE_SEED_HEX=seed, PULSO_SERVICE_KID=KID)
    out = subprocess.run([EXE, "--now", str(now), *extra], env=env, capture_output=True, text=True, check=True)
    return json.loads(out.stdout)


@unittest.skipIf(SKIP is not None, SKIP or "")
class ServeCredentialContract(unittest.TestCase):
    def setUp(self) -> None:
        self.seed = secrets.token_hex(32)
        self.world = World()
        self.now = int(self.world.clock.now().timestamp())
        self.minted = mint(self.seed, self.now)
        key = Ed25519PublicKey.from_public_bytes(b64url_decode(self.minted["public_key"]))
        self.verifier = JwsIdentityVerifier({KID: key}, {}, lambda ref, at: False)
        deps, _ = api_deps()
        from testing.fakes.identity import TestIdentityIssuer

        customers = TestIdentityIssuer(self.world.clock)
        app = create_app(replace(deps, verifier=customers.verifier(),
                                 extensions=(registry_extension(self.world.service, self.verifier, self.world.clock),)))
        self.client = TestClient(app, raise_server_exceptions=False)

    def post(self, token: str):
        return self.client.post("/v1/registry/proposals", json=BODY(), headers={"Authorization": f"Bearer {token}"})

    def test_the_serve_verifier_reads_the_credential_as_a_builder_principal_with_constructor_only(self) -> None:
        p = self.verifier.verify(self.minted["credential"])
        self.assertEqual((p.type.value, p.id, p.roles, p.scopes, p.attrs), ("builder", "pulso-engine", ["constructor"], [], {}))
        self.assertEqual(p.auth.level.value, "session")
        self.assertEqual(p.exp - p.auth.at, timedelta(minutes=5), "a short-lived credential: minutes, not hours")

    def test_the_registry_api_accepts_it_and_it_creates_a_proposal_as_pulso_engine(self) -> None:
        r = self.post(self.minted["credential"])
        self.assertEqual(r.status_code, 201, r.text)
        self.assertEqual(r.json()["created_by"], "pulso-engine")

    def test_it_expires_with_the_serve_clock_and_a_fresh_one_works_again(self) -> None:
        self.world.clock.advance(timedelta(minutes=5, seconds=1))
        r = self.post(self.minted["credential"])
        self.assertEqual((r.status_code, r.json()["code"]), (401, "principal_expired"))
        fresh = mint(self.seed, int(self.world.clock.now().timestamp()))
        self.assertEqual(self.post(fresh["credential"]).status_code, 201)

    def test_a_credential_refreshed_before_expiry_is_valid_where_the_old_one_is_about_to_die(self) -> None:
        self.world.clock.advance(timedelta(minutes=4))  # inside the last fifth of its life: the engine refreshes here
        fresh = mint(self.seed, int(self.world.clock.now().timestamp()))
        self.world.clock.advance(timedelta(minutes=2))  # the first one is dead now, the refreshed one lives
        self.assertEqual(self.post(self.minted["credential"]).status_code, 401)
        self.assertEqual(self.post(fresh["credential"]).status_code, 201)

    def test_the_wrong_key_an_unlisted_kid_and_a_tampered_credential_are_refused(self) -> None:
        other = mint(secrets.token_hex(32), self.now)  # same kid, another seed: the signature does not verify
        self.assertEqual(self.post(other["credential"]).status_code, 401)
        head, body, sig = self.minted["credential"].split(".")
        flipped = ("A" if sig[0] != "A" else "B") + sig[1:]
        self.assertEqual(self.post(".".join([head, body, flipped])).status_code, 401)
        env = {k: v for k, v in os.environ.items() if not k.startswith("PULSO_")}
        env.update(PULSO_SERVICE_SEED_HEX=self.seed, PULSO_SERVICE_KID="not-listed")
        stray = json.loads(subprocess.run([EXE, "--now", str(self.now)], env=env, capture_output=True, text=True, check=True).stdout)
        self.assertEqual(self.post(stray["credential"]).status_code, 401)

    def test_an_exporter_credential_cannot_open_proposals(self) -> None:
        exporter = mint(self.seed, self.now, "--roles", "exporter", "--id", "pulso-poller")
        self.assertEqual(self.post(exporter["credential"]).status_code, 403)


if __name__ == "__main__":
    unittest.main()
