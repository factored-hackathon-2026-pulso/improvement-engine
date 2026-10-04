"""E2E-THREAD-01 runner (Q1r): the ten demo steps on the Python host, REPLAY mode, in-process doubles only.

Pieces assembled (all already in this branch): ED0 (ed0_detect, real Rust sensor), ED0L (ed0_lab), roleplay-llm
shim (replay of recorded agent_roleplay answers), M3 stage policy and timeouts (core-bridge stages), SMAP, CMPpy,
GSIpy, P2py (platform-sim release and effect series) and the G1 engine-run report (`check()` must pass).

Steps and honest labels in replay (plan 2.1):
  1 data wakes the engine      stand-in      manual command, no trigger
  2 signals and discards       real-narrow   existing Rust sensor on a synthetic E0-shaped package
  3 scout + separate verifier  agent_roleplay  recorded answers via the shim; tool results from the ED0L lab
  4 opportunity (builder)      agent_roleplay  target chosen from the ReadBase catalogue (SMAP), never from prose
  5 concrete change            stand-in      CMPpy generic compile                         [hook: dry_run]
  6 base vs candidate, gates   stand-in      structural GSIpy verdict over stand-in arms   [hook: run_arms]
  7 failure -> revision        not_exercised unless the gate fails (then rule-driven stand-in, bounded)
  8 human only for authority   simulated     local Ed25519 issuer, bound to the draft digest
  9 staging + alias read       stand-in      in-process registry double                    [hooks: publish, alias_read]
 10 observation                simulated     platform-sim release.* and effect series; observation only

INT0 swap-in points are the fields of `CoreHooks` (each documented there). A hook that is supplied flips the
step label to `real-narrow`: the step then reports what the real Core returned, not the double.
Raw E0 is never read here: packages are SYNTHETIC and live only under `ThreadConfig.workdir` (untracked).
"""
from __future__ import annotations

import json
import os
import subprocess
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Callable

from . import ed0_detect as ed0

ROOT = Path(__file__).resolve().parents[3]
CONTRACT_REVISION = "engine-run/c2-1"
HOST = "python"
CATEGORY_LABELS = {"A": "closing_reply_unclear", "B": "followup_wording"}  # SMAP catalogue vocabulary


@dataclass
class CoreHooks:
    """INT0 swap-in points. Each is optional; None keeps the in-process double and its stand-in label.

    dry_run(operations) -> "sha256:<64 hex>"        step 5: the real Core dry-run digest of the compiled plan
    run_arms(ctx) -> {"base": runs, "candidate": runs} | None
                                                    step 6: ArmReport runs of the real Core arms (None keeps the
                                                    stand-in runs); the verdict stays the stand-in GSIpy one
    publish(ctx) -> {"release_id", "alias"}         step 9: publish to local staging on the real Core
    alias_read(ctx, alias) -> {"release_id", "alias"}  step 9: read the alias back from the real Core
    """
    dry_run: Callable | None = None
    run_arms: Callable | None = None
    publish: Callable | None = None
    alias_read: Callable | None = None


@dataclass
class ThreadConfig:
    workdir: Path
    exe: str
    queue_dir: Path
    mode: str = "replay"  # "replay" (shim replay_only) | "record" (scripted responder writes the queue)
    hooks: CoreHooks = field(default_factory=CoreHooks)
    gate_evaluators: dict | None = None


@dataclass
class Ctx:
    cfg: ThreadConfig
    sha: str
    out: dict[str, Any] = field(default_factory=dict)  # per-step outputs consumed by later steps


def _sha() -> str:
    try:
        s = subprocess.run(["git", "rev-parse", "HEAD"], cwd=ROOT, capture_output=True, text=True).stdout.strip()
    except OSError:
        s = ""
    return s if len(s) == 40 else "0" * 40


# ---- step 1 ----------------------------------------------------------------------------------------------------
def step_01(ctx: Ctx) -> dict:
    pkg = ctx.cfg.workdir / "e0_package"
    ed0.write_synthetic_e0(str(pkg), CATEGORY_LABELS)
    ctx.out["package"] = str(pkg)
    return {"status": "stand-in", "data_class": "generated_sample", "receipt": {"provider": "local"},
            "detail": {"trigger": "manual_command", "package": "synthetic E0-shaped (workdir, untracked)"}}


# ---- step 2 ----------------------------------------------------------------------------------------------------
def step_02(ctx: Ctx) -> dict:
    if not (ctx.cfg.exe and os.path.exists(ctx.cfg.exe)):
        return {"status": "blocked(sensor-exe)", "data_class": "generated_sample", "detail": {}}
    det = ed0.detect(ctx.cfg.exe, ctx.out["package"], str(ctx.cfg.workdir / "sensor_out"))
    ctx.out["detection"] = det
    return {"status": "real-narrow", "data_class": "generated_sample", "receipt": {"provider": "local"},
            "detail": {k: det[k] for k in ("producer", "admitted_family", "winner_support", "denominator",
                                           "discards", "holdout_status")}}


def step_03(ctx: Ctx) -> list[dict]:
    raise NotImplementedError("step 3")


def step_04(ctx: Ctx) -> dict:
    raise NotImplementedError("step 4")


def step_05(ctx: Ctx) -> dict:
    raise NotImplementedError("step 5")


def step_06(ctx: Ctx) -> dict:
    raise NotImplementedError("step 6")


def step_07(ctx: Ctx) -> dict:
    raise NotImplementedError("step 7")


def step_08(ctx: Ctx) -> dict:
    raise NotImplementedError("step 8")


def step_09(ctx: Ctx) -> dict:
    raise NotImplementedError("step 9")


def step_10(ctx: Ctx) -> dict:
    raise NotImplementedError("step 10")


STEPS: list[tuple[int, str, Callable]] = [
    (1, "trigger", step_01), (2, "signals", step_02), (3, "scout", step_03), (4, "opportunity", step_04),
    (5, "compile", step_05), (6, "gate", step_06), (7, "revision", step_07), (8, "approval", step_08),
    (9, "publish", step_09), (10, "observation", step_10),
]


def _finish(n: int, sid: str, rec: dict, ctx: Ctx) -> dict:
    return {"n": n, "id": rec.get("id", sid), "target": "local", "sha": ctx.sha,
            "contract_revision": CONTRACT_REVISION, "host": HOST, **{k: v for k, v in rec.items() if k != "id"}}


def run_thread(cfg: ThreadConfig) -> dict:
    cfg.workdir = Path(cfg.workdir)
    cfg.workdir.mkdir(parents=True, exist_ok=True)
    ctx = Ctx(cfg, _sha())
    steps: list[dict] = []
    for n, sid, fn in STEPS:
        try:
            res = fn(ctx)
            recs = res if isinstance(res, list) else [res]
            steps += [_finish(n, sid, r, ctx) for r in recs]
        except Exception as e:  # noqa: BLE001 - a step that cannot run is RED, never silently skipped
            steps.append(_finish(n, sid, {"status": "red", "error": f"{type(e).__name__}: {e}", "data_class": None,
                                          "detail": {}}, ctx))
    return {"steps": steps, "mode": cfg.mode, "host": HOST, "replay": ctx.out.get("replay", {"misses": 1, "calls": 0}),
            "report": ctx.out.get("report", {}), "m3": ctx.out.get("m3", {}), "ctx": ctx.out}
