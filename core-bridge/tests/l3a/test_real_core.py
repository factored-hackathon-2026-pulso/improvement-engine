"""L3a against the REAL pinned Core in-process on PG16: composed app (L2 `main`), the pulso-evolution seed
(L4 world), `POST /v1/runs` through `httpx.ASGITransport`, real identity verifier, real pin, real
idempotency. Doubles: control-api binding callback (MockTransport) and the `pulso/bind_context` handler
(stand-in for L3b `tools/bind.py`); `wiki_read` and the model are absent on purpose, so the scout run stops at
its first missing tool."""

from __future__ import annotations

import io
import json
from pathlib import Path
from typing import Any

import httpx
import pytest
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from cryptography.hazmat.primitives.serialization import Encoding, PublicFormat

from pulso_core_runtime import main as runtime_main
from pulso_core_runtime.credentials.issuer import PrincipalSigner
from pulso_core_runtime.internal.auth import b64url_encode
from pulso_core_runtime.invoke.core_client import AsgiCoreClient
from pulso_core_runtime.invoke.pin import RegistryReleaseChecker
from pulso_core_runtime.invoke.runs import PgRunReader
from pulso_core_runtime.invoke.service import InvokeService, InvokeSettings
from pulso_core_runtime.pin import PinnedRegistryPort
from pulso_core_runtime.store.migrations import apply_l3
from pulso_core_runtime.store.receipts import ReceiptStore

from .conftest import PgDbs
from .helpers import body, idem_key

pytestmark = [pytest.mark.l3a, pytest.mark.pg, pytest.mark.anyio]
WORLD = Path(__file__).resolve().parents[3] / "agent-core-assets" / "worlds" / "pulso-evolution"


def _pub(key: Ed25519PrivateKey) -> str:
    return b64url_encode(key.public_key().public_bytes(Encoding.Raw, PublicFormat.Raw))


class World:
    def __init__(self) -> None:
        self.callbacks: list[dict[str, Any]] = []
        self.callback_status = 200
        self.app: Any = None
        self.registry: Any = None
        self.service_ctl: Any = None
        self.clock: Any = None
        self.release_ids: dict[str, str] = {}


@pytest.fixture
def world(pg: PgDbs, tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> World:
    import psycopg

    from agent_core.registry import PgRegistryStore, PostgresRegistry
    from agent_core.registry.service import RegistryService
    try:
        from testing.fakes.clock import FakeClock
        from testing.fakes.ids import FakeIds
        from tests.registry.helpers import admin
        from tests.registry.service_world import FakeEvaluator
    except ImportError:
        pytest.skip("pin checkout test helpers not importable")
    if not WORLD.is_dir():
        pytest.skip("agent-core-assets world absent")
    from pulso_core_runtime.internal.store import ensure_schema
    ensure_schema(pg.runtime)
    apply_l3(pg.runtime)

    # GAP for L2 (factories.py): PulsoAuthz admits only constructor/aprobador, but non-writer stage principals
    # carry `stage_task` (plan: `constructor` only for the writer). Needs BUILDER_ROLES |= {"stage_task"}.
    from pulso_core_runtime import factories
    monkeypatch.setattr(factories, "BUILDER_ROLES", factories.BUILDER_ROLES | {"stage_task"})

    w = World()
    store = PgRegistryStore(lambda: psycopg.connect(pg.runtime, autocommit=False))
    w.clock = FakeClock()
    service = RegistryService(store, FakeEvaluator(), w.clock, FakeIds())
    service.import_seed(admin(), WORLD)
    w.registry, w.service_ctl = PostgresRegistry(store, w.clock), service

    ident = Ed25519PrivateKey.generate()
    svc = Ed25519PrivateKey.generate()
    (tmp_path / "identity.json").write_text(json.dumps({"principal_keys": {"id1": _pub(ident)},
                                                        "delegation_keys": {"id1": _pub(ident)}}))
    (tmp_path / "staff.json").write_text(json.dumps({"principal_keys": {"st1": _pub(Ed25519PrivateKey.generate())}}))
    (tmp_path / "service.json").write_text(json.dumps(
        {"keys": {"cp1": {"iss": "control-api", "aud": "core-bridge", "key": _pub(svc)}}}))
    from cryptography.hazmat.primitives.serialization import Encoding as Enc, NoEncryption, PrivateFormat
    for name, kid in (("bridge-identity", "id1"), ("bridge-staff", "st1"), ("bridge-callback", "cb1"), ("bridge-executor", "ex1")):
        seed = (ident if name == "bridge-identity" else Ed25519PrivateKey.generate()).private_bytes(
            Enc.Raw, PrivateFormat.Raw, NoEncryption())
        (tmp_path / f"{name}.json").write_text(json.dumps({"kid": kid, "key": b64url_encode(seed)}))
    w.ident = ident  # type: ignore[attr-defined]
    w.dir = tmp_path  # type: ignore[attr-defined]
    w.pg = pg  # type: ignore[attr-defined]

    w.store = ReceiptStore(pg.runtime)  # type: ignore[attr-defined]

    from integration.loopback import Loopback
    loop = Loopback()
    w.loop = loop  # type: ignore[attr-defined]
    w.binding = None  # type: ignore[attr-defined]

    def resolve(args: Any, env: Any, clock: Any, ids: Any, **kw: Any) -> Any:
        from agent_core.composition.serve_ports import resolve_ports
        ports = resolve_ports(args, env, clock, ids, **kw)  # real `pulso/bind_context` via the loopback control-api
        w.inv_registry = ports.tools._contexts  # type: ignore[attr-defined]  # the composed (shared) registry
        return ports

    env = {"AGENTCORE_REGISTRY_DSN": pg.runtime, "AGENTCORE_EVAL_DSN": pg.eval,
           "AGENTCORE_KEYS_FINGERPRINT": "k1:" + b64url_encode(b"f" * 32).replace("-", "A").replace("_", "B") + "=",
           "AGENTCORE_KEYS_TOKEN_MAP": "k1:" + b64url_encode(b"m" * 32).replace("-", "A").replace("_", "B") + "=",
           "PULSO_IDENTITY_KEYS": str(tmp_path / "identity.json"), "PULSO_STAFF_KEYS": str(tmp_path / "staff.json"),
           "PULSO_SERVICE_KEYS": str(tmp_path / "service.json"), "PULSO_PORT": "0",
           "PULSO_BRIDGE_IDENTITY_SIGNER": str(tmp_path / "bridge-identity.json"),
           "PULSO_BRIDGE_STAFF_SIGNER": str(tmp_path / "bridge-staff.json"),
           "PULSO_BRIDGE_CALLBACK_SIGNER": str(tmp_path / "bridge-callback.json"),
           "PULSO_BRIDGE_EXECUTOR_SIGNER": str(tmp_path / "bridge-executor.json"),
           "PULSO_LAB_BROKER_URL": loop.url, "PULSO_CONTROL_API_URL": loop.url}
    captured: list[Any] = []
    err = io.StringIO()
    code = runtime_main.run([], env=env, stderr=err, serve=lambda app, **kw: captured.append(app), resolve=resolve)
    assert code == 0, err.getvalue()
    w.app = captured[0]
    from agent_core.domain import AgentSelector
    from testing.builders import principal
    sel = AgentSelector.parse("pulso-scout@prod")
    w.release_ids["scout"] = w.registry.resolve_release(
        sel, principal(type="builder", id="b", roles=["constructor"], attrs={})).id
    return w


def _service(w: World, **kw: Any) -> InvokeService:
    pinned = PinnedRegistryPort(w.registry)
    return InvokeService(
        store=w.store, core=AsgiCoreClient(lambda: w.app), releases=RegistryReleaseChecker(pinned),
        signer=PrincipalSigner("id1", w.ident), runs=PgRunReader(w.pg.runtime), registry=w.inv_registry,
        settings=InvokeSettings(), **kw)


def _scout_body(w: World, **over: Any) -> dict[str, Any]:
    over.setdefault("input", {"briefing_ref": "artifact:b1"})
    over.setdefault("release_id", w.release_ids["scout"])
    return body(agent_id="pulso-scout", agent_version="1.0.0", **over)


def _k(**kw: Any) -> str:
    return idem_key(kw.get("tenant", "t1"), kw.get("job", "j1"), "scout", kw.get("attempt", 1), kw.get("logical", "k"))


async def test_real_core_run_pinned_binding_confirmed_and_idempotent(world: World) -> None:
    svc = _service(world)
    out = await svc.invoke("t1", _k(), _scout_body(world))
    # Real Core ran the real flow: bind_context confirmed, then the first absent tool stopped the stage.
    assert out.body["core_run_id"], out.body
    assert world.loop.bindings and world.loop.bindings[0]["core_run_id"] == out.body["core_run_id"]
    assert world.loop.bindings[0]["task_binding_ref"] == out.body["task_binding_ref"]
    print("REAL CORE RECEIPT", {k: out.body.get(k) for k in ("state", "outcome", "reason", "receipt")})
    assert out.body["state"] in {"terminal_failed", "terminal_ok", "manual_reconcile"}, out.body
    stored = world.store.get("t1", _k())
    assert stored is not None and stored.core_run_id == out.body["core_run_id"]
    # Same key + same body: Core's own idempotency gives the same run; the bridge replays the receipt.
    again = await svc.invoke("t1", _k(), _scout_body(world))
    assert again.body["core_run_id"] == out.body["core_run_id"] and len(world.loop.bindings) == 1
    # Same key, other digest: 409 from the bridge, still exactly one callback and one run.
    clash = await svc.invoke("t1", _k(), _scout_body(world, input={"briefing_ref": "artifact:OTHER"}))
    assert clash.status == 409 and clash.body["code"] == "pulso:digest_conflict"
    with __import__("psycopg").connect(world.pg.runtime) as conn:  # type: ignore[attr-defined]
        n = conn.execute("SELECT count(*) FROM runs").fetchone()
    assert n is not None and n[0] == 1


async def test_real_core_alias_moved_after_authorisation_still_runs_the_pinned_release(world: World) -> None:
    pinned = world.release_ids["scout"]
    svc = _service(world)
    out = await svc.invoke("t1", _k(logical="pin"), _scout_body(world, logical="pin"))
    assert out.body["core_run_id"] and out.body["state"] != "unknown"
    row = world.store.get("t1", _k(logical="pin"))
    assert row is not None and row.release_id == pinned  # RunResult.release == pinned, else release_drift


async def test_real_core_binding_denied_means_tools_denied_and_no_confirmation(world: World) -> None:
    world.loop.backend.bind_mode = "conflict"
    svc = _service(world)
    out = await svc.invoke("t1", _k(logical="den"), _scout_body(world, logical="den"))
    assert out.body["state"] != "terminal_ok"
    row = world.store.get("t1", _k(logical="den"))
    assert row is not None and row.state != "binding_confirmed"


async def test_real_core_timeout_after_effect_is_unknown_then_reconcile_adopts_without_second_run(
        world: World) -> None:
    """Fault-injection proxy: the real Core commits the run, the response is lost."""
    real = AsgiCoreClient(lambda: world.app)

    class LostResponse:
        calls = 0

        async def start_run(self, bearer: str, key: str, body_: dict[str, Any]) -> Any:
            LostResponse.calls += 1
            await real.start_run(bearer, key, body_)
            raise httpx.ReadTimeout("response lost")

    svc = _service(world)
    svc._core = LostResponse()  # type: ignore[assignment]
    first = await svc.invoke("t1", _k(logical="lost"), _scout_body(world, logical="lost"))
    assert first.status == 202 and first.body["state"] == "unknown"
    # same key again: re-read only; Core's stored RunResult for (principal, key) is terminal and wins
    second = await svc.invoke("t1", _k(logical="lost"), _scout_body(world, logical="lost"))
    assert LostResponse.calls == 1
    assert second.body["state"] == "terminal_failed" and second.body["core_run_id"]
    assert second.body["reason"] == "unexpected_outcome" and second.body["outcome"] == "escalated"  # adopted from Core
    with __import__("psycopg").connect(world.pg.runtime) as conn:  # type: ignore[attr-defined]
        n = conn.execute("SELECT count(*) FROM runs").fetchone()
    assert n is not None and n[0] == 1


async def test_real_core_pin_to_another_agents_release_never_starts_a_run(world: World) -> None:
    from agent_core.domain import AgentSelector
    from testing.builders import principal
    other = world.registry.resolve_release(
        AgentSelector.parse("pulso-writer@prod"), principal(type="builder", id="b", roles=["constructor"], attrs={})).id
    out = await _service(world).invoke("t1", _k(logical="mis"), _scout_body(world, logical="mis", release_id=other))
    assert out.status == 409 and out.body["code"] == "pulso:release_pin_unavailable"
    with __import__("psycopg").connect(world.pg.runtime) as conn:  # type: ignore[attr-defined]
        n = conn.execute("SELECT count(*) FROM runs").fetchone()
    assert n is not None and n[0] == 0 and world.loop.bindings == []
