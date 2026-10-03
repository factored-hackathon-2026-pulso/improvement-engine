"""Composed-runtime world (`main._compose`) on real PG16.

REAL: PG16 (registry, eval DB, bridge schemas), pinned agent-core 789d6c8 (Core API, engine, M9 identity,
registry service, in-process evaluator), `pulso_core_runtime` (invoke, receipts, tools, guards, evaluation).
DOUBLES (declared in every report): Loopback control-api + lab-broker (HTTP, route-table driven), the LLM
gateway (scripted/`CitingGateway`), the decision providers + calibration of the evaluation fixture bank, the seed
`FakeEvaluator/FakeClock/FakeIds` of the pin test helpers used only to import the seed world."""

from __future__ import annotations

import dataclasses
import io
import json
import time
import uuid
from collections.abc import Iterator
from dataclasses import dataclass, field
from decimal import Decimal
from pathlib import Path
from typing import Any

import psycopg
import pytest
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from cryptography.hazmat.primitives.serialization import Encoding, NoEncryption, PrivateFormat, PublicFormat
from fastapi.testclient import TestClient

from integration.loopback import BankBackend, Loopback
from llm.gateway_double import GatewayDouble
from pulso_core_runtime import main as runtime_main
from pulso_core_runtime.internal.auth import b64url_encode, sign_service_jwt
from runtime.conftest import PgDbs, pg  # noqa: F401  (fixture re-export)

WORLD = Path(__file__).resolve().parents[3] / "agent-core-assets" / "worlds" / "pulso-evolution"
TENANT = "t1"
SCOUT_OUTPUT = {"schema_version": "1", "hypotheses": [{
    "id": "h1", "statement": "retention dips on day 7", "mechanism": "onboarding gap", "evidence_refs": [],
    "counterevidence_refs": [], "missing_evidence": ["cohort split"], "next_queries": []}]}


def pub(key: Ed25519PrivateKey) -> str:
    return b64url_encode(key.public_key().public_bytes(Encoding.Raw, PublicFormat.Raw))


def seed(key: Ed25519PrivateKey) -> str:
    return b64url_encode(key.private_bytes(Encoding.Raw, PrivateFormat.Raw, NoEncryption()))


class DualGateway:
    """Double: agent steps answer `final` with the scout output; every other call is the engine's `CitingGateway`."""

    def __init__(self) -> None:
        from testing.engine_world import CitingGateway
        self._citing = CitingGateway()
        self.agent_calls = 0
        self.other_calls = 0

    def generate(self, prompt: Any, inputs_model_view: Any, locale: Any, schema: Any = None) -> Any:
        from agent_core.ports import GenerationResult
        if isinstance(inputs_model_view, dict) and "goal" in inputs_model_view:
            self.agent_calls += 1
            return GenerationResult(output={"kind": "final", "output": SCOUT_OUTPUT}, tokens_in=10, tokens_out=5,
                                    cost_usd=Decimal("0.001"), model="double-model")
        self.other_calls += 1
        return self._citing.generate(prompt, inputs_model_view, locale, schema)


class ConstProvider:
    """Double: the fixture bank's `resuelto` script as a constant (decision providers are not under test)."""

    def __init__(self, name: str) -> None:
        self.name = name

    def predict(self, spec: Any, inputs_model_view: Any, schema: Any, locale: Any) -> Any:
        from agent_core.decision import RawPrediction
        if self.name == "jev":
            return RawPrediction(value={"command": "continue"}, p_raw={"command": 0.95}, tokens=20)
        return RawPrediction(value={"match": "unica", "transaction": "tx-1"}, p_raw={"match": 0.9}, tokens=10)


@dataclass
class Composed:
    pg: PgDbs
    loop: Loopback
    dir: Path
    svc_key: Ed25519PrivateKey
    ident: Ed25519PrivateKey
    app: Any = None
    client: Any = None
    ports: Any = None
    gateway: DualGateway = field(default_factory=DualGateway)
    store: Any = None
    registry: Any = None
    exit_code: int = -1
    stderr: str = ""
    release_ids: dict[str, str] = field(default_factory=dict)

    @property
    def registry_service(self) -> Any:
        return self.ports.tools._builder_factory.__closure__ and self._svc()

    def _svc(self) -> Any:
        from agent_core.registry import RegistryService
        return RegistryService(self.store, None, None, self.ports.ids)  # read-only `get_write` use only

    @property
    def contexts(self) -> Any:
        return self.ports.tools._contexts

    def token(self, purpose: str, *, tenant: str = TENANT, aud: str = "core-bridge", **extra: Any) -> str:
        now = int(time.time())
        return sign_service_jwt(self.svc_key, kid="cp1", claims={
            "iss": "control-api", "aud": aud, "sub": "worker:1", "tenant_id": tenant, "purpose": purpose,
            "job_id": "j1", "iat": now, "exp": now + 60, "jti": uuid.uuid4().hex, **extra})

    def headers(self, purpose: str, **kw: Any) -> dict[str, str]:
        return {"Authorization": f"Bearer {self.token(purpose, **kw)}"}


def build_env(c: Composed, extra: dict[str, str] | None = None) -> dict[str, str]:
    d = c.dir
    return {"AGENTCORE_REGISTRY_DSN": c.pg.runtime, "AGENTCORE_EVAL_DSN": c.pg.eval,
            "AGENTCORE_KEYS_FINGERPRINT": "k1:" + b64url_encode(b"f" * 32).replace("-", "A").replace("_", "B") + "=",
            "AGENTCORE_KEYS_TOKEN_MAP": "k1:" + b64url_encode(b"m" * 32).replace("-", "A").replace("_", "B") + "=",
            "PULSO_IDENTITY_KEYS": str(d / "identity.json"), "PULSO_STAFF_KEYS": str(d / "staff.json"),
            "PULSO_SERVICE_KEYS": str(d / "service.json"), "PULSO_PORT": "0",
            "PULSO_BRIDGE_IDENTITY_SIGNER": str(d / "bridge-identity.json"),
            "PULSO_BRIDGE_STAFF_SIGNER": str(d / "bridge-staff.json"),
            "PULSO_BRIDGE_CALLBACK_SIGNER": str(d / "bridge-callback.json"),
            "PULSO_BRIDGE_EXECUTOR_SIGNER": str(d / "bridge-executor.json"),
            "PULSO_LAB_BROKER_URL": c.loop.url, "PULSO_CONTROL_API_URL": c.loop.url,
            "PULSO_EVAL_BUDGETS": str(d / "budgets.json"), "PULSO_EVAL_PERMITS": "1",
            "PULSO_SHA": "integ", "PULSO_TENANT_ID": TENANT,
            "AGENTCORE_LLM_GATEWAY_URL": "http://llm-gateway.test:8080", "AGENTCORE_LLM_GATEWAY_TOKEN": "tok-ok", **(extra or {})}


def boot(c: Composed, extra: dict[str, str] | None = None) -> Composed:
    from agent_core.composition.serve_ports import resolve_ports
    from agent_core.decision.calibration.artifact import InMemoryCalibrationSource
    from testing.engine_world import demo_calibration

    def resolve(args: Any, env: Any, clock: Any, ids: Any, **kw: Any) -> Any:
        ports = resolve_ports(args, env, clock, ids, **kw)
        ports = dataclasses.replace(
            ports, gateway=c.gateway,
            providers={"jev": ConstProvider("jev"), "classifier": ConstProvider("classifier")},
            calibrations=InMemoryCalibrationSource({"cal-demo": demo_calibration()}))
        c.ports = ports
        return ports

    captured: list[Any] = []
    err = io.StringIO()
    c.exit_code = runtime_main.run([], env=build_env(c, extra), stderr=err,
                                   serve=lambda app, **kw: captured.append(app), resolve=resolve,
                                   llm_probe_client=GatewayDouble().client())
    c.stderr = err.getvalue()
    assert c.exit_code == 0, c.stderr
    c.app = captured[0]
    c.client = TestClient(c.app)
    return c


@pytest.fixture
def loop() -> Iterator[Loopback]:
    lb = Loopback(BankBackend())
    try:
        yield lb
    finally:
        lb.close()


@pytest.fixture
def composed(pg: PgDbs, loop: Loopback, tmp_path: Path) -> Composed:  # noqa: F811
    try:
        from l5.world import seed_demo
        from testing.builders import principal
        from testing.fakes.clock import FakeClock
        from testing.fakes.ids import FakeIds
        from tests.registry.helpers import admin
        from tests.registry.service_world import FakeEvaluator
    except ImportError:
        pytest.skip("pin checkout test helpers not importable")
    from agent_core.domain import AgentSelector
    from agent_core.registry import PgRegistryStore, PostgresRegistry
    from agent_core.registry.service import RegistryService

    ident, staff, svc, callback, executor = (Ed25519PrivateKey.generate() for _ in range(5))
    (tmp_path / "identity.json").write_text(json.dumps(
        {"principal_keys": {"id1": pub(ident)}, "delegation_keys": {"id1": pub(ident)}}))
    (tmp_path / "staff.json").write_text(json.dumps({"principal_keys": {"st1": pub(staff)}}))
    (tmp_path / "service.json").write_text(json.dumps(
        {"keys": {"cp1": {"iss": "control-api", "aud": "core-bridge", "key": pub(svc)}}}))
    for name, kid, key in (("bridge-identity", "id1", ident), ("bridge-staff", "st1", staff),
                           ("bridge-callback", "cb1", callback), ("bridge-executor", "ex1", executor)):
        (tmp_path / f"{name}.json").write_text(json.dumps({"kid": kid, "key": seed(key)}))
    (tmp_path / "budgets.json").write_text(json.dumps(
        {"bud-1": {"cost_usd_max": "5", "tokens_max": 100000, "jobs_max": 20}}))
    c = Composed(pg=pg, loop=loop, dir=tmp_path, svc_key=svc, ident=ident)
    store = PgRegistryStore(lambda: psycopg.connect(pg.runtime, autocommit=False))
    clock = FakeClock()
    if WORLD.is_dir():
        RegistryService(store, FakeEvaluator(), clock, FakeIds()).import_seed(admin(), WORLD)
    seed_demo(store)
    c.store, c.registry = store, PostgresRegistry(store, clock)
    if WORLD.is_dir():
        for stage in ("scout", "writer"):
            c.release_ids[stage] = c.registry.resolve_release(
                AgentSelector.parse(f"pulso-{stage}@prod"),
                principal(type="builder", id="b", roles=["constructor"], attrs={})).id
    return boot(c)
