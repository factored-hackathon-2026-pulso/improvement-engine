"""Demo driver: plays the scripted 'magic demo' scenario through the e2e-core Codex stand-in against the REAL real_local Core stack
(or, with --offline, against labelled synthetic Core values for a stack-free console preview), then writes the results bundle, the console
fixture worlds and the honest demo report.

python -m pulso_demo.driver --out demo/out [--offline]       (live mode reads E2E_ENV_FILE / E2E_KEYS_FILE, set by demo/run.ps1)"""

from __future__ import annotations

import argparse
import json
import os
import sys
import time
import traceback
from datetime import UTC, datetime
from pathlib import Path
from typing import Any

from pulso_demo import analysis, bundle, dataset, human_flow, translate
from pulso_demo.decision_hook import DecisionHook, DecisionRequest

TENANT = "tenant-local"
RESEARCH, VERIFY, DESIGN = "research stage", "independent verification stage", "design stage"


def inline_sql(q: dict[str, Any]) -> str:
    """The lab_query tool takes SQL text: bind the recorded parameters as literals (same statement the dataset executed)."""
    sql, params = q["sql"], list(q["params"])
    for p in params:
        sql = sql.replace("?", repr(p) if isinstance(p, str) else str(p), 1)
    return sql


def _tool_calls(queries: list[dict[str, Any]]) -> list[dict[str, Any]]:
    return [{"kind": "tool_call", "tool": "pulso/lab_query@1.0.0", "args": {"sql": inline_sql(q)}} for q in queries]


def model_outputs(scout: dict[str, Any], verify: dict[str, Any], alts: list[dict[str, Any]], ref: dict[str, str]) -> dict[str, Any]:
    """Final answers of the scripted stages, derived from the analysis (the hypotheses/verdicts are what the SQL measured)."""
    assess = {a["key"]: a for a in verify["assessments"]}
    hyps = []
    for i, h in enumerate(scout["hypotheses"][:4], 1):
        hyps.append({"id": f"h{i}", "statement": f"{h['flow']} {h['value']}: abandonment {h['rate']:.1%} vs {h['baseline_rate']:.1%} baseline",
                     "mechanism": f"concentration of abandonment by {h['dim']} (descriptive; mechanism not established by the scout)",
                     "evidence_refs": [ref], "counterevidence_refs": [], "missing_evidence": ["causal test"], "next_queries": []})
    key_of = {f"h{i}": h["key"] for i, h in enumerate(scout["hypotheses"][:4], 1)}
    assessments = []
    for hid, key in key_of.items():
        a = assess[key]
        assessments.append({"hypothesis_id": hid, "verdict": a["verdict"], "evidence_refs": [ref],
                            "counterevidence_refs": [ref] if a["verdict"] == "refuted" else [], "limitations": a["limitations"]})
    return {
        RESEARCH: {"schema_version": "1", "hypotheses": hyps},
        VERIFY: {"schema_version": "1", "assessments": assessments},
        DESIGN: {"schema_version": "1", "change_spec": {"target": "pulso-scout", "kind": "prompt"},
                 "rationale": ("verifier-supported hypothesis: " + ", ".join(k for k, a in assess.items() if a["verdict"] == "supported")) if any(a["verdict"] == "supported" for a in assess.values()) else "no supported hypothesis", "evidence_refs": [ref],
                 "alternatives": [{"id": a["id"], "kind": a["kind"], "summary": a["summary"]} for a in alts]},
    }


# ------------------------------------------------------------------------------------------------------------- live mode
def _stage_core(stage: Any, facts: dict[str, Any]) -> dict[str, Any]:
    out = stage.out
    return {"state": out.get("state"), "outcome": out.get("outcome"), "core_run_id": out.get("core_run_id"),
            "release_id": (out.get("receipt") or {}).get("release_id"), "task_binding_ref": out.get("task_binding_ref"),
            "facts_keys": sorted(facts)}


def run_live(scout: dict[str, Any], verify: dict[str, Any], attempts: list[dict[str, Any]], alts: list[dict[str, Any]]) -> tuple[dict[str, Any], dict[str, Any], list[str], dict[str, Any]]:
    import httpx
    from codex_standin.bridge import Bridge
    from codex_standin.dto import admission, idempotency_key
    from codex_standin.engine import ASSETS, Db, Engine, candidate_changes, ref_of, suite_digest  # noqa: F401
    from codex_standin.stack import SECRETS
    from helpers import OPS, arm_body, drafts_digest, run_arm, smoke_scenarios, writer_commitment  # e2e-core/tests/live/helpers.py

    import hashlib
    notes: list[str] = []
    env = json.loads(Path(os.environ["E2E_ENV_FILE"]).read_text("ascii"))
    keys = json.loads(Path(os.environ["E2E_KEYS_FILE"]).read_text("ascii"))
    runtime = f"http://127.0.0.1:{env['ports']['runtime']}"
    bridge = Bridge(runtime, keys["service_seed"], keys["service_kid"])
    fx = httpx.Client(base_url=env["url"], timeout=120)
    for _ in range(90):
        try:
            if httpx.get(runtime + "/readyz", timeout=3).status_code == 200 and fx.get("/_e2e/info").status_code == 200:
                break
        except httpx.HTTPError:
            pass
        time.sleep(1)
    pw = next(line.split("=", 1)[1].strip() for line in (SECRETS / env["namespace"] / "core.env").read_text("utf-8").splitlines()
              if line.startswith("POSTGRES_PASSWORD="))
    db = Db(f"postgresql://postgres:{pw}@127.0.0.1:{env['ports']['postgres']}/core_runtime")
    e = Engine(bridge, fx, TENANT)
    n = str(time.time_ns())[-12:]
    outs = model_outputs(scout, verify, alts, ref_of(TENANT))
    # scripted model: the scout/verifier issue the REAL lab_query tool calls (SQL text recorded from the dataset), then answer
    for rid, marker, queries in ((f"scout-{n}", RESEARCH, scout["queries"]), (f"verifier-{n}", VERIFY, verify["queries"][:3]), (f"design-{n}", DESIGN, [{"sql": analysis.DO_NOTHING_SQL, "params": []}])):  # the builder prices do_nothing through the lab too
        e.configure(llm_rules=[{"id": rid, "match": {"system_contains": marker},
                                "responses": _tool_calls(queries) + [{"kind": "final", "output": outs[marker]}]}])
    rel = e.releases
    core: dict[str, Any] = {"attempts": []}
    s = e.stage("scout", f"job-scout-{n}", "scout", "pulso-scout", {"briefing_ref": f"wiki/briefing-{n}.md"}, memory_snapshot_ref=f"mem-{n}", extract_manifest_ref=f"ex-{n}")
    sf = e.facts(s.out["core_run_id"]) if s.out.get("core_run_id") else {}
    core["scout"] = _stage_core(s, sf)
    hyp = sf.get("pulso_hypotheses", {}).get("value", {})
    e.seal(f"hyp-{n}", hyp)
    v = e.stage("verifier", f"job-verifier-{n}", "verifier", "pulso-verifier", {"hypotheses_ref": f"hyp-{n}"}, memory_snapshot_ref=f"mem-{n}", extract_manifest_ref=f"ex-{n}")
    vf = e.facts(v.out["core_run_id"]) if v.out.get("core_run_id") else {}
    core["verifier"] = _stage_core(v, vf)
    e.seal(f"design-in-{n}", {"hypotheses": hyp.get("hypotheses"), "assessments": vf.get("pulso_verification", {}).get("value", {}).get("assessments")})
    d = e.stage("builder_design", f"job-design-{n}", "design", "pulso-builder-design", {"design_input_ref": f"design-in-{n}"}, memory_snapshot_ref=f"mem-{n}", extract_manifest_ref=f"ex-{n}")
    df = e.facts(d.out["core_run_id"]) if d.out.get("core_run_id") else {}
    core["design"] = _stage_core(d, df)
    core["design"]["alternative_kinds"] = sorted({a["kind"] for a in df.get("pulso_change_spec", {}).get("value", {}).get("alternatives", [])})
    manifest = None
    for i, att in enumerate(attempts, 1):
        version = f"1.{i}.0"
        changes = candidate_changes(version)
        title = f"pulso-key:demo-{n}-{i}"
        e.seal(f"plan-{n}-{i}", {"agent_id": "pulso-scout", "title": title, "changes": changes})
        base = rel["pulso-scout"]
        w = e.stage("writer", f"job-writer-{n}-{i}", "writer", "pulso-writer", {"draft_plan_ref": f"plan-{n}-{i}", "proposal_id": None, "base_release_id": base,
                    "evaluate_enabled": False}, registry_mutation_commitment=writer_commitment(base, title, changes))
        wf = e.facts(w.out["core_run_id"]) if w.out.get("core_run_id") else {}
        wr = wf.get("pulso_writer_receipts", {}).get("value", {})
        proposal_id, chash = wr.get("proposal_id"), wr.get("candidate_hash")
        ctx, job = f"ctx-{n}-{i}", f"job-evalonly-{n}-{i}"
        key = idempotency_key(TENANT, job, "writer", 1, "evalonly")
        bref = hashlib.sha256(f"{TENANT}|{key}".encode()).hexdigest()
        e.configure(preauthorized_bindings=[{"tenant": TENANT, "binding_ref": bref}])
        adm = admission(ref=ctx, binding_ref=bref, proposal_id=proposal_id, candidate_hash=chash, suite_id="pulso-smoke", suite_version=version,
                        suite_digest=suite_digest(changes), budget_ref="bud-e2e")
        a_resp = bridge.admit(TENANT, job, adm)
        eo = e.stage("writer", job, "evalonly", "pulso-writer", {"draft_plan_ref": f"plan-{n}-{i}", "proposal_id": proposal_id, "base_release_id": base,
                     "evaluate_enabled": True, "evaluation_suite_id": "pulso-smoke", "evaluation_suite_version": version},
                     registry_mutation_commitment={"mode": "evaluate_only", "proposal_id": proposal_id, "base_release_id": base, "evaluate_enabled": True,
                                                   "evaluation_context_ref": ctx, "operations": []})
        ef = e.facts(eo.out["core_run_id"]) if eo.out.get("core_run_id") else {}
        ewr = ef.get("pulso_writer_receipts", {}).get("value", {})
        native = ewr.get("native_evaluation")
        row: dict[str, Any] = {"candidate_id": att["candidate"]["id"], "proposal_id": proposal_id, "candidate_hash": chash, "suite_version": version,
                               "admission_status": a_resp.status_code, "writer_run_id": w.out.get("core_run_id"), "evalonly_run_id": eo.out.get("core_run_id"),
                               "write_receipts": wr.get("write_receipts", []) + [r for r in ewr.get("write_receipts", []) if r["op"] == "evaluate"],
                               "native": ({"verdict": native["verdict"], "eval_run_ref": native["eval_run_ref"], "report_digest": native["report_digest"]}
                                          if native and native.get("report_digest") else None)}
        try:  # base and candidate scenarios on the bank fixture (plan step 6); failure is reported, never hidden
            if manifest is None:
                scen = smoke_scenarios()
                manifest = f"manifest-{n}"
                e.seal(manifest, {"scenarios": scen, "entries": {x["id"]: {} for x in scen}})
            rev = int(db.one("select (proposal_json::json->>'rev')::int from reg_proposals where proposal_id=%s", proposal_id))
            target = {"kind": "frozen_candidate", "proposal_id": proposal_id, "expected_rev": rev, "base_release_id": base, "candidate_hash": chash,
                      "draft_plan_digest": drafts_digest(changes)}
            b = run_arm(e, arm_body(f"arm-base-{n}-{i}", w.out["task_binding_ref"], manifest, "task_builder", {"kind": "published_release", "release_id": base}))
            c = run_arm(e, arm_body(f"arm-cand-{n}-{i}", w.out["task_binding_ref"], manifest, "task_builder", target, arm="candidate"))
            row["arms"] = {"baseline": (b.report or {}).get("status", f"http_{b.response.status_code}"), "candidate": (c.report or {}).get("status", f"http_{c.response.status_code}")}
        except Exception as exc:  # noqa: BLE001
            row["arms"] = {"error": repr(exc)[:200]}
            notes.append(f"arms attempt {i}: {exc!r}"[:240])
        core["attempts"].append(row)
    exporter = collect_exporter(e, db, notes)
    finalize_core_stats(e, core)
    ctx = {"engine": e, "bridge": bridge, "db": db, "n": n, "env": env}
    return core, exporter, notes, ctx


def collect_exporter(e: Any, db: Any, notes: list[str], want_run: str | None = None, timeout_s: int = 120) -> dict[str, Any]:
    """Wait until every Core audit row reached the exporter's ingest fixture (and, if given, the chain receipt of `want_run`), then read it."""
    deadline, ing = time.time() + timeout_s, {}
    while time.time() < deadline:
        ing = e.state().get("ingest", {})
        audit = [k for k in ing.get("event_keys", []) if k[1] == "local.audit" and k[3] == "engine_event"]
        caught_up = bool(audit) and len(audit) >= int(db.one("select count(*) from audit_events"))
        if caught_up and (want_run is None or any(r["run_id"] == want_run for r in ing.get("verification_receipts", []))):
            break
        time.sleep(3)
    else:
        notes.append(f"exporter did not catch up within {timeout_s} s" + (f" (no chain receipt for {want_run})" if want_run else ""))
    return {"audit_events": len([k for k in ing.get("event_keys", []) if k[1] == "local.audit"]),
            "verification_receipts": [{"run_id": r["run_id"], "check_result": r["check_result"], "chain_head_hash": r["chain_head_hash"]}
                                      for r in ing.get("verification_receipts", [])],
            "batches": len(ing.get("batch_summaries", []))}


def finalize_core_stats(e: Any, core: dict[str, Any]) -> None:
    st = e.state()
    core["llm_calls"] = len(st.get("llm_calls", []))
    core["llm_unscripted"] = st.get("llm_unscripted")
    core["lab_queries"] = [r["body"]["sql"] for r in st["requests"] if r["route"] == "lab_query"]


def run_successor_live(ctx: dict[str, Any], obs: dict[str, Any], exporter_before: dict[str, Any], core: dict[str, Any], notes: list[str]) -> tuple[dict[str, Any], dict[str, Any]]:
    """Step 10 (stand-in engine): the SECOND batch of scripted observations goes through the REAL Core (scout run issuing lab_query calls with the
    batch-2 SQL, audit events) and the REAL exporter -> ingest fixture (second batch of audit events + chain verification)."""
    from codex_standin.engine import ref_of
    e, db, n = ctx["engine"], ctx["db"], ctx["n"]
    outs = model_outputs(obs["scout"], obs["verify"], [], ref_of(TENANT))
    e.configure(llm_rules=[{"id": f"scout-b2-{n}", "match": {"system_contains": RESEARCH},
                            "responses": _tool_calls(obs["scout"]["queries"]) + [{"kind": "final", "output": outs[RESEARCH]}]}])
    s = e.stage("scout", f"job-scout-b2-{n}", "scout", "pulso-scout", {"briefing_ref": f"wiki/briefing-b2-{n}.md"}, memory_snapshot_ref=f"mem-b2-{n}",
                extract_manifest_ref=f"ex-b2-{n}")
    sf = e.facts(s.out["core_run_id"]) if s.out.get("core_run_id") else {}
    sc_core = _stage_core(s, sf)
    exporter = collect_exporter(e, db, notes, want_run=s.out.get("core_run_id"))
    finalize_core_stats(e, core)
    rec = next((r for r in exporter["verification_receipts"] if r["run_id"] == s.out.get("core_run_id")), None)
    succ = {"batch": 2, "core": sc_core, "exporter": {"batches_before": exporter_before.get("batches"), "batches_after": exporter["batches"],
                                                      "audit_events_before": exporter_before.get("audit_events"), "audit_events_after": exporter["audit_events"],
                                                      "verification_ok": (rec["check_result"].get("ok") if rec else None)},
            **successor_analysis(obs)}
    return succ, exporter


def successor_analysis(obs: dict[str, Any]) -> dict[str, Any]:
    return {"memory_updates": obs["memory_updates"], "new_hypotheses": obs["new_hypotheses"], "successor_target": obs["successor_target"],
            "evidence": [{"id": q["id"], "digest": q["rows_digest"]} for q in obs["scout"]["queries"]]}


# ---------------------------------------------------------------------------------------------------------- offline mode
def offline_core(attempts: list[dict[str, Any]]) -> tuple[dict[str, Any], dict[str, Any], list[str]]:
    """No stack: SYNTHETIC Core values, marked as such everywhere (ids start with `offline-`)."""
    ok = lambda name: {"state": "terminal_ok", "outcome": "completed", "core_run_id": f"offline-{name}", "release_id": "offline-release"}  # noqa: E731
    wr = [{"op": o, "verified": True} for o in ("create_proposal", "put_draft", "freeze", "evaluate")]
    core = {"scout": ok("scout"), "verifier": ok("verifier"), "design": ok("design"), "synthetic": True, "attempts": [
        {"candidate_id": a["candidate"]["id"], "proposal_id": f"offline-prop-{i}", "candidate_hash": f"{i:064x}", "write_receipts": wr,
         "native": {"verdict": "pass", "eval_run_ref": f"offline-evr-{i}", "report_digest": f"{i + 90:064x}"}} for i, a in enumerate(attempts, 1)]}
    exporter = {"audit_events": 0, "verification_receipts": [], "synthetic": True}
    return core, exporter, ["offline mode: Core values are synthetic, exporter section empty (observation stays unknown)"]


# --------------------------------------------------------------------------------------------------------------- report
STEPS = [
    (1, "Snapshot/E0/observations wake the engine; nobody picks the category", "analysis.scout"),
    (2, "Measured signal families, discarded candidates, investigation", "analysis.scout"),
    (3, "Scout uses SQL adaptively; separate verifier contradicts or supports", "core.verifier"),
    (4, "Opportunity with population, mechanism, uncertainty, alternatives incl. do_nothing", "core.design"),
    (5, "Concrete change: Core entities, versions, diff", "core.attempts.write_receipts"),
    (6, "Scenarios on base and candidate; two distinct gates", "core.attempts.native + analysis.attempts.improvement"),
    (7, "Failure triggers a bounded automatic revision", "analysis.attempts"),
    (8, "Human decision/approve (authority)", "decision_hook / human_flow"),
    (9, "Staging confirmation / exposure", "core.registry alias read"),
    (10, "New observations: exporter, chain verification, successor run", "exporter + analysis.observe"),
]
OUTCOME = {"complete": "shown", "dead": "failed", "unknown": "unknown"}


def step_status(i: int, world: dict[str, Any], core: dict[str, Any], exporter: dict[str, Any], decision: dict[str, Any] | None = None,
                successor: dict[str, Any] | None = None) -> tuple[str, str, str]:
    """(status, outcome, basis): status is WHO produced it (`real` = real Core/issuer receipts, `stand-in` = engine/judge/LLM double,
    `simulated` = a simulated human or a synthetic offline value); outcome is what happened (shown | failed | unknown | pending | not_run ...)."""
    nodes = {n["node_id"]: n["status"] for n in world["runs"][translate.MAIN]["nodes"]}
    syn = bool(core.get("synthetic"))
    real = "simulated" if syn else "real"
    if i in (1, 2):
        return "stand-in", "shown", "scout SQL over the synthetic dataset (scripted model)"
    if i == 3:
        return "stand-in", "shown" if nodes["verify"] == "complete" else nodes["verify"], "Core verifier run with a scripted LLM + data-derived verdicts"
    if i == 4:
        return "stand-in", "shown" if nodes["opportunity"] == "complete" else "blocked", "alternatives incl. do_nothing from the builder_design output"
    if i == 5:
        return real, "shown" if nodes["change"] == "complete" else nodes["change"], "writer receipts verified in the real Core registry (change spec is a stand-in)"
    if i == 6:
        return "stand-in", OUTCOME.get(nodes["evaluate_2"], nodes["evaluate_2"]), "native evaluation real; improvement judge is a stand-in"
    if i == 7:
        why = {"complete": "candidate 1 failed a gate; the bounded revision search found a candidate (see attempts[])",
               "dead": "candidate 1 failed a gate and no revision within bounds cleared it"}.get(nodes["revise"], "no gate failed (or no candidate): no revision needed")
        return "stand-in", {"complete": "shown", "dead": "failed"}.get(nodes["revise"], "not_triggered"), why
    stage = decision["stage"] if decision else None
    if i == 8:
        if decision is None:
            return "stand-in", "pending", "no human flow ran (hook pending); the demo never approves on its own"
        who = "simulated" if (decision["simulated_human"] or syn) else "real"
        basis = ("SIMULATED human supervisor (--human-mode scripted): " if decision["simulated_human"] else "manual approval by a person: ") + (
            "credential from the local sandbox human issuer, bound to the durable intention; Core approve with operation/hash fixed" if not syn
            else "synthetic offline flow")
        outcome = {"requested": "pending", "rejected": "rejected", "failed": "failed"}.get(stage or "", "shown")
        return who, outcome, basis
    if i == 9:
        if stage in ("staging_confirmed", "promoted"):
            return real, "shown", ("staging alias read confirms the release; prod " + ("promoted by the explicit --promote" if stage == "promoted" else "unchanged (exposure needs the explicit promote)"))
        if stage in ("approved_not_published", "published_unconfirmed"):
            return real, stage, "approved is not published: staging was not confirmed by an alias read"
        return real, "not_run", "needs an approved operation and the staging alias receipt"
    ok = bool(successor) and successor.get("core", {}).get("state") == "terminal_ok" and bool(successor.get("successor_target"))
    if not successor:
        return "stand-in", "not_run", "exporter delivery and chain verification of the first round only; no second batch before staging was confirmed"
    return ("simulated" if syn else "stand-in"), (OUTCOME.get(nodes["observation"], "unknown") if ok else "failed"), (
        "second batch of scripted observations -> exporter ingest fixture + chain verification; stand-in engine publishes/contradicts memory and starts a "
        "successor investigation")


def _no_change(results: dict[str, Any]) -> str | None:
    """Honest non-error terminal outcomes: nothing was supported (no candidate designed) or no candidate cleared both gates."""
    an = results["analysis"]
    if not an["attempts"]:
        return "no_opportunity"
    if an["attempts"][-1]["improvement"]["status"] != "pass":
        return "no_candidate_passed_gates"
    return None


def make_report(results: dict[str, Any], world: dict[str, Any], notes: list[str], mode: str, hook_state: str, started: str, error: str | None) -> dict[str, Any]:
    core, exporter = results["core"], results["exporter"]
    no_change = _no_change(results)
    dec, succ = results.get("decision"), results.get("successor")
    if dec and dec["stage"] == "requested":
        no_change = "awaiting_human_decision"
    elif dec and dec["stage"] == "rejected":
        no_change = "rejected_by_human"
    human = None
    if dec:
        human = {k: dec.get(k) for k in ("mode", "simulated_human", "actor", "stage", "proposal_id", "candidate_hash", "release_id", "staging_alias",
                                         "prod_alias", "promoted", "error", "reason")}
        human["label"] = "SIMULATED human supervisor (scripted)" if dec["simulated_human"] else "manual approval by a person"
    successor = None
    if succ:
        successor = {k: succ.get(k) for k in ("batch", "core", "exporter", "memory_updates")}
        successor["new_hypotheses"] = [h["key"] for h in succ.get("new_hypotheses", [])]
        successor["successor_investigation"] = (succ.get("successor_target") or {}).get("key")
    return {"schema": "pulso-demo-report/2", "mode": mode, "started_at": started, "finished_at": datetime.now(UTC).isoformat(), "namespace": results["namespace"],
            "outcome": "error" if error else (no_change or "ok"), "error": error, "notes": notes,
            "claims": "Codex stand-in demo over a synthetic bank dataset; NOT the Rust engine, NOT a real model, NOT a causal/SLA claim (mechanism_proxy only)",
            "doubles": results["doubles"] + ([{"id": "offline_core", "what": "synthetic Core values (no stack was run)"}] if core.get("synthetic") else []),
            "steps": [{"n": n, "title": t, "source": src, **dict(zip(("status", "outcome", "basis"), step_status(n, world, core, exporter, dec, succ)))} for n, t, src in STEPS],
            "gates": world["gates"], "attempts": world["demo"]["attempts"], "decision_hook": hook_state,
            "human": human, "decision_trail": dec["trail"] if dec else [], "successor": successor,
            "chain_verification": [{"run_id": r["run_id"], "ok": r["check_result"].get("ok")} for r in exporter.get("verification_receipts", [])],
            "exported_audit_events": exporter.get("audit_events"), "lab_queries_through_core": len(core.get("lab_queries", [])),
            "llm_unscripted_calls": core.get("llm_unscripted"), "console_files": {"world": "world.json", "replay": "replay.json", "results": "results.json"}}


_INTERIM = {"decision_requested": "requested", "human_decided": "requested", "approved": "approved", "published": "published_unconfirmed",
            "staging_confirmed": "staging_confirmed", "promoted": "promoted"}


def offline_successor(obs: dict[str, Any]) -> dict[str, Any]:
    return {"batch": 2, "core": {"state": "terminal_ok", "core_run_id": "offline-successor-scout", "synthetic": True},
            "exporter": {"batches_before": None, "batches_after": None, "verification_ok": None, "synthetic": True}, **successor_analysis(obs)}


def write_outputs(out: Path, results: dict[str, Any], world: dict[str, Any]) -> None:
    for name, obj in (("results.json", results), ("world.json", world), ("replay.json", translate.replay_world(world))):
        (out / name).write_text(json.dumps(obj, indent=1), "utf-8")


def main(argv: list[str] | None = None, hook: DecisionHook | None = None) -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", required=True)
    ap.add_argument("--offline", action="store_true")
    ap.add_argument("--namespace", default=os.environ.get("DEMO_NAMESPACE", "demo"))
    ap.add_argument("--human-mode", choices=["scripted", "manual"], default="scripted",
                    help="scripted = labelled SIMULATED human supervisor approves; manual = pause until `python -m pulso_demo.decide --out <out> approve`")
    ap.add_argument("--human-timeout", type=float, default=900.0, help="manual mode: seconds to wait for the person before leaving the decision pending")
    ap.add_argument("--promote", action="store_true", help="explicitly promote the staged release to prod (default: staging only)")
    a = ap.parse_args(argv)
    out = Path(a.out)
    out.mkdir(parents=True, exist_ok=True)
    started = datetime.now(UTC).isoformat()
    conn = dataset.build()
    sc = analysis.scout(conn)
    ver = analysis.verify(conn, sc["hypotheses"])
    assess = {x["key"]: x["verdict"] for x in ver["assessments"]}
    attempts: list[dict[str, Any]] = []
    alts: list[dict[str, Any]] = []
    analysis_notes: list[str] = []
    if any(v == "supported" for v in assess.values()):  # a candidate is only designed when the verifier supports something
        c1 = analysis.first_candidate()  # canned aggressive first draft (see README: scripted); the gate verdict on it is derived from the data
        j1 = analysis.judge(conn, c1)
        attempts.append(j1)
        if j1["improvement"]["status"] != "pass":  # revision is triggered ONLY by a failing gate
            try:
                c2 = analysis.revise(conn, c1, j1)
                attempts.append(analysis.judge(conn, c2))
            except RuntimeError as exc:
                analysis_notes.append(f"automatic revision exhausted its bounds: {exc}")
        alts = analysis.alternatives(conn, attempts[-1]["candidate"])
    attempts = [{"candidate": j["candidate"], "improvement": j["improvement"], "native_proxy": j["native_proxy"]} for j in attempts]
    obs = analysis.observe(dataset.build(seed=20260102, post=True), sc["hypotheses"], ver)  # the scripted second batch of observations (step 10)
    error: str | None = None
    notes: list[str] = []
    ctx: dict[str, Any] | None = None
    try:
        if a.offline:
            core, exporter, notes = offline_core(attempts)
        else:
            core, exporter, notes, ctx = run_live(sc, ver, attempts, alts)
    except Exception:  # noqa: BLE001 - the report must exist even when the stack failed; the failure is recorded, nothing is faked
        error = traceback.format_exc()[-1500:]
        core = {"scout": None, "verifier": None, "design": None, "attempts": []}
        exporter = {"audit_events": 0, "verification_receipts": []}
    notes = analysis_notes + notes
    common: dict[str, Any] = dict(namespace=a.namespace, tenant=TENANT, generated_at=datetime.now(UTC).strftime("%Y-%m-%dT%H:%M:%SZ"), scout=sc, verify=ver,
                                  alternatives=alts, attempts=attempts, dataset={"rows": 8000, "seed": 20260101})
    results = bundle.assemble(core=core, exporter=exporter, **common)
    world = translate.build_world(results)
    # ---- steps 8-9: only a candidate that passed BOTH gates reaches the human; nothing is approved otherwise
    decision: dict[str, Any] | None = None
    top = world["demo"]["core"]["attempts"][-1] if world["demo"]["core"]["attempts"] else {}
    gates_pass = world["gates"]["combined"]["decision"] == "needs_human" and error is None
    if hook is None and gates_pass:
        try:
            hook = _make_hook(a, out, top, ctx, common, core, exporter)
        except Exception:  # noqa: BLE001
            error = "human flow could not be wired: " + traceback.format_exc()[-800:]
    if hook is not None and gates_pass:
        res = hook.decide(DecisionRequest(run_id=translate.MAIN, proposal_id=str(top.get("proposal_id")), candidate_hash=str(top.get("candidate_hash")), operation="approve"))
        if res.detail.get("mode") != "none":
            decision = res.detail
            if a.human_mode == "scripted" and decision["stage"] not in ("staging_confirmed", "promoted") and error is None:
                error = f"scripted human flow ended at stage {decision['stage']}: {decision.get('error')}"
    # ---- step 10: only after staging is confirmed
    successor: dict[str, Any] | None = None
    if decision and decision["stage"] in ("staging_confirmed", "promoted"):
        try:
            if a.offline:
                successor = offline_successor(obs)
            else:
                assert ctx is not None
                successor, exporter = run_successor_live(ctx, obs, exporter, core, notes)
        except Exception:  # noqa: BLE001
            error = (error or "") + "step 10 failed: " + traceback.format_exc()[-800:]
    results = bundle.assemble(core=core, exporter=exporter, decision=decision, successor=successor, **common)
    world = translate.build_world(results)
    write_outputs(out, results, world)
    report = make_report(results, world, notes, "offline" if a.offline else "live_real_local_core", decision["stage"] if decision else "pending", started, error)
    (out / "demo-report.json").write_text(json.dumps(report, indent=1), "utf-8")
    print(json.dumps({"outcome": report["outcome"], "mode": report["mode"], "human": a.human_mode, "steps": {s["n"]: f"{s['status']}/{s['outcome']}" for s in report["steps"]}}))
    return 0 if error is None else 1


def _make_hook(a: argparse.Namespace, out: Path, top: dict[str, Any], ctx: dict[str, Any] | None, common: dict[str, Any], core: dict[str, Any],
               exporter: dict[str, Any]) -> DecisionHook:
    """Approval flow wired to the live issuer + Core registry (or the labelled synthetic ports offline). While the human decides, the console
    world is rewritten after every event so the pending state is visible."""
    gate = human_flow.make_gate(a.human_mode, out, timeout_s=a.human_timeout)
    if a.offline:
        from pulso_demo.offline_ports import OfflineAuthorizer, OfflineRegistry
        registry: Any = OfflineRegistry(str(top.get("proposal_id")), str(top.get("candidate_hash")))
        authorizer: Any = OfflineAuthorizer()
    else:
        from pulso_demo import human_live
        assert ctx is not None
        registry, authorizer = human_live.build_live_ports(ctx["env"], ctx["bridge"], TENANT, out)

    def interim(trail: list[dict[str, Any]]) -> None:
        stage = _INTERIM.get(trail[-1]["event"], "requested") if trail else "requested"
        detail = {"mode": gate.mode, "simulated_human": gate.simulated, "actor": human_flow.ACTOR, "stage": stage, "proposal_id": top.get("proposal_id"),
                  "candidate_hash": top.get("candidate_hash"), "release_id": None, "staging_alias": None, "prod_alias": None, "promoted": False,
                  "error": None, "reason": None, "trail": list(trail)}
        r = bundle.assemble(core=core, exporter=exporter, decision=detail, **common)
        write_outputs(out, r, translate.build_world(r))

    return human_flow.FlowHook(human_flow.ApprovalFlow(registry, authorizer, gate, promote=a.promote, on_event=interim))


if __name__ == "__main__":
    sys.exit(main())
