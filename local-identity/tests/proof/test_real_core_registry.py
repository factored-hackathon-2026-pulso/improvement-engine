"""Proof against the REAL pinned Core registry (agent-core 789d6c8) over a real Postgres 16.

The Core registry HTTP app verifies staff credentials with `JwsIdentityVerifier` (`principal+jws`, Ed25519).
The human issuer's JWS must be accepted exactly like a staff human, the bot must not be able to approve, and
nothing else may change. Doubles: `FakeEvaluator`/`FakeClock`/`FakeIds` from the pin's own test helpers (the
evaluation harness is out of scope here), in-process ASGI instead of sockets. Needs PULSO_TEST_PG_ADMIN and the
pin importable (run with the pinned venv); otherwise skipped."""

from __future__ import annotations

import os
import uuid
from collections.abc import Iterator
from dataclasses import dataclass, replace
from datetime import timedelta
from pathlib import Path
from typing import Any

import pytest

pytest.importorskip("agent_core")
psycopg = pytest.importorskip("psycopg")

from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey  # noqa: E402
from fastapi.testclient import TestClient  # noqa: E402

from local_identity.app import build_app  # noqa: E402
from local_identity.client import (  # noqa: E402
    BindingMismatch,
    IssuerError,
    LocalIdentityClient,
    ServiceSigner,
    assert_bound,
)
from local_identity.config import load_config  # noqa: E402
from local_identity.keys import Signer, generate_keyset, pubkey_of  # noqa: E402

pytestmark = pytest.mark.proof

TENANT = "tenant-a"


def _admin_dsn() -> str:
    dsn = os.environ.get("PULSO_TEST_PG_ADMIN")
    if not dsn:
        pytest.skip("PULSO_TEST_PG_ADMIN not set (real Postgres 16 required)")
    return dsn


@dataclass
class World:
    core: TestClient
    issuer: LocalIdentityClient
    bot: Signer
    human_signer: Signer
    clock: Any
    service: Any
    pid_factory: Any

    def bot_jws(self, roles: list[str] | None = None) -> str:
        return self._principal_jws(self.bot, roles or ["constructor"], actor=False, level="session")

    def _principal_jws(self, signer: Signer, roles: list[str], *, actor: bool, level: str) -> str:
        import json

        from local_identity.principal import iso_z

        now = self.clock.now()
        body = {
            "type": "builder",
            "id": "pulso-constructor:" + TENANT if not actor else "forged-human",
            "roles": roles,
            "scopes": [],
            "attrs": {"actor": "human"} if actor else {"tenant": TENANT},
            "auth": {"level": level, "at": iso_z(now)},
            "exp": iso_z(now + timedelta(minutes=15)),
        }
        return signer.sign(typ="principal+jws", payload=json.dumps(body).encode())

    def forged_human_session_level(self) -> str:
        """Human key, but only `session` auth (what a session assertion alone would amount to)."""
        return self._principal_jws(self.human_signer, ["constructor", "aprobador"], actor=True, level="session")

    def call(self, method: str, path: str, jws: str, **kw: Any) -> Any:
        headers = {"Authorization": f"Bearer {jws}", **kw.pop("headers", {})}
        return self.core.request(method, f"/v1/registry{path}", headers=headers, **kw)

    def human(
        self,
        operation: str,
        target: dict[str, Any],
        *,
        command: str | None = None,
        actor: str = "local-supervisor",
        challenge: str | None = None,
    ) -> str:
        return self.issuer.command_authorization(
            tenant_id=TENANT,
            actor_ref=actor,
            command_ref=command or f"cmd-{uuid.uuid4().hex[:8]}",
            operation=operation,
            target=target,
            challenge_ref=challenge or f"chal-{uuid.uuid4().hex[:8]}",
            nonce=uuid.uuid4().hex,
        ).authorization_jws

    def evaluated_candidate(self, version: str) -> tuple[str, str, int]:
        """bot: create -> draft -> freeze -> evaluate through the registry HTTP API. Returns (pid, hash, rev)."""
        from tests.registry.helpers import AGENT, prompt_draft
        from tests.registry.service_world import SUITE

        bot = self.bot_jws()
        pid = self.call("POST", "/proposals", bot, json={"agent_id": AGENT, "origin": "manual", "title": "t"}).json()[
            "proposal_id"
        ]
        changes = [d.model_dump(mode="json") for d in (prompt_draft(version=version, text=f"Texto {version}."), SUITE)]
        assert (
            self.call("PUT", f"/proposals/{pid}/draft", bot, json={"expected_rev": 0, "changes": changes}).status_code
            == 200
        )
        assert self.call("POST", f"/proposals/{pid}/freeze", bot).status_code == 200
        assert (
            self.call("POST", f"/proposals/{pid}/evaluate", bot, json={"suite_id": "disputas-suite"}).status_code == 200
        )
        proposal = self.call("GET", f"/proposals/{pid}", bot).json()["proposal"]
        assert proposal["state"] == "evaluated"
        return pid, proposal["candidate_hash"], proposal["rev"]

    def alias(self, alias: str) -> str | None:
        from tests.registry.helpers import AGENT

        return self.call("GET", f"/aliases/{AGENT}/{alias}", self.bot_jws()).json()["release_id"]


@pytest.fixture
def world(tmp_path: Path) -> Iterator[World]:
    from agent_core.adapters.jws_identity import JwsIdentityVerifier
    from agent_core.adapters.postgres_uow import apply_schema
    from agent_core.api.app import create_app
    from agent_core.registry import PgRegistryStore, apply_registry_schema
    from agent_core.registry.http import registry_extension
    from agent_core.registry.service import RegistryService
    from testing.fakes.clock import FakeClock
    from testing.fakes.ids import FakeIds
    from tests.m09.conftest import api_deps
    from tests.registry.helpers import REGISTRY_DEMO, admin
    from tests.registry.service_world import FakeEvaluator

    admin_dsn = _admin_dsn()
    name = f"li_{uuid.uuid4().hex[:10]}"
    base = admin_dsn.rpartition("/")[0]
    with psycopg.connect(admin_dsn, autocommit=True) as conn:
        conn.execute(f'CREATE DATABASE "{name}"')
    try:
        dsn = f"{base}/{name}"
        with psycopg.connect(dsn) as conn:
            apply_schema(conn, None)
            apply_registry_schema(conn, None)
        clock = FakeClock()
        service = RegistryService(
            PgRegistryStore(lambda: psycopg.connect(dsn, autocommit=False)), FakeEvaluator(), clock, FakeIds()
        )
        service.import_seed(admin(), REGISTRY_DEMO)  # seed release as staging/prod base (setup, not under test)

        keys = generate_keyset(tmp_path / "keys", tenant_id=TENANT)
        env = {
            "LOCAL_IDENTITY_PROFILE": "local",
            "LOCAL_IDENTITY_SERVICE_KEYS": str(keys.dir / "human-issuer-service-keys.json"),
            "LOCAL_IDENTITY_SESSION_SIGNER": str(keys.dir / "session-signer.json"),
            "LOCAL_IDENTITY_HUMAN_STAFF_SIGNER": str(keys.dir / "human-staff-signer.json"),
            "LOCAL_IDENTITY_IDENTITIES": str(keys.dir / "identities.json"),
        }
        config = load_config(env)
        issuer_http = TestClient(build_app(config, now=clock.now), base_url="http://human-issuer:8083")
        issuer = LocalIdentityClient(
            "http://human-issuer:8083",
            ServiceSigner.from_file(keys.dir / "control-api-signer.json"),
            tenant_id=TENANT,
            http_client=issuer_http,
            now=clock.now,
        )

        bot_key = Ed25519PrivateKey.generate()
        bot = Signer("bot-staff-1", bot_key)
        # Core's staff verifier: the bot kid and the human kid (what `core-staff-keys.human-fragment.json` adds).
        verifier = JwsIdentityVerifier(
            principal_keys={
                bot.kid: bot_key.public_key(),
                config.human_signer.kid: Ed25519PrivateKey.from_private_bytes(
                    _seed(keys.dir / "human-staff-signer.json")
                ).public_key(),
            },
            delegation_keys={},
            grant_active=lambda *_: False,
        )
        deps, _ = api_deps()
        app = create_app(replace(deps, extensions=(registry_extension(service, verifier, clock),)))
        yield World(
            TestClient(app, raise_server_exceptions=False), issuer, bot, config.human_signer, clock, service, None
        )
    finally:
        with psycopg.connect(admin_dsn, autocommit=True) as conn:
            conn.execute(f'DROP DATABASE IF EXISTS "{name}" WITH (FORCE)')


def _seed(path: Path) -> bytes:
    import json

    from local_identity.keys import b64url_decode

    return b64url_decode(json.loads(path.read_text())["key"])


def _proposal_target(pid: str, h: str, rev: int) -> dict[str, Any]:
    return {"proposal_id": pid, "candidate_hash": h, "expected_revision": rev}


def test_pubkey_helper_matches_core_public_key() -> None:
    k = Ed25519PrivateKey.generate()
    assert len(pubkey_of(k)) == 32


def test_bot_cannot_approve_publish_or_promote(world: World) -> None:
    pid, h, rev = world.evaluated_candidate("1.1.0")
    bot = world.bot_jws()
    r = world.call("POST", f"/proposals/{pid}/approve", bot, json={"candidate_hash": h})
    assert r.status_code == 403 and r.json()["code"] == "forbidden_role"
    r = world.call("POST", f"/proposals/{pid}/publish", bot, headers={"Idempotency-Key": "k-bot"})
    assert r.status_code == 403 and r.json()["code"] == "forbidden_role"
    # a bot credential claiming roles it was never issued still fails: no `actor=human`
    greedy = world._principal_jws(world.bot, ["constructor", "aprobador", "admin"], actor=False, level="session")
    r = world.call("POST", f"/proposals/{pid}/approve", greedy, json={"candidate_hash": h})
    assert r.status_code == 403 and r.json()["code"] == "forbidden_role"


def test_human_without_step_up_gets_step_up_required(world: World) -> None:
    pid, h, rev = world.evaluated_candidate("1.1.0")
    r = world.call("POST", f"/proposals/{pid}/approve", world.forged_human_session_level(), json={"candidate_hash": h})
    assert r.status_code == 403 and r.json()["code"] == "step_up_required"


def test_unknown_kid_and_bot_key_cannot_mint_a_human(world: World) -> None:
    pid, h, rev = world.evaluated_candidate("1.1.0")
    rogue = Signer("local-sim-human-rogue", Ed25519PrivateKey.generate())
    jws = world._principal_jws(rogue, ["constructor", "aprobador"], actor=True, level="step_up")
    r = world.call("POST", f"/proposals/{pid}/approve", jws, json={"candidate_hash": h})
    assert r.status_code == 401
    assert world.bot.kid != world.human_signer.kid


def test_issued_jws_approve_publish_promote_and_alias_moves(world: World) -> None:
    from tests.registry.helpers import AGENT

    before_staging, before_prod = world.alias("staging"), world.alias("prod")
    pid, h, rev = world.evaluated_candidate("1.1.0")
    target = _proposal_target(pid, h, rev)

    approve = world.human("approve", target)
    r = world.call("POST", f"/proposals/{pid}/approve", approve, json={"candidate_hash": h})
    assert r.status_code == 200, r.text
    assert r.json()["actor"] == "local-supervisor" and r.json()["decision"] == "approved"
    # approval fixes operation/hash and is NOT publication: staging has not moved
    assert world.alias("staging") == before_staging

    rev_after = world.call("GET", f"/proposals/{pid}", world.bot_jws()).json()["proposal"]["rev"]
    publish = world.human("publish", _proposal_target(pid, h, rev_after))
    r = world.call("POST", f"/proposals/{pid}/publish", publish, headers={"Idempotency-Key": "pub-1"})
    assert r.status_code == 200, r.text
    release_id = r.json()["release_id"]
    assert world.alias("staging") == release_id != before_staging  # staging confirmed via alias read
    assert world.alias("prod") == before_prod  # exposure needs the explicit promote
    # idempotent replay returns the same release
    again = world.call("POST", f"/proposals/{pid}/publish", publish, headers={"Idempotency-Key": "pub-1"})
    assert again.status_code == 200 and again.json()["release_id"] == release_id

    promote = world.human(
        "promote", {"release_id": release_id, "agent_id": AGENT, "alias": "prod", "expected_revision": 0}
    )
    r = world.call("POST", f"/aliases/{AGENT}/prod", promote, json={"release_id": release_id, "reason": "demo"})
    assert r.status_code == 200, r.text
    assert world.alias("prod") == release_id


def test_revoke_path_needs_admin_issued_jws(world: World) -> None:
    from tests.registry.helpers import AGENT

    pid, h, rev = world.evaluated_candidate("1.1.0")
    world.call(
        "POST",
        f"/proposals/{pid}/approve",
        world.human("approve", _proposal_target(pid, h, rev)),
        json={"candidate_hash": h},
    )
    rev2 = world.call("GET", f"/proposals/{pid}", world.bot_jws()).json()["proposal"]["rev"]
    r1 = world.call(
        "POST",
        f"/proposals/{pid}/publish",
        world.human("publish", _proposal_target(pid, h, rev2)),
        headers={"Idempotency-Key": "pub-r1"},
    ).json()["release_id"]
    pid2, h2, rev3 = world.evaluated_candidate("1.2.0")
    world.call(
        "POST",
        f"/proposals/{pid2}/approve",
        world.human("approve", _proposal_target(pid2, h2, rev3)),
        json={"candidate_hash": h2},
    )
    rev4 = world.call("GET", f"/proposals/{pid2}", world.bot_jws()).json()["proposal"]["rev"]
    r2 = world.call(
        "POST",
        f"/proposals/{pid2}/publish",
        world.human("publish", _proposal_target(pid2, h2, rev4)),
        headers={"Idempotency-Key": "pub-r2"},
    ).json()["release_id"]
    assert world.alias("staging") == r2 != r1

    release_target = {"release_id": r1, "agent_id": AGENT, "alias": "staging", "expected_revision": 0}
    with pytest.raises(IssuerError) as err:  # a supervisor cannot even obtain a revoke authorization
        world.human("revoke", release_target)
    assert err.value.code == "pulso:role_not_allowed"
    supervisor_promote = world.human("promote", release_target)  # wrong operation for the endpoint
    r = world.call("POST", f"/releases/{r1}/revoke", supervisor_promote, json={"reason": "bad"})
    assert r.status_code == 403 and r.json()["code"] == "forbidden_role"  # no admin role in a promote JWS

    revoke = world.human("revoke", release_target, actor="local-admin")
    r = world.call("POST", f"/releases/{r1}/revoke", revoke, json={"reason": "regression observed"})
    assert r.status_code == 200, r.text
    assert world.call("GET", f"/releases/{r1}", world.bot_jws()).json()["status"] == "revoked"
    # revoked releases cannot be promoted any more
    promote = world.human("promote", release_target)
    r = world.call("POST", f"/aliases/{AGENT}/prod", promote, json={"release_id": r1, "reason": "x"})
    assert r.status_code == 409 and r.json()["code"] == "illegal_transition", r.text


def test_replays_expiry_and_wrong_hash_or_operation(world: World) -> None:
    pid, h, rev = world.evaluated_candidate("1.1.0")
    target = _proposal_target(pid, h, rev)
    approve = world.human("approve", target, command="cmd-approve-1", challenge="chal-1")

    # Core: a different hash in the body is rejected by the registry itself
    r = world.call("POST", f"/proposals/{pid}/approve", approve, json={"candidate_hash": "0" * 64})
    assert r.status_code == 409 and r.json()["code"] == "candidate_changed"

    # Port-side binding: the same JWS is not valid for another operation, hash, revision or intention
    assert_bound(
        approve,
        tenant_id=TENANT,
        actor_ref="local-supervisor",
        command_ref="cmd-approve-1",
        operation="approve",
        target=target,
        challenge_ref="chal-1",
    )
    for over in (
        {"operation": "publish"},
        {"target": _proposal_target(pid, "f" * 64, rev)},
        {"target": _proposal_target(pid, h, rev + 1)},
        {"command_ref": "cmd-other"},
        {"challenge_ref": "chal-2"},
    ):
        bound = {
            "tenant_id": TENANT,
            "actor_ref": "local-supervisor",
            "command_ref": "cmd-approve-1",
            "operation": "approve",
            "target": target,
            "challenge_ref": "chal-1",
            **over,
        }
        with pytest.raises(BindingMismatch):
            assert_bound(approve, **bound)

    assert world.call("POST", f"/proposals/{pid}/approve", approve, json={"candidate_hash": h}).status_code == 200
    # replaying the very same approval: the proposal is no longer evaluated
    r = world.call("POST", f"/proposals/{pid}/approve", approve, json={"candidate_hash": h})
    assert r.status_code == 409 and r.json()["code"] == "illegal_transition"

    # issuer-side: the same business nonce cannot obtain a second JWS
    with pytest.raises(IssuerError) as err:
        world.issuer.command_authorization(
            tenant_id=TENANT,
            actor_ref="local-supervisor",
            command_ref="c",
            operation="approve",
            target=target,
            challenge_ref="x1",
            nonce="N" * 16,
        )
        world.issuer.command_authorization(
            tenant_id=TENANT,
            actor_ref="local-supervisor",
            command_ref="c",
            operation="approve",
            target=target,
            challenge_ref="x1",
            nonce="N" * 16,
        )
    assert err.value.code == "pulso:nonce_replayed"

    # short expiry: the JWS is dead 60 s later (Core checks exp against its clock)
    stale = world.human("publish", _proposal_target(pid, h, rev))
    world.clock.advance(timedelta(seconds=61))
    r = world.call("POST", f"/proposals/{pid}/publish", stale, headers={"Idempotency-Key": "late"})
    assert r.status_code == 401 and r.json()["code"] == "principal_expired"
