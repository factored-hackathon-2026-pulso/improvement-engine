"""INT0 live: the E2E-THREAD-01 runner with real-Core hooks (dry_run, alias_read) against the real_local stack.
Read-only against Core (dry-run writes nothing). run_arms / publish stay stand-in (see claude_standin/core_hooks.py)."""
from __future__ import annotations

import os
import time
from typing import Any

import pytest
from claude_standin import compile_step as C
from claude_standin import core_hooks as H
from claude_standin import thread01 as T
from codex_standin.engine import TENANT

pytestmark = pytest.mark.live
EXE = os.environ.get("ED0_RUNNER_EXE", "D:/cargo-targets/claude-ed0/debug/improvement-engine.exe")


def test_thread01_with_real_core_dry_run_and_alias_read(stack: Any, tmp_path: Any, effect: Any) -> None:
    world = C.load_world(T.WORLD_FILE)
    ar = H.make_alias_read(stack.bridge, tenant=TENANT, agent_id="atencion")
    t0 = time.time()
    base = ar(None, "prod")["release_id"]
    hooks = T.CoreHooks(dry_run=H.make_dry_run(stack.bridge, world, tenant=TENANT, agent_id="atencion",
                                               base_release_id=base))
    res = T.run_thread(T.ThreadConfig(workdir=tmp_path, exe=EXE, queue_dir=T.ROOT / "e2e-core/tests/fixtures/thread01_queue",
                                      hooks=hooks))
    steps = {s["n"]: s for s in res["steps"]}
    effect("thread01_core_dry_run", {"step5": steps[5]["status"], "seconds": round(time.time() - t0, 2),
                                     "digest": steps[5]["detail"]["draft_plan"]["digest"],
                                     "staging": ar(None, "staging")["release_id"],
                                     "doubles": [d.get("piece") for d in res["report"]["doubles"]]})
    assert steps[5]["status"] == "real-narrow" and steps[5]["receipt"]["provider"] == "core-dry-run"
    assert all(s["status"] != "red" for s in res["steps"]), [(s["n"], s.get("error")) for s in res["steps"]]
    assert steps[6]["status"] == "stand-in" and steps[9]["status"] == "stand-in"
