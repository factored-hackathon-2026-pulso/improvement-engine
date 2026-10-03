"""kill -9 of the bridge between a committed registry write and the Core run commit, reconciled by the COMPOSED app
(`main._compose`: production Reconciler, `ServiceWriteProbe`, `sealed_commitment_check`, receipt binding lookup).

The "dying bridge" is a real subprocess: it persists the receipt (`sent`), the frozen context with the sealed
commitment and a confirmed binding, then commits a real registry write (`create_proposal` under the derived key)
through the pinned `RegistryService` on PG16, and is SIGKILLed with no cleanup. The restarted bridge (the composed
app of the test) is re-entered with the same key: it must adopt the write only if it matches the commitment."""

from __future__ import annotations

import json
import os
import subprocess
import sys
import textwrap
from typing import Any

import pytest

from integration.conftest import WORLD, Composed
from l3a.helpers import body, idem_key
from pulso_core_runtime.invoke.models import CoreTaskInvocation, request_digest
from pulso_core_runtime.invoke.service import _commitment, _commitment_json
from pulso_core_runtime.store.receipts import ReceiptStore
from pulso_core_runtime.tools.builder import write_key

pytestmark = [pytest.mark.integration, pytest.mark.pg]

CHILD = textwrap.dedent("""
    import sys, json, psycopg
    from datetime import UTC, datetime, timedelta
    from types import SimpleNamespace
    from agent_core.adapters.system_clock import SystemClock
    from agent_core.adapters.system_ids import SystemIds
    from agent_core.registry import PgRegistryStore
    from agent_core.registry.models import Origin
    from agent_core.registry.service import RegistryService
    from pulso_core_runtime.main import _constructor_principal
    from pulso_core_runtime.store.receipts import ReceiptStore
    from pulso_core_runtime.tools.builder import write_key

    dsn, key, digest, release, commitment, written_title = sys.argv[1:7]
    s = ReceiptStore(dsn)
    s.begin(tenant_id="t1", key=key, digest=digest, stage="writer", job_id="j1", attempt=1, release_id=release,
            task_binding_ref="bind-kill", principal_id="p")
    assert s.transition("t1", key, "sent") is not None
    s.save_context("bind-kill", "t1", "j1", key, {
        "tenant_id": "t1", "job_id": "j1", "stage": "writer", "attempt": 1, "operations": ["create_proposal"],
        "evaluation_context_ref": None, "commitment": json.loads(commitment)},
        datetime.now(UTC) + timedelta(minutes=15))
    assert s.transition("t1", key, "binding_confirmed") is not None
    store = PgRegistryStore(lambda: psycopg.connect(dsn, autocommit=False))
    svc = RegistryService(store, None, SystemClock(), SystemIds())
    svc.create_proposal(_constructor_principal(SimpleNamespace(tenant_id="t1")), "atencion", Origin.builder_chat,
                        written_title, idempotency_key=write_key(key, "writer", 0))
    print("READY", flush=True)  # the registry effect is committed; the Core run commit never happens
    sys.stdin.read()
""")


def payload(composed: Composed, title: str) -> dict[str, Any]:
    return body(stage="writer", agent_id="pulso-writer", agent_version="1.0.0",
                release_id=composed.release_ids["writer"], logical="wc", memory_snapshot_ref=None,
                input={"draft_plan_ref": "artifact:plan", "proposal_id": None, "base_release_id": "rel-demo",
                       "evaluate_enabled": False},
                registry_mutation_commitment={
                    "mode": "write", "base_release_id": "rel-demo", "create_agent_id": "atencion",
                    "create_origin": "builder_chat", "create_title": title, "operations": ["create_proposal"]})


def kill9_then_reenter(composed: Composed, committed_title: str, written_title: str) -> tuple[Any, Any]:
    pl = payload(composed, committed_title)
    key = idem_key("t1", "j1", "writer", 1, "wc")
    dto = CoreTaskInvocation.model_validate(pl)
    sealed = json.dumps(_commitment_json(_commitment(dto.registry_mutation_commitment)))
    proc = subprocess.Popen(
        [sys.executable, "-c", CHILD, composed.pg.runtime, key, request_digest(pl), composed.release_ids["writer"],
         sealed, written_title], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True,
        env={**os.environ, "PYTHONPATH": os.pathsep.join(sys.path)})
    assert proc.stdout is not None and proc.stdout.readline().strip() == "READY"
    proc.kill()  # SIGKILL / TerminateProcess: no cleanup, no Core commit, no further transition
    proc.wait(timeout=10)
    store = ReceiptStore(composed.pg.runtime)
    assert store.get("t1", key).state == "binding_confirmed"  # type: ignore[union-attr]
    r = composed.client.post("/internal/v1/core-tasks/invoke", json=pl, headers={
        **composed.headers("core_task_invoke"), "Idempotency-Key": key})
    return r, store.get("t1", key)


@pytest.mark.skipif(not WORLD.is_dir(), reason="agent-core-assets world absent")
def test_kill9_after_committed_write_is_adopted_into_manual_reconcile_when_it_matches_the_commitment(
        composed: Composed) -> None:
    r, row = kill9_then_reenter(composed, "pulso-key:k9", "pulso-key:k9")
    assert r.status_code == 202, r.text
    assert row.state == "manual_reconcile" and row.reason == "adopted_writes"  # never terminal_ok, never re-run
    assert row.receipt["adopted_writes"] == [write_key(row.idempotency_key, "writer", 0)]
    assert composed.gateway.agent_calls == 0  # Core was not started again


@pytest.mark.skipif(not WORLD.is_dir(), reason="agent-core-assets world absent")
def test_kill9_with_a_write_that_breaks_the_commitment_is_flagged(composed: Composed) -> None:
    r, row = kill9_then_reenter(composed, "pulso-key:sealed", "pulso-key:SOMETHING-ELSE")
    assert r.status_code == 202, r.text
    assert (row.state, row.reason) == ("manual_reconcile", "commitment_mismatch")
    assert composed.gateway.agent_calls == 0
