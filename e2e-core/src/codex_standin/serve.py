"""Container entrypoint of the `e2e-fixtures` DOUBLE: control-api + lab-broker + scripted LLM + ingest fixture.
Reads E2E_VERIFY_KEYS (public keys only) and listens on 0.0.0.0:E2E_PORT (default 8700)."""

from __future__ import annotations

import json
import os


def build() -> object:
    from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PublicKey

    from codex_standin.fixtures_app import World, create_app
    from codex_standin.jwtsvc import KeyRing, b64d

    cfg = json.loads(os.environ["E2E_VERIFY_KEYS"])
    world = World(KeyRing({kid: tuple(v) for kid, v in cfg["ring"].items()}))
    ingest = None
    if cfg.get("ingest"):
        from ingest_fixture.app import FixtureAuth, FixtureKey, IngestState
        from ingest_fixture.app import create_app as ingest_app

        ing = cfg["ingest"]
        keys = {kid: FixtureKey(v["iss"], v["aud"], Ed25519PublicKey.from_public_bytes(b64d(v["key"])))
                for kid, v in ing["keys"].items()}
        ingest = ingest_app(IngestState(), FixtureAuth(keys, ing["binding_ref"], ing["tenant_id"]))
    return create_app(world, ingest)


if __name__ == "__main__":
    import uvicorn

    uvicorn.run(build(), host="0.0.0.0", port=int(os.environ.get("E2E_PORT", "8700")), log_level="warning",
                access_log=False)
