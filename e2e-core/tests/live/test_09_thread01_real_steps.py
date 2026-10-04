"""INT0 live: E2E-THREAD-01 with real-Core hooks for steps 5, 6, 8 and 9 (and the frozen proposal of the thread's own
draft) against the real_local stack. Doubles stay labelled: the control-api broker/lab/bank (`e2e-fixtures`), the local
human issuer (internal-only container), the roleplay shim (steps 3-4), the stand-in GSIpy verdict judge, platform-sim.

Each parametrized case is one live "window" (a full thread run); seconds per live call are logged through `effect`.
Runs after test_07 (alphabetical order): it publishes a release of `atencion` to staging (prod is never promoted)."""
from __future__ import annotations

import json
import os
import time
from typing import Any

import httpx
import pytest
from claude_standin import compile_step as C
from claude_standin import core_hooks as H
from claude_standin import thread01 as T
from codex_standin.engine import TENANT
from codex_standin.human_port import HumanAuthorizationPort, IntentionStore, PodmanExecTransport
from codex_standin.stack import CONNECTION, PODMAN, podman, project
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from local_identity.client import LocalIdentityClient, ServiceSigner
from local_identity.keys import b64url_decode

pytestmark = pytest.mark.live
EXE = os.environ.get("ED0_RUNNER_EXE", "D:/cargo-targets/claude-ed0/debug/improvement-engine.exe")
AGENT = "atencion"
WINDOWS: list[dict[str, Any]] = []
BLOCKED_GATE = "blocked(jev): Core approval and publish need an `evaluated` proposal; its native evaluation of the atencion suite ends failed_infra"
BLOCKED_6 = "blocked(jev: the Core composes no `jev` decision provider for the atencion world; agent-core PR 28 not on main)"


@pytest.fixture(scope="module")
def authorizer(stack: Any, tmp_path_factory: pytest.TempPathFactory) -> Any:
    container = f"{project(stack.env['namespace'])}-human-issuer-1"
    seed = json.loads(podman("exec", container, "cat", "/run/hi-keys/control-api-signer.json"))
    signer = ServiceSigner(seed["kid"], Ed25519PrivateKey.from_private_bytes(b64url_decode(seed["key"])))
    http = httpx.Client(transport=PodmanExecTransport(CONNECTION, container, podman=PODMAN), base_url="http://human-issuer:8083")
    client = LocalIdentityClient("http://human-issuer:8083", signer, tenant_id=TENANT, http_client=http)
    port = HumanAuthorizationPort(IntentionStore(tmp_path_factory.mktemp("t01-intentions") / "i.sqlite3"), client)
    return H.make_human_authorizer(port, tenant=TENANT)


@pytest.mark.parametrize("window", [1, 2, 3])
def test_thread01_frozen_draft_and_the_jev_blocked_gate_against_core(stack: Any, authorizer: Any, tmp_path: Any, effect: Any,
                                                                     window: int) -> None:
    """One live window: the thread with the real dry-run (step 5), then the thread's own draft frozen by the real writer
    stage (digest == the dry-run digest), the native evaluation (blocked: failed_infra) and the human approve/publish
    attempts (refused by Core, staging unmoved). Steps 6, 8, 9 stay stand-in with the blocker named."""
    world = C.load_world(T.WORLD_FILE)
    rc = H.RealCore(engine=stack.engine, bridge=stack.bridge, registry=H.make_registry(stack.runtime), authorize=authorizer,
                    world=world, tenant=TENANT, agent_id=AGENT)
    t0 = time.time()
    hooks = T.CoreHooks(dry_run=rc.dry_run, blocked={6: BLOCKED_6, 8: BLOCKED_GATE, 9: BLOCKED_GATE})
    res = T.run_thread(T.ThreadConfig(workdir=tmp_path, exe=EXE, queue_dir=T.ROOT / "e2e-core/tests/fixtures/thread01_queue",
                                      hooks=hooks))
    steps = {s["n"]: s for s in res["steps"]}
    rows = "; ".join(f"{s['n']}:{s['status']}:{s.get('error')}" for s in res["steps"])
    assert all(s["status"] != "red" for s in res["steps"]), rows
    assert steps[5]["status"] == "real-narrow" and steps[6]["status"] == "stand-in"
    assert steps[8]["status"] == "simulated" and steps[9]["status"] == "stand-in", rows
    ctx = type("C", (), {"out": res["ctx"]})()
    fz = rc.freeze(ctx)  # the real writer froze THIS thread's draft
    assert "sha256:" + fz.candidate_hash == steps[5]["detail"]["draft_plan"]["digest"]
    probe = rc.gate_probe(ctx)
    WINDOWS.append({"window": window, "seconds_total": round(time.time() - t0, 2), "calls": rc.timings})
    effect("thread01_windows", WINDOWS)
    effect(f"thread01_window_{window}_gate", {**probe, "proposal_id": fz.proposal_id, "candidate_hash": fz.candidate_hash})
    assert probe["evaluation"] == "failed_infra", probe  # no `jev` provider: the native evaluation cannot run
    assert probe["approve"][0] == 409 and probe["publish"][0] == 409 and probe["staging_unchanged"] is True, probe


@pytest.mark.parametrize("profile", ["evolution_task", "attention_stateful_complementary", None])
def test_step_6_core_arms_on_the_atencion_world_are_blocked_by_the_missing_jev_provider(stack: Any, authorizer: Any,
                                                                                     tmp_path: Any, effect: Any,
                                                                                     profile: str | None) -> None:
    """Documents the blocker: the real Core cannot run atencion arms (its decision models use provider `jev`). When this
    test starts failing, JEV landed: wire `rc.hooks()` (arms=True) in the thread test and flip step 6."""
    world = C.load_world(T.WORLD_FILE)
    rc = H.RealCore(engine=stack.engine, bridge=stack.bridge, registry=H.make_registry(stack.runtime), authorize=authorizer,
                    world=world, tenant=TENANT, agent_id=AGENT, arm_profile=profile)
    res = T.run_thread(T.ThreadConfig(workdir=tmp_path, exe=EXE, queue_dir=T.ROOT / "e2e-core/tests/fixtures/thread01_queue",
                                      hooks=T.CoreHooks(dry_run=rc.dry_run)))
    with pytest.raises(RuntimeError, match="failed_infra.*DecisionConfigError"):
        rc.run_arms(type("C", (), {"out": res["ctx"]})())
    effect(f"arm_blocked_{profile}", [t for t in rc.timings if t["call"].startswith("arm_run")][0])
