"""INT0 live: E2E-THREAD-01 with real-Core hooks for steps 5, 6, 8 and 9 (and the frozen proposal of the thread's own
draft) against the real_local stack, over the seeded base world `attention-task` (task agent `atencion-tarea`, no `jev`).
Doubles stay labelled: the control-api broker/lab/bank (`e2e-fixtures`), the scripted llm-gateway double (it answers the
agent's closing reply), the local human issuer (internal-only container), the roleplay shim (steps 3-4), the stand-in GSIpy
verdict judge, platform-sim.

Each parametrized case is one live "window" (a full thread run); seconds per live call are logged through `effect`.
Runs after test_07 (alphabetical order): it publishes releases of `atencion-tarea` to staging (prod is never promoted)."""
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
AGENT = "atencion-tarea"
WINDOWS: list[dict[str, Any]] = []
# The closing reply is the only model call of the seeded agent: one scripted answer for any prompt text (no PII, no digits).
RESPOND_RULE = {"id": "t01-respond", "match": {}, "repeat_last": True,
                "responses": [{"text": "Recibimos tu disputa y la estamos revisando.", "citations": []}]}
PROFILE = "evolution_task"  # native arms: LocalSandbox with the scenario's scripted tools, no bank world


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
def test_thread01_steps_5_6_8_9_are_real_narrow_against_core(stack: Any, authorizer: Any, tmp_path: Any, effect: Any,
                                                             window: int) -> None:
    """One live window: the whole thread with the real dry-run (5), Core arms (6, verdict judge stays the GSIpy
    stand-in), Core native evaluation + human-JWS approval verified by Core (8) and publish to staging with the alias
    read after publish (9). The proposal is the thread's own draft (hash equals the dry-run digest)."""
    stack.engine.configure(llm_replace=True, llm_rules=[RESPOND_RULE])
    world = C.load_world(T.WORLD_FILE)
    rc = H.RealCore(engine=stack.engine, bridge=stack.bridge, registry=H.make_registry(stack.runtime), authorize=authorizer,
                    world=world, tenant=TENANT, agent_id=AGENT, arm_profile=PROFILE)
    t0 = time.time()
    res = T.run_thread(T.ThreadConfig(workdir=tmp_path, exe=EXE, queue_dir=T.ROOT / "e2e-core/tests/fixtures/thread01_queue",
                                      hooks=rc.hooks()))
    steps = {s["n"]: s for s in res["steps"]}
    rows = "; ".join(f"{s['n']}:{s['status']}:{s.get('error')}" for s in res["steps"])
    WINDOWS.append({"window": window, "seconds_total": round(time.time() - t0, 2), "calls": rc.timings})
    effect("thread01_windows", WINDOWS)
    assert all(s["status"] != "red" for s in res["steps"]), rows
    assert [steps[n]["status"] for n in (5, 6, 8, 9)] == ["real-narrow"] * 4, rows
    fz = rc.freeze(type("C", (), {"out": res["ctx"]})())
    assert "sha256:" + fz.candidate_hash == steps[5]["detail"]["draft_plan"]["digest"]
    assert steps[8]["detail"]["verified_by"] == "core" and steps[8]["detail"]["tampered_rejected"] is True
    pub = steps[9]["detail"]
    assert pub["alias_read"]["release_id"] == pub["published"]["release_id"] and pub["registry"] == "core"
    assert pub["published"]["release_id"] != rc.base_release()  # staging moved to a new release; prod untouched
    assert rc.alias_read_raw("prod") == rc.base_release()
    effect(f"thread01_window_{window}_gate", {"gate_verdict": res["gate_verdict"], "gates": res["ctx"]["gate"]["gates"],
                                              "proposal_id": fz.proposal_id, "release_id": pub["published"]["release_id"],
                                              "native_evaluation": rc.evaluate(None).get("verdict")})
