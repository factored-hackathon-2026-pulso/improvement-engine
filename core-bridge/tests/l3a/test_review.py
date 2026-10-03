"""Adversarial-review regressions for L3a: concurrency, in-flight re-entry, budget cap, credentials, kill -9."""

from __future__ import annotations

import asyncio
import os
import subprocess
import sys
import textwrap
from concurrent.futures import ThreadPoolExecutor
from typing import Any

import pytest

from pulso_core_runtime.invoke.core_client import CoreResponse
from pulso_core_runtime.store.migrations import apply_l3
from pulso_core_runtime.store.receipts import ReceiptStore

from .conftest import PgDbs
from .fakes import FakeCore
from .helpers import body, build_service, idem_key

pytestmark = [pytest.mark.l3a, pytest.mark.pg, pytest.mark.anyio]


@pytest.fixture
def dsn(pg: PgDbs) -> str:
    from pulso_core_runtime.internal.store import ensure_schema
    ensure_schema(pg.runtime)
    apply_l3(pg.runtime)
    return pg.runtime


K = idem_key("t1", "j1", "scout", 1, "k")


def _begin(store: ReceiptStore, digest: str = "d" * 64) -> bool:
    return store.begin(tenant_id="t1", key=K, digest=digest, stage="scout", job_id="j1", attempt=1,
                       release_id="rel-1", task_binding_ref="r", principal_id="p")[1]


def test_begin_single_flight_under_threads(dsn: str) -> None:
    store = ReceiptStore(dsn)
    with ThreadPoolExecutor(16) as ex:
        created = list(ex.map(lambda _: _begin(store), range(16)))
    assert created.count(True) == 1


def test_cas_has_exactly_one_winner_per_edge(dsn: str) -> None:
    store = ReceiptStore(dsn)
    _begin(store)
    with ThreadPoolExecutor(16) as ex:
        won = list(ex.map(lambda _: store.transition("t1", K, "sent") is not None, range(16)))
    assert won.count(True) == 1
    # terminal wins exactly once and is final
    with ThreadPoolExecutor(8) as ex:
        res = list(ex.map(lambda i: store.transition("t1", K, "terminal_ok" if i % 2 else "terminal_failed"),
                          range(8)))
    assert sum(r is not None for r in res) == 1
    assert store.transition("t1", K, "unknown") is None and store.transition("t1", K, "manual_reconcile") is None


def test_budget_spend_never_exceeds_cap_under_threads(dsn: str) -> None:
    store = ReceiptStore(dsn)
    with ThreadPoolExecutor(32) as ex:
        ok = list(ex.map(lambda _: store.meter_spend("t1", "j1", "scout", 1, cost_usd="0.30", cap_usd="2.00"),
                         range(32)))
    row = store.meter_get("t1", "j1", "scout", 1)
    assert ok.count(True) == 6 and row is not None and float(row["cost_usd"]) == pytest.approx(1.8)
    assert store.meter_spend("t1", "j2", "scout", 1, cost_usd="5", cap_usd="2") is False  # first spend over cap
    assert store.meter_get("t1", "j2", "scout", 1) is None


class GatedCore(FakeCore):
    def __init__(self) -> None:
        super().__init__()
        self.gate = asyncio.Event()
        self.entered = asyncio.Event()

    async def start_run(self, bearer: str, key: str, body: dict[str, Any]) -> CoreResponse:
        self.entered.set()
        await self.gate.wait()
        return await super().start_run(bearer, key, body)


async def test_same_key_retry_while_first_call_is_live_does_not_poison_it(dsn: str) -> None:
    core = GatedCore()
    svc, _ = build_service(dsn, core)
    first = asyncio.create_task(svc.invoke("t1", K, body()))
    await core.entered.wait()
    dup = await svc.invoke("t1", K, body())
    assert dup.status == 202 and dup.body["state"] == "sent"
    assert ReceiptStore(dsn).get("t1", K).state == "sent"  # type: ignore[union-attr]
    core.gate.set()
    done = await first
    assert done.status == 200 and done.body["state"] == "terminal_ok" and len(core.start_calls) == 1


async def test_concurrent_same_key_invokes_start_one_run(dsn: str) -> None:
    core = GatedCore()
    svc, _ = build_service(dsn, core)
    tasks = [asyncio.create_task(svc.invoke("t1", K, body())) for _ in range(10)]
    await core.entered.wait()
    await asyncio.sleep(0.2)
    core.gate.set()
    outs = await asyncio.gather(*tasks)
    assert len(core.start_calls) == 1 and sum(o.body["state"] == "terminal_ok" for o in outs) >= 1


def test_credential_issue_requires_tenant_claim() -> None:
    from pulso_core_runtime.credentials.issuer import CredentialIssuer, PrincipalSigner
    from pulso_core_runtime.invoke.errors import BridgeError
    from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey

    iss = CredentialIssuer({"identity": PrincipalSigner("i", Ed25519PrivateKey.generate())})
    with pytest.raises(BridgeError) as e:
        iss.issue(claims_tenant=None, tenant_id="t1", role="constructor", purpose="core_task")
    assert e.value.status == 403


CHILD = textwrap.dedent("""
    import sys, psycopg
    from pulso_core_runtime.store.receipts import ReceiptStore
    dsn, key, dig = sys.argv[1], sys.argv[2], sys.argv[3]
    s = ReceiptStore(dsn)
    s.begin(tenant_id="t1", key=key, digest=dig, stage="writer", job_id="j1", attempt=1, release_id="rel-1",
            task_binding_ref="r", principal_id="p")
    assert s.transition("t1", key, "sent") is not None
    conn = psycopg.connect(dsn)  # the "registry write" + idempotency record: uncommitted when killed
    conn.execute("CREATE TABLE IF NOT EXISTS pulso_bridge.fake_effect (k text primary key)")
    conn.commit()
    conn.execute("INSERT INTO pulso_bridge.fake_effect VALUES (%s)", (key,))
    print("READY", flush=True)
    sys.stdin.read()
""")


async def test_kill9_between_write_and_commit_reenters_without_rerun(dsn: str) -> None:
    from pulso_core_runtime.invoke.models import request_digest
    K = idem_key("t1", "j1", "writer", 1, "k")
    b = body(stage="writer", agent_id="pulso-writer")
    proc = subprocess.Popen([sys.executable, "-c", CHILD, dsn, K, request_digest(b)], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                            text=True,
                            env={**os.environ, "PYTHONPATH": os.pathsep.join(sys.path)})
    assert proc.stdout is not None and proc.stdout.readline().strip() == "READY"
    proc.kill()  # SIGKILL / TerminateProcess: no cleanup, txn never commits
    proc.wait(timeout=10)
    import psycopg
    with psycopg.connect(dsn) as c:
        assert c.execute("SELECT count(*) FROM pulso_bridge.fake_effect").fetchone()[0] == 0  # rolled back
    svc, core = build_service(dsn)
    out = await svc.invoke("t1", K, b)
    assert core.start_calls == []  # never re-executes
    assert out.status == 202 and out.body["state"] in ("manual_reconcile", "unknown")
    assert ReceiptStore(dsn).get("t1", K).state != "terminal_ok"  # type: ignore[union-attr]
