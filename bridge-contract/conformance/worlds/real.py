"""World: OUR real composed runtime (`pulso_core_runtime.main._compose`) on a throwaway PG16, served over real HTTP
(uvicorn on 127.0.0.1) so the conformance suite talks to it exactly like an external client.

REAL: PG16, pinned agent-core (Core API, engine, M9 identity, registry, native evaluator), all `pulso_core_runtime`
code. DOUBLES (declared in every report): loopback control-api + lab-broker + stateful bank (HTTP), the LLM gateway
(scripted), decision providers + calibration of the fixture bank, the seed fakes of the pin test helpers (only used
to import the seed world). Needs PULSO_TEST_PG_ADMIN (names only) and the pinned agent-core importable."""

from __future__ import annotations

import hashlib
import json
import os
import shutil
import socket
import threading
import time
import uuid
from datetime import UTC, datetime, timedelta
from decimal import Decimal
from pathlib import Path
from typing import Any

import psycopg
import pytest
import uvicorn
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey

from conformance.kit import Api, Signer, binding_ref, idempotency_key
from conformance.worlds import World

DOUBLES = ["loopback control-api + lab-broker + bank (integration/loopback.py)",
           "scripted LLM gateway (DualGateway) + decision providers",
           "seed fakes of the pin test helpers (world import only)"]


def _free_port() -> int:
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return int(s.getsockname()[1])


class RealWorld(World):
    target = "real"
    doubles = DOUBLES

    def __init__(self) -> None:
        admin_dsn = os.environ.get("PULSO_TEST_PG_ADMIN")
        if not admin_dsn:
            pytest.skip("PULSO_TEST_PG_ADMIN not set (real Postgres 16 required for the in-process real target)")
        from agent_core.adapters.postgres_uow import apply_schema
        from agent_core.registry import (
            PgRegistryStore,
            PostgresRegistry,
            apply_registry_schema,
        )
        from agent_core.registry.service import RegistryService
        from integration.conftest import WORLD, Composed, boot, pub, seed
        from integration.loopback import BankBackend, Loopback
        from runtime.conftest import PgDbs, _with_db

        self._tmp = Path(os.environ.get("TEMP", ".")) / f"bridge-contract-{uuid.uuid4().hex[:8]}"
        self._tmp.mkdir(parents=True, exist_ok=True)
        tag = uuid.uuid4().hex[:10]
        names = [f"bc_{tag}_{k}" for k in ("rt", "ev", "ot")]
        self._admin = psycopg.connect(admin_dsn, autocommit=True, connect_timeout=5)
        self._names = names
        for n in names:
            self._admin.execute(f'CREATE DATABASE "{n}"')
        for n, reg in ((names[0], True), (names[1], False)):
            with psycopg.connect(_with_db(admin_dsn, n)) as conn:
                apply_schema(conn, None)
                if reg:
                    apply_registry_schema(conn, None)
        pg = PgDbs(*(_with_db(admin_dsn, n) for n in names), admin=admin_dsn)
        loop = Loopback(BankBackend())
        ident, staff, svc, callback, executor = (Ed25519PrivateKey.generate() for _ in range(5))
        d = self._tmp
        (d / "identity.json").write_text(json.dumps({"principal_keys": {"id1": pub(ident)},
                                                     "delegation_keys": {"id1": pub(ident)}}))
        (d / "staff.json").write_text(json.dumps({"principal_keys": {"st1": pub(staff)}}))
        (d / "service.json").write_text(json.dumps({"keys": {"cp1": {"iss": "control-api", "aud": "core-bridge",
                                                                      "key": pub(svc)}}}))
        for name, kid, key in (("bridge-identity", "id1", ident), ("bridge-staff", "st1", staff),
                               ("bridge-callback", "cb1", callback), ("bridge-executor", "ex1", executor)):
            (d / f"{name}.json").write_text(json.dumps({"kid": kid, "key": seed(key)}))
        (d / "budgets.json").write_text(json.dumps(
            {"bud-1": {"cost_usd_max": "5", "tokens_max": 100000, "jobs_max": 40}}))
        # the key material the SUITE reads (names only in docs): a seed file, like an external target would give
        self.key_file = d / "contract-service-key.json"
        self.key_file.write_text(json.dumps({"kid": "cp1", "seed": seed(svc)}))
        c = Composed(pg=pg, loop=loop, dir=d, svc_key=svc, ident=ident)
        self._seed_world(c, pg, WORLD, RegistryService, PgRegistryStore, PostgresRegistry)
        from integration.conftest import build_env
        extra = {"PULSO_ALLOWED_TENANTS": "t1,t2"}
        captured: dict[str, Any] = {}
        c = _boot(c, boot, build_env, extra)
        captured["c"] = c
        self.c = c
        self.control = _Control(c)
        self.tenant, self.other_tenant, self.unknown_tenant = "t1", "t2", "t-not-deployed"
        self.scout_release, self.writer_release = c.release_ids["scout"], c.release_ids["writer"]
        self.agent_version = "1.0.0"
        from l5.world import AGENT
        self.arm_agent, self.arm_release, self.budget_ref = AGENT, "rel-demo", "bud-1"
        self.port = _free_port()
        cfg = uvicorn.Config(c.app, host="127.0.0.1", port=self.port, log_level="warning", lifespan="off")
        self._server = uvicorn.Server(cfg)
        self._thread = threading.Thread(target=self._server.run, daemon=True)
        self._thread.start()
        deadline = time.time() + 30
        while not self._server.started and time.time() < deadline:
            time.sleep(0.05)
        self.base_url = f"http://127.0.0.1:{self.port}"
        self.signer = Signer("cp1", _seed_bytes(self.key_file))
        self.api = Api(self.base_url, self.signer, self.tenant)
        self.caps = {"invoke", "writer", "evaluation", "control", "credentials", "arms", "authoring"}
        self._WORLD_OK = WORLD.is_dir()

    # -- seeding (copy of integration.conftest.composed, session-scoped) ---------------------------------------------
    @staticmethod
    def _seed_world(c: Any, pg: Any, world: Path, RegistryService: Any, PgRegistryStore: Any, PostgresRegistry: Any) -> None:
        from agent_core.domain import AgentSelector
        from l5.world import seed_demo
        from testing.builders import principal
        from testing.fakes.clock import FakeClock
        from testing.fakes.ids import FakeIds
        from tests.registry.helpers import admin
        from tests.registry.service_world import FakeEvaluator

        store = PgRegistryStore(lambda: psycopg.connect(pg.runtime, autocommit=False))
        clock = FakeClock()
        if world.is_dir():
            RegistryService(store, FakeEvaluator(), clock, FakeIds()).import_seed(admin(), world)
        seed_demo(store)
        c.store, c.registry = store, PostgresRegistry(store, clock)
        if world.is_dir():
            for stage in ("scout", "writer"):
                c.release_ids[stage] = c.registry.resolve_release(
                    AgentSelector.parse(f"pulso-{stage}@prod"),
                    principal(type="builder", id="b", roles=["constructor"], attrs={})).id

    # -- hooks ------------------------------------------------------------------------------------------------------
    def before_scout(self) -> None:
        """Scripts the scout model: first agent call queries the lab, the second answers `final` citing it."""
        from agent_core.ports import GenerationResult
        from integration.conftest import SCOUT_OUTPUT

        gw = self.c.gateway
        gw.agent_calls = 0
        original = gw._citing.generate

        def generate(prompt: Any, view: Any, locale: Any, schema: Any = None) -> Any:
            if not (isinstance(view, dict) and "goal" in view):
                return original(prompt, view, locale, schema)
            gw.agent_calls += 1
            if gw.agent_calls == 1:
                out: dict[str, Any] = {"kind": "tool_call", "tool": "pulso/lab_query@1.0.0", "args": {"sql": "select 1"}}
            else:
                hyp = {**SCOUT_OUTPUT["hypotheses"][0],
                       "evidence_refs": [{"id": "res-1", "digest": "e" * 64, "media_type": "application/json"}]}
                out = {"kind": "final", "output": {"schema_version": "1", "hypotheses": [hyp]}}
            return GenerationResult(output=out, tokens_in=10, tokens_out=5, cost_usd=Decimal("0.001"), model="double-model")

        gw.generate = generate  # type: ignore[method-assign]

    def seal_artifact(self, name: str, content: Any) -> None:
        from agent_core.domain.json import canonical_bytes
        digest = hashlib.sha256(canonical_bytes(content)).hexdigest()
        self.c.loop.backend.artifacts[name] = {
            "schema_version": "1", "artifact": {"id": name, "digest": digest, "media_type": "application/json"},
            "encoding": "json", "content": content, "byte_length": len(canonical_bytes(content))}

    def preauthorize(self, ref: str) -> None:  # the real loopback authorises every binding by default
        return None

    def writer_plan(self, tag: str) -> dict[str, Any]:
        """Seals the draft plan artifact and returns what the writer invocation needs (plan ref, commitment, digests)."""
        from agent_core.registry import EvalSuite
        from agent_core.registry.entities import content_hash
        from l5.world import prompt_draft, suite_draft
        from pulso_core_runtime.tools.builder import put_draft_digest

        changes = [d.model_dump(mode="json") for d in (prompt_draft(), suite_draft())]
        plan_ref = f"plan-{tag}"
        self.seal_artifact(plan_ref, {"agent_id": "atencion", "title": f"pulso-key:{tag}", "changes": changes})
        suite = EvalSuite.model_validate(changes[1]["content"])
        return {"plan_ref": plan_ref, "base_release_id": "rel-demo", "suite_digest": content_hash(suite),
                "suite_id": "disputas-suite", "suite_version": "1.0.0", "title": f"pulso-key:{tag}",
                "commitment": {
                    "mode": "write", "base_release_id": "rel-demo", "create_agent_id": "atencion",
                    "create_origin": "builder_chat", "create_title": f"pulso-key:{tag}",
                    "put_draft_digest": put_draft_digest(None, None, changes),
                    "operations": ["create_proposal", "put_draft", "freeze"]}}

    def new_candidate(self, tag: str) -> dict[str, Any]:
        """Runs the committed write sequence over HTTP (invoke writer) and returns the frozen candidate facts."""
        plan = self.writer_plan(tag)
        job, logical = f"job-w-{tag}", f"w-{tag}"
        body = self.invocation("writer", job, logical, input={
            "draft_plan_ref": plan["plan_ref"], "proposal_id": None, "base_release_id": plan["base_release_id"],
            "evaluate_enabled": False}, registry_mutation_commitment=plan["commitment"])
        r = self.invoke(body, job)
        assert r.status_code == 200 and r.json()["state"] == "terminal_ok", r.text
        receipts = r.json()["result"]["facts"]["pulso_writer_receipts"]["value"]
        return {**plan, "proposal_id": receipts["proposal_id"], "candidate_hash": receipts["candidate_hash"],
                "writer_receipt": r.json()}

    def seal_manifest(self, ref: str) -> None:
        """Seals a one-scenario manifest (the demo suite's first scenario) under `ref` in the loopback broker."""
        from l5.world import suite_with
        scenarios = [suite_with().scenarios[0].model_dump(mode="json")]
        self.seal_artifact(ref, {"scenarios": scenarios, "entries": {s["id"]: {} for s in scenarios}})

    def close(self) -> None:
        self._server.should_exit = True
        self._thread.join(5)
        self.api.close()
        self.c.loop.close()
        for n in self._names:
            self._admin.execute(f'DROP DATABASE IF EXISTS "{n}" WITH (FORCE)')
        self._admin.close()
        shutil.rmtree(self._tmp, ignore_errors=True)


class _Control:
    """Fault hooks on the loopback doubles (real target only)."""

    def __init__(self, c: Any) -> None:
        self._c = c

    def deny_binding(self) -> None:
        self._c.loop.backend.bind_mode = "conflict"

    def restore(self) -> None:
        self._c.loop.backend.bind_mode = "ok"

    @property
    def model_calls(self) -> int:
        return int(self._c.gateway.agent_calls)


def _seed_bytes(key_file: Path) -> bytes:
    from conformance.kit import b64u_decode
    return b64u_decode(json.loads(key_file.read_text())["seed"])


def _boot(c: Any, boot: Any, build_env: Any, extra: dict[str, str]) -> Any:
    return boot(c, extra)


def start() -> RealWorld:
    return RealWorld()


__all__ = ["UTC", "RealWorld", "binding_ref", "datetime", "idempotency_key", "start", "timedelta"]
