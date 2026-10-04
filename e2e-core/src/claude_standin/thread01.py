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
import shutil
import subprocess
import sys
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Callable

from . import ed0_detect as ed0
from . import compile_step as cmp
from . import ed0_lab as lab
from . import gate_step as gate
from . import smap

ROOT = Path(__file__).resolve().parents[3]
WORLD_FILE = ROOT / "agent-core-assets" / "worlds" / "seeded-base.world.yaml"
CONTRACT_REVISION = "engine-run/c2-1"
HOST = "python"
CATEGORY_LABELS = {"A": "closing_reply_unclear", "B": "followup_wording"}  # SMAP catalogue vocabulary
# synthetic treated cases behind the lab: label -> (cases, recurring cases). Same shape as ed0's package (A larger).
LAB_SHAPE = {"closing_reply_unclear": (40, 22), "followup_wording": (25, 12)}
LAB_SALT = b"thread01-synthetic-salt"
FAMILY_ID = "e0_recurring_copilot_query_cases"
LAB_TOOL = "pulso/lab_query@1.0.0"
INVOKE_TIMEOUT_S = 1800.0  # CLT0: live DEMO-0 Core invoke timeout
HOLD_S = 55.0
RESPONDERS = {"scout": "responder-scout", "verifier": "responder-verifier", "builder_design": "responder-builder"}
SYSTEMS = {
    "scout": "You are the scout. Query the treated lab, then answer with hypotheses that cite evidence refs.",
    "verifier": "You are the verifier. Check each hypothesis against the treated lab and assess it.",
    "builder_design": "You are the builder. Choose among the offered candidates or do nothing; cite evidence refs.",
}


def _ensure_paths() -> None:
    for rel in ("roleplay-llm", "core-bridge/src", "platform-sim", "platform-contract"):
        p = str(ROOT / rel)
        if p not in sys.path:
            sys.path.insert(0, p)


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


# ---- the model path: agent_roleplay through the shim (replay) ---------------------------------------------------
def _lab_cases():
    i = 0
    for label, (n, hits) in LAB_SHAPE.items():
        for j in range(n):
            i += 1
            yield (f"private-case-{i}", label, "w1", j < hits)


def _lab_tool(db, args: dict) -> dict:
    return lab.lab_query(db, args["metric_id"], args["window_id"])


def scripted_responder(stage: str, inputs: dict) -> dict:
    """The role-played answer (deterministic). It sees only the treated, scanner-passed request, like a real responder."""
    if stage in ("scout", "verifier") and inputs["step"] == 1:
        return {"kind": "tool_call", "tool": LAB_TOOL, "args": {"metric_id": lab.METRIC, "window_id": "w1"}}
    if stage == "scout":
        rows = inputs["observations"][-1]["result"]["rows"]
        best = max(rows, key=lambda r: (r["rate"], r["evidence_ref"]))
        return {"kind": "final", "output": {"hypotheses": [
            {"hypothesis_id": "h_1", "evidence_ref": best["evidence_ref"], "rate": best["rate"], "count": best["count"]}]}}
    if stage == "verifier":
        return {"kind": "final", "output": {"assessments": [
            {"hypothesis_id": h["hypothesis_id"], "evidence_ref": h["evidence_ref"], "verdict": "supported"}
            for h in inputs["inputs"]["hypotheses"]]}}
    if stage == "builder_design":
        cands = inputs["inputs"]["candidates"]
        return {"kind": "final", "output": {"design_intent": {"verdict": "linked", "target_ref": cands[0]["target_ref"]},
                                            "evidence_refs": inputs["inputs"]["evidence_refs"],
                                            "alternatives": [{"kind": "do_nothing"}]}}
    raise ValueError(f"no scripted answer for {stage}")


class LLMDouble:
    """Chat-completions caller over the roleplay shim. replay: shim.replay_only on a copy of the recorded queue.
    record: the scripted responder writes the answer file first (what a live responder lane does), then the shim serves it."""

    def __init__(self, cfg: ThreadConfig):
        _ensure_paths()
        from roleplay_llm import shim as S
        self.S, self.mode = S, cfg.mode
        if cfg.mode == "replay":
            self.queue = cfg.workdir / "queue"
            if self.queue.exists():
                shutil.rmtree(self.queue)
            shutil.copytree(cfg.queue_dir, self.queue)
            self.shim = S.Shim(self.queue, replay_only=True)
        else:
            self.queue = Path(cfg.queue_dir)
            self.shim = S.Shim(self.queue, hold_s=1.0, poll_s=0.02)
        self.calls = 0
        self.misses = 0
        self.scanner_ids: set[str] = set()

    def step(self, stage: str, inputs: dict) -> dict:
        system = SYSTEMS[stage]
        if self.mode == "record":
            key = self.S.replay_key(system, inputs)
            doc = {"protocol": self.S.PROTOCOL, "key": key, "provenance": "agent_roleplay",
                   "quality_claims": "forbidden", "responder": {"id": RESPONDERS[stage], "role": stage},
                   "content": scripted_responder(stage, inputs)}
            path = self.queue / "responses" / f"{key}.json"
            path.write_text(json.dumps(doc, indent=1, sort_keys=True), encoding="utf-8", newline="\n")
        body = json.dumps({"model": "external-reasoning-model", "temperature": 0, "max_tokens": 4000,
                           "messages": [{"role": "system", "content": system},
                                        {"role": "user", "content": json.dumps(inputs, sort_keys=True,
                                                                               separators=(",", ":"))}]}).encode()
        status, resp = self.shim.handle(body)
        self.calls += 1
        if status != 200:
            self.misses += 1
            raise RuntimeError(f"{stage}: shim answered {status} {resp.get('error', {}).get('type')}")
        self.scanner_ids.add("tps-1")
        return json.loads(resp["choices"][0]["message"]["content"])


def agent_loop(llm: LLMDouble, stage: str, goal: str, inputs: dict, db: str, cap: int) -> tuple[dict, int]:
    """One agent stage: tool calls executed against the treated lab until `final`, at most `cap` calls (M3 STEP_CAPS)."""
    tools = [{"tool": LAB_TOOL, "description": "Query treated k-anonymous aggregates.",
              "args_schema": {"type": "object"}}]
    obs: list[dict] = []
    for step in range(1, cap + 1):
        out = llm.step(stage, {"goal": goal, "inputs": inputs, "step": step, "tools": tools, "observations": obs,
                               "feedback": None, "output_schema": {"type": "object"}})
        if out["kind"] == "final":
            return out["output"], step
        if out["kind"] != "tool_call" or out["tool"] != LAB_TOOL:
            raise RuntimeError(f"{stage}: tool outside the stage allow-list: {out.get('tool')}")
        obs.append({"tool": out["tool"], "args": out["args"], "status": "ok",
                    "result": _lab_tool(db, out["args"]), "error": None})
    raise RuntimeError(f"{stage}: exceeded the {cap}-call cap without a final answer")


def _m3() -> dict:
    """M3 stage policy and timeout plan evaluated on the registry profile the Flows reference."""
    _ensure_paths()
    from pulso_core_runtime.stages import policy as P
    from pulso_core_runtime.stages.catalog import STEP_CAPS
    from pulso_core_runtime.stages.timeouts import timeout_plan
    prof = P.load_registry_profile(ROOT / "agent-core-assets" / "worlds" / "pulso-evolution" / "model_profiles"
                                   / "pulso-evolution-structured@1.0.0.yaml")
    pol = P.evolution_policy(prof)
    plan = timeout_plan(profile_timeout_s=prof.timeout_s, invoke_timeout_s=INVOKE_TIMEOUT_S, hold_s=HOLD_S)
    return {"caps": dict(STEP_CAPS), "timeout_problems": list(plan.problems), "worst_case_s": plan.worst_case_s,
            "pin_problems": [p for p in (pol.check(s, prof) for s in STEP_CAPS) if p]}


def _llm(ctx: Ctx) -> LLMDouble:
    if "llm" not in ctx.out:
        ctx.out["llm"] = LLMDouble(ctx.cfg)
        ctx.out["lab_db"] = lab.build_lab(ctx.cfg.workdir / "lab.sqlite", _lab_cases(), LAB_SALT)
        ctx.out["m3"] = _m3()
    return ctx.out["llm"]


def _role(stage: str, **extra) -> dict:
    return {"status": "agent_roleplay", "data_class": "generated_sample", "actor": f"pulso-{stage}",
            "model": f"agent_roleplay:{RESPONDERS[stage]}", "stage_output": {"source": "model"},
            "receipt": {"provider": "agent_roleplay", "scanner_id": "tps-1"}, **extra}


def step_03(ctx: Ctx) -> list[dict]:
    llm, db, caps = _llm(ctx), ctx.out["lab_db"], ctx.out["m3"]["caps"]
    out, calls = agent_loop(llm, "scout", "Find the largest recurrence in the treated lab.",
                            {"family_id": FAMILY_ID, "binding_id": "binding-scout-0001"}, db, caps["scout"])
    hyps = out["hypotheses"]
    scout = _role("scout", detail={"calls": calls, "hypotheses": len(hyps)})
    v_out, v_calls = agent_loop(llm, "verifier", "Verify the hypotheses against the treated lab.",
                                {"family_id": FAMILY_ID, "binding_id": "binding-verifier-0001", "hypotheses": hyps},
                                db, caps["verifier"])
    # the verifier's verdict is only accepted if the independent ED0L recompute agrees (never from model prose)
    recompute = {h["hypothesis_id"]: lab.verify_claim(db, h, LAB_SALT) for h in hyps}
    supported = [a["evidence_ref"] for a in v_out["assessments"] if a["verdict"] == "supported"
                 and recompute[a["hypothesis_id"]]["ok"]]
    ctx.out["verified_refs"] = supported
    ctx.out["hypotheses"] = hyps
    ver = _role("verifier", id="verifier", detail={"calls": v_calls, "recompute_ok": bool(supported) and all(
        r["ok"] for r in recompute.values()), "supported_refs": supported})
    ctx.out["replay"] = {"misses": llm.misses, "calls": llm.calls}
    return [{**scout, "id": "scout"}, ver]


def _enc(ref: str) -> str:
    """Scanner-safe boundary encoding of a catalogue ref ('@' is not an opaque-id character)."""
    return ref.replace("@", "__at__")


def _dec(ref: str) -> str:
    return ref.replace("__at__", "@")


def _label_refs() -> dict[str, str]:
    return {label: lab._ref(lab.METRIC, "w1", lab.group_hash(LAB_SALT, lab.GROUP_FIELD, label)) for label in LAB_SHAPE}


def step_04(ctx: Ctx) -> dict:
    llm, db, caps = _llm(ctx), ctx.out["lab_db"], ctx.out["m3"]["caps"]
    world = cmp.load_world(WORLD_FILE)
    catalogue = smap.catalogue_from_world(world)
    refs = _label_refs()
    by_ref = {v: k for k, v in refs.items()}
    verified = ctx.out["verified_refs"]
    if not verified:
        raise RuntimeError("no verified hypothesis: the builder does not run on unverified evidence")
    categories = {label: lab._fetch(db, ref)[4] for label, ref in refs.items()}  # label -> support (lab numerator)
    finding = {"finding_ref": "finding_1", "category": by_ref[verified[0]], "evidence_refs": sorted(verified)}
    ctx.out.update(world=world, catalogue=catalogue, finding=finding, categories=categories, mapper=smap.winning_category)
    if ctx.out["detection"]["winner_support"] != categories[smap.winning_category(categories)]:
        raise RuntimeError("lab categories disagree with the sensor's winner support")
    di = smap.design_input(finding, catalogue)
    inputs = {"binding_id": "binding-builder-0001", "finding_ref": di["finding_ref"], "category": di["category"],
              "evidence_refs": di["evidence_refs"],
              "candidates": [{"target_ref": _enc(c["target_ref"]), "op": c["op"]} for c in di["candidates"]]}
    out, calls = agent_loop(llm, "builder_design", "Design one change for the finding, or do nothing.", inputs, db,
                            caps["builder_design"])
    di_out = dict(out["design_intent"])
    if di_out.get("target_ref"):
        di_out["target_ref"] = _dec(di_out["target_ref"])
    res = smap.classify({"design_intent": di_out, "evidence_refs": out["evidence_refs"]}, finding, catalogue, world)
    ctx.out["design"] = res
    ctx.out["replay"] = {"misses": llm.misses, "calls": llm.calls}
    return _role("builder_design", id="opportunity", detail={
        "calls": calls, "smap": res, "category": finding["category"],
        "do_nothing_considered": any(a.get("kind") == "do_nothing" for a in out.get("alternatives", []))})


SUITE_SEALED_AT = "2026-10-04T12:00:00Z"  # the suite (world slot) is sealed before any candidate exists
CANDIDATE_CREATED_AT = "2026-10-04T12:00:10Z"


def step_05(ctx: Ctx) -> dict:
    world, catalogue, design = ctx.out["world"], ctx.out["catalogue"], ctx.out["design"]
    if design["verdict"] != "valid" or not design["target"]:
        raise RuntimeError(f"no compilable design: {design['verdict']}")
    entries = {e["target_ref"]: e for e in catalogue["entries"]}
    suite = next(e for e in catalogue["entries"] if e["target_kind"] == "eval_suite")
    ops = [{"op": e["op"], "target_kind": e["target_kind"], "target_ref": e["target_ref"], "new_ref": e["new_ref"],
            "precondition_digest": cmp.asset_digest(world, e["target_ref"])} for e in (entries[design["target"]], suite)]
    doc = {"contract_version": "engine-steps/0", "step": "compile", "run_id": "run-thread01-0001",
           "data_class": "synthetic", "base_bundle_ref": "bundle:attention-demo@1",
           "change_spec": {"base_bundle_ref": "bundle:attention-demo@1", "opportunity_ref": "opportunity:thread01@1",
                           "workflow_bridge_ref": "bridge:disputa-cargo@1", "operations": ops,
                           "expected_mechanism": "recorded synthetic", "affected_routes": ["disputa-cargo"],
                           "rollback_ref": "bundle:attention-demo@1"}}
    hook = ctx.cfg.hooks.dry_run
    out = cmp.compile_change_spec(doc, world, dry_run=hook)
    if out["status"] != "compiled":
        raise RuntimeError(f"compile denied: {out.get('denied_reason')}")
    ctx.out["compiled"] = out
    ctx.out["candidate_created_at"] = CANDIDATE_CREATED_AT
    return {"status": "real-narrow" if hook else "stand-in", "data_class": "synthetic",
            "receipt": {"provider": "core-dry-run" if hook else "claude-standin"},
            "detail": {"compiler_label": out["compiler_label"], "draft_plan": out["draft_plan"],
                       "diff": [{"target": o["target_ref"], "to": o["new_ref"]} for o in out["draft_plan"]["operations"]]}}


def _run(case: str, status: str = "completed") -> dict:
    return {"arm": "x", "case_ref": case, "status": status, "closed_early": False, "cost_known": True,
            "oracle_ref": "oracle:handwritten@1", "final_state_ref": "state:s@1", "effect_receipts": [], "reason": None}


def _standin_arms() -> dict:
    """Stand-in ArmReport runs (assets_handwritten suite): the base fails two cases, the candidate completes all."""
    cases = ["c1", "c2", "c3", "c4"]
    return {"base": [_run(c, "failed" if c in ("c1", "c2") else "completed") for c in cases],
            "candidate": [_run(c) for c in cases]}


JUDGE = "claude-gsipy"


def step_06(ctx: Ctx) -> dict:
    world, hook = ctx.out["world"], ctx.cfg.hooks.run_arms
    core_arms = hook(ctx) if hook else None
    arms, arms_from = (core_arms, "core") if core_arms else (_standin_arms(), "stand-in")
    reports = {"arm_report:base@1": {"runs": arms["base"]}, "arm_report:cand@1": {"runs": arms["candidate"]}}
    doc = {"contract_version": "engine-steps/0", "step": "gate", "run_id": "run-thread01-0001", "data_class": "synthetic",
           "base_arm_report_ref": "arm_report:base@1", "candidate_arm_report_ref": "arm_report:cand@1",
           "suite_ref": "eval_suite:disputas-suite@1", "judge_actor": JUDGE, "author_actors": ["claude-wrld0"]}
    out = gate.gate_verdict(doc, reports, world, evaluators=ctx.cfg.gate_evaluators)
    ctx.out.update(gate_verdict=out["verdict"], gate=out, arms=arms)
    return {"status": "stand-in", "data_class": "synthetic", "actor": JUDGE,
            "receipt": {"provider": "claude-standin"},
            "detail": {"verdict": out["verdict"], "gates": out["gates"], "judge_actor": out["judge_actor"],
                       "quality_claims": out["quality_claims"], "arms": arms_from}}


MAX_REVISION_ROUNDS = 1


def step_07(ctx: Ctx) -> dict:
    if ctx.out["gate_verdict"] == "pass":
        return {"status": "not_exercised", "data_class": "synthetic", "detail": {"reason": "gate passed"}}
    # gate failed: a rule-driven stand-in revision, bounded; the same arms are re-judged and the loop then stops
    rounds, final = 0, ctx.out["gate_verdict"]
    while final != "pass" and rounds < MAX_REVISION_ROUNDS:
        rounds += 1  # rule: no model, no new candidate; the revision is recorded, not claimed to improve anything
    return {"status": "stand-in", "data_class": "synthetic", "receipt": {"provider": "claude-standin"},
            "detail": {"rounds": rounds, "bounded": rounds <= MAX_REVISION_ROUNDS, "final_decision": "do_nothing",
                       "last_verdict": final}}


def step_08(ctx: Ctx) -> dict:
    """Human only for authority: a SIMULATED local issuer signs an approval bound to the compiled draft digest.
    Replay verifies it with the local A03 Verifier; on the real Core the same JWS is verified by Core (INT0)."""
    src = str(ROOT / "e2e-core" / "src")
    if src not in sys.path:
        sys.path.insert(0, src)
    from codex_standin import jwtsvc as J
    digest = ctx.out["compiled"]["draft_plan"]["digest"]
    key = J.private_from_seed(J.b64u(b"thread01-issuer-seed-32-bytes!!!!"[:32]))
    ring = J.KeyRing({"sim-issuer-1": ("sim-human-issuer", "pulso-core", J.public_of(key))})
    now = 1_800_000_000
    ver = J.Verifier(ring, now=lambda: now)
    claims = {"iss": "sim-human-issuer", "aud": "pulso-core", "exp": now + 300, "jti": "approval-0001",
              "scope": "approve", "purpose": f"publish:{digest}", "tenant_id": "pulso_local", "sub": "simulated-approver"}
    token = J.sign(key, "sim-issuer-1", claims)
    ok = ver.verify(token, aud="pulso-core", scope="approve", purpose=f"publish:{digest}")
    try:  # the same approval must not authorise a different plan
        ver.verify(J.sign(key, "sim-issuer-1", {**claims, "jti": "approval-0002"}), aud="pulso-core", scope="approve",
                   purpose="publish:sha256:" + "0" * 64)
        tampered = False
    except J.Denied:
        tampered = True
    ctx.out["approval"] = {"jti": ok["jti"], "digest": digest}
    return {"status": "simulated", "data_class": "synthetic", "receipt": {"provider": "simulated-issuer"},
            "detail": {"bound_to_digest": digest, "tampered_rejected": tampered, "verified_by": "local-stand-in-verifier",
                       "issuer": "simulated"}}


class RegistryDouble:
    """In-process stand-in for the Core registry writer and alias table (INT0 replaces it through CoreHooks)."""

    def __init__(self):
        self.aliases: dict[str, str] = {}

    def publish(self, ctx: "Ctx") -> dict:
        digest = ctx.out["approval"]["digest"]  # only an approved digest can be published
        rid = "rel-" + digest.split(":")[1][:12]
        self.aliases["staging"] = rid
        return {"release_id": rid, "alias": "staging"}

    def alias_read(self, ctx: "Ctx", alias: str) -> dict:
        return {"release_id": self.aliases[alias], "alias": alias}


def step_09(ctx: Ctx) -> dict:
    if not ctx.out.get("approval"):
        raise RuntimeError("no approval: nothing to publish")
    reg, h = RegistryDouble(), ctx.cfg.hooks
    published = (h.publish or reg.publish)(ctx)
    ctx.out["published"] = published
    read = (h.alias_read or (reg.alias_read if not h.publish else None))
    if read is None:
        raise RuntimeError("publish hook supplied without an alias_read hook")
    alias = read(ctx, published["alias"])
    both = bool(h.publish and h.alias_read)
    ctx.out["alias_read"] = alias
    return {"status": "real-narrow" if both else "stand-in", "data_class": "synthetic",
            "receipt": {"provider": "core-local-staging" if both else "claude-standin"},
            "detail": {"published": published, "alias_read": alias, "registry": "core" if both else "in-process-double"}}


EFFECT_AUTHOR = "claude-p2py-effects"
MECHANISM_AUTHOR = "claude-ed0"


def step_10(ctx: Ctx) -> dict:
    """Observation only: platform-sim emits release.* and a simulated effect series. Nothing here feeds a decision,
    the gate or a revision; memory and successor are not exercised in DEMO-0."""
    _ensure_paths()
    from platform_live import PlatformLiveSim
    from platform_live import effects as fx
    rid = ctx.out["published"]["release_id"]
    effect = fx.EffectSpec(effect_id="effect-thread01", author=EFFECT_AUTHOR, metric="first_response_seconds",
                           baseline=300.0, delta_pct=-20.0, ramp_days=2, noise_sd=0.0, seed=7)
    planted = fx.PlantedMechanism(mechanism_id="mech-thread01", author=MECHANISM_AUTHOR, kind="recurrence")
    sim = PlatformLiveSim(seed=1)
    sim.publish_release("atencion", ctx.out["published"]["alias"], rid, effect=effect, mechanism=planted)
    sim.fast_forward(days=3)
    series = sim.effect_series(rid)
    types = sorted({r[0] for r in sim.conn.execute("select event_type from event_log")})
    labels = sim.observation_labels()
    ctx.out["effect_author"] = EFFECT_AUTHOR
    return {"status": "simulated", "data_class": "simulated", "receipt": {"provider": "platform-sim"},
            "detail": {"observation_only": True, "feeds_decision": False, "event_types": types, "effect_series": series,
                       "memory": labels["memory"], "successor": labels["successor"],
                       "release_event": labels["release_event"], "effect": labels["effect"]}}


STEPS: list[tuple[int, str, Callable]] = [
    (1, "trigger", step_01), (2, "signals", step_02), (3, "scout", step_03), (4, "opportunity", step_04),
    (5, "compile", step_05), (6, "gate", step_06), (7, "revision", step_07), (8, "approval", step_08),
    (9, "publish", step_09), (10, "observation", step_10),
]


def _finish(n: int, sid: str, rec: dict, ctx: Ctx) -> dict:
    return {"n": n, "id": rec.get("id", sid), "target": "local", "sha": ctx.sha,
            "contract_revision": CONTRACT_REVISION, "host": HOST, **{k: v for k, v in rec.items() if k != "id"}}


def _er():
    import importlib.util
    if "engine_run" in sys.modules:
        return sys.modules["engine_run"]
    spec = importlib.util.spec_from_file_location("engine_run", ROOT / "contracts" / "engine-run" / "engine_run.py")
    m = importlib.util.module_from_spec(spec)
    sys.modules["engine_run"] = m
    spec.loader.exec_module(m)
    return m


def build_report(ctx: Ctx, steps: list[dict]) -> dict:
    """The final engine-run report (C-2): per-step labels, per-port provenance, authors, engine-generated doubles[]."""
    keep = ("id", "n", "status", "data_class", "target", "sha", "contract_revision", "host", "receipt", "actor",
            "model", "stage_output")
    rep_steps = [{k: s[k] for k in keep if k in s} for s in steps]
    by = {(s["n"], s["id"]): s for s in steps}
    st = lambda n: next((s["status"] for s in steps if s["n"] == n), "red")  # noqa: E731
    world = ctx.out.get("world") or {"authors": {}}
    h = ctx.cfg.hooks
    ports = [{"port": "llm_gateway", "provenance": "roleplay-shim:replay", "price_source": "placeholder-rate-card"},
             {"port": "registry", "provenance": "core-local-staging" if (h.publish and h.alias_read) else "in-process-double",
              "price_source": "n/a"},
             {"port": "human_issuer", "provenance": "simulated-local-issuer", "price_source": "n/a"},
             {"port": "platform", "provenance": "platform-sim", "price_source": "n/a"}]
    report = {"contract_revision": CONTRACT_REVISION, "target": "local", "sha": ctx.sha, "host": HOST, "label": "DEMO-0",
              "quality_claims": "forbidden", "mode": ctx.cfg.mode, "steps": rep_steps, "ports": ports,
              "authors": {"world": world["authors"].get("world"), "suite": world["authors"].get("suite"),
                          "effect": ctx.out.get("effect_author"), "judge": JUDGE,
                          "suite_sealed_at": SUITE_SEALED_AT, "candidate_created_at": ctx.out.get("candidate_created_at")}}
    observed = {"model": by.get((3, "scout"), {}).get("status", "red"),
                "jev": "not_exercised(blocked: agent-core PR 28 not on main)",
                "issuer": st(8), "product": "simulated" if st(9) == "stand-in" else st(9), "host": HOST,
                "gate": "claude-authored(structural, quality_claims forbidden)", "data_origin": "generated_sample"}
    report["doubles"] = _er().generate_doubles(report, observed)
    return report


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
    ctx.out["report"] = build_report(ctx, steps)
    return {"steps": steps, "mode": cfg.mode, "host": HOST, "replay": ctx.out.get("replay", {"misses": 1, "calls": 0}),
            "report": ctx.out.get("report", {}), "m3": ctx.out.get("m3", {}), "mapper": ctx.out.get("mapper"),
            "categories": ctx.out.get("categories"), "gate_verdict": ctx.out.get("gate_verdict"), "ctx": ctx.out}


def main(argv=None) -> int:
    """python -m claude_standin.thread01 --record QUEUE_DIR   re-record the replay fixtures with the scripted responder
    python -m claude_standin.thread01 --replay QUEUE_DIR      replay a recorded queue; prints the labelled step table
    The synthetic package and lab are written to a temp dir (untracked)."""
    import argparse
    import tempfile
    ap = argparse.ArgumentParser()
    ap.add_argument("--record")
    ap.add_argument("--replay")
    ap.add_argument("--exe", default=os.environ.get("ED0_RUNNER_EXE", "D:/cargo-targets/claude-ed0/debug/improvement-engine.exe"))
    a = ap.parse_args(argv)
    qdir, mode = (a.record, "record") if a.record else (a.replay, "replay")
    with tempfile.TemporaryDirectory() as tmp:
        res = run_thread(ThreadConfig(workdir=Path(tmp), exe=a.exe, queue_dir=Path(qdir), mode=mode))
    for s in res["steps"]:
        print(f"{s['n']:>2} {s['id']:<12} {s['status']:<16} {s.get('data_class')}  {s.get('error', '')}")
    print("replay", res["replay"])
    return 0 if all(s["status"] != "red" for s in res["steps"]) else 1


if __name__ == "__main__":
    raise SystemExit(main())
