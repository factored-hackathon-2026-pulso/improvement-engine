"""Prove a regression suite "fails today, passes with the fix" on the LOCAL agent-core stack (REG1 / plan W1-1).

    python scripts/regression/prove_fails_on_base.py --bundle <suite>.bundle.json --candidate patch1.json [--candidate patch2.json]
        [--base http://127.0.0.1:8001] [--state-dir .dev-stack] [--out story.json] [--no-live] [--candidate-always]

For one bundle (scripts/regression/build_suite.py) it attaches the suite to a draft of the BASE (no change) and to a draft of
each CANDIDATE (compiled patch `changes`, attempt 1, attempt 2 ...; evaluation stops at the first attempt that passes), freezes
and evaluates each through agent-core's own `evaluate`, merges the wording probes the native scorer cannot express (rendered
template text vs the seeded state), and decides:

  regression_suite_proven  at least one finding case fails on the base, every case passes on the final candidate, guards pass on both
  non_discriminating       the base already passes everything: this is NOT a regression suite (never announced as one)
  not_fixed                a finding case still fails on the final candidate
  guard_regressed          a guard case fails (on the base: the guard is not stable; on the candidate: the patch broke a behaviour)
  not_exercised            the stack is down (exit 3) and nothing live was measured
  infra_failed             evaluate did not return a verdict (infra error), nothing is claimed

It NEVER calls approve, publish, promote or reject. Output: the JSON verdict story the engine can embed in the dossier.
Credentials: the local staff token comes from <state-dir>/tokens.json (written by stack.py) and is only sent as a bearer
header to the local registry; it is never printed. Draft proposals are created with origin `manual`, so the 10/24h auto_detect
quota is not consumed.
"""
from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import re
import sys
import time
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
BASE_ARTIFACTS = REPO / "seams" / "crates" / "reasoning" / "fixtures" / "base_artifacts.json"
STORY_SCHEMA = "reg1.verdict_story/1"
INFRA_BACKOFF_S = 20
INFRA_RETRIES = 4  # `evaluate` answers failed_infra when the real JEV provider drops mid-scenario (seen live); retried, recorded
MODEL_POLICY = {"generation": "xiaomi/mimo-v2.6-flash", "reasoning": "xiaomi/mimo-v2.6-pro", "judge": "z-ai/glm-5.3-flash",
                "judge_rule": "the judge never scores what its own family attacked; this proof uses deterministic checks only"}


# ------------------------------------------------------------------------------------------------------------ probes
_VAR = re.compile(r"\{\{\s*(.+?)\s*\}\}")


def render(text: str, values: dict) -> str:
    """Same substitution as agent-core `render_template`: every `{{ path }}` replaced by values[path]."""
    def sub(m: re.Match) -> str:
        key = m.group(1).strip()
        if key not in values:
            raise KeyError(key)
        return str(values[key])
    return _VAR.sub(sub, text)


def run_probe(probe: dict, text: str | None, generate=None, cache: dict | None = None) -> dict:
    """`state_reflected`: render the template text with the seeded state (deterministic, offline).
    `generated_contains`: sample the real model with the prompt text (`generate(text, inputs, locale, n) -> [str]`); every
    sample must name the follow-up and match no `must_not_match` (model-in-the-loop; samples are recorded in the story)."""
    if text is None:
        return {"passed": False, "reason": f"no {probe['locale']} text for {probe.get('template_id') or probe.get('prompt_id')}"}
    if probe["kind"] == "generated_contains":
        if generate is None:
            return {"passed": False, "reason": "generated probe not measured (no generator)"}
        key = (text, probe["locale"], json.dumps(probe["inputs"], sort_keys=True), probe["samples"])
        if cache is not None and key in cache:
            outs = cache[key]
        else:
            outs = generate(text, probe["inputs"], probe["locale"], probe["samples"])
            if cache is not None:
                cache[key] = outs
        if not outs:
            return {"passed": False, "reason": "generated probe not measured (generator returned nothing: "
                                               f"{getattr(generate, 'last_error', '')})"}
        bad_digit = [o for o in outs if re.search(probe["must_not_match"], o)]
        missing = [o for o in outs if not any(w in o.lower() for w in probe["must_contain_any"])]
        ok = not bad_digit and not missing
        why = "" if ok else (f"{len(missing)}/{len(outs)} samples do not name the follow-up (one of {probe['must_contain_any']})"
                              + (f"; {len(bad_digit)} sample(s) carry a digit" if bad_digit else ""))
        return {"passed": ok, "reason": why, "samples": outs}
    try:
        out = render(text, probe["values"])
    except KeyError as e:
        return {"passed": False, "reason": f"render_error: placeholder {e.args[0]!r} has no value in the probe"}
    low = out.lower()
    ok = any(w in low for w in probe["must_contain_any"])
    return {"passed": ok, "reason": "" if ok else f"rendered text does not reflect the state (no one of {probe['must_contain_any']})",
            "rendered": out}


def probe_key(text: str, locale: str, inputs: dict, samples: int) -> str:
    """Stable key of one generation request (judge_story looks the engine-collected samples up by it)."""
    return hashlib.sha256(json.dumps([text, locale, inputs, samples], sort_keys=True, ensure_ascii=False).encode()).hexdigest()[:16]


def texts_of(changes: list[dict] | None, base_artifacts: dict, artifact_id: str, kind: str = "template") -> dict:
    """Locale texts of a template/prompt: from the candidate changes if it patches it, else the byte-exact base artifact."""
    for c in changes or []:
        content = c.get("content") or {}
        if c.get("kind") == kind and content.get("id") == artifact_id:
            return content.get("locales") or {}
    for a in base_artifacts.get("artifacts", []):
        if a["id"] == artifact_id:
            return a["locales"]
    return {}


# ------------------------------------------------------------------------------------------------------------ judging
def case_results(bundle: dict, native: dict[str, dict], changes: list[dict] | None, base_artifacts: dict, generate=None,
                 ignore_native: frozenset = frozenset(), absent: bool = False) -> dict:
    """Per-case {passed, source, reason}: native scenario result AND (when the case has one) the wording/generation probe.
    `ignore_native`: case ids whose native result is NOT used (agent-core `evaluate` did not exercise the candidate prompt:
    `native_not_candidate_bound`); the harness probe alone decides them and the entry is labelled. `absent`: the agent does not
    exist on this side (new agent on the base): finding cases fail by absence, guards are not applicable."""
    probes = {p["case_id"]: p for p in bundle.get("probes", [])}
    cache: dict = {}
    out = {}
    for cid in bundle["finding_case_ids"] + bundle["guard_case_ids"]:
        if absent:
            is_f = cid in bundle["finding_case_ids"]
            out[cid] = {"passed": not is_f, "source": "absent_on_base",
                        "reason": "the agent does not exist on the base: not run" if is_f else "not applicable: the agent does not exist on the base"}
            continue
        n = native.get(cid, {"passed": False, "reason": "not measured"})
        entry = {"passed": bool(n["passed"]), "source": "native", "reason": n.get("reason", "")}
        if cid in ignore_native:
            entry = {"passed": True, "source": "native_ignored", "reason": "", "native_ignored": {"passed": bool(n["passed"]), "why": "native_not_candidate_bound"}}
        p = probes.get(cid)
        if p is not None:
            if p["kind"] == "generated_contains":
                texts, name = texts_of(changes, base_artifacts, p["prompt_id"], "prompt"), "generated_probe"
            else:
                texts, name = texts_of(changes, base_artifacts, p["template_id"], "template"), "wording_probe"
            pr = run_probe(p, texts.get(p["locale"]), generate, cache)
            entry["probe"] = {k: pr[k] for k in ("rendered", "samples") if k in pr}
            if not pr["passed"]:
                ignored = cid in ignore_native
                entry.update(passed=False, source=name if (n["passed"] or ignored) else "native+" + name,
                             reason=("" if ignored or not n.get("reason") else n["reason"] + "; ") + pr["reason"])
        out[cid] = entry
    return out


def failed(per_case: dict, ids: list[str]) -> list[str]:
    return [i for i in ids if not per_case[i]["passed"]]


def decide(bundle: dict, base: dict, attempts: list[dict]) -> dict:
    """Outcome from the base per-case results and the attempts (last one is the final candidate)."""
    f_ids, g_ids = bundle["finding_case_ids"], bundle["guard_case_ids"]
    if base["verdict"] == "failed_infra" or any(a["verdict"] == "failed_infra" for a in attempts):
        return {"outcome": "infra_failed", "reason": "evaluate returned no verdict for a run"}
    base_fail_f, base_fail_g = failed(base["per_case"], f_ids), failed(base["per_case"], g_ids)
    if base_fail_g:
        return {"outcome": "guard_regressed", "reason": f"guards failing on the BASE (unstable guard): {base_fail_g}"}
    if not base_fail_f:
        if attempts:  # --candidate-always: the candidate was evaluated although the suite does not discriminate
            cand_bad = failed(attempts[-1]["per_case"], f_ids + g_ids)
            if cand_bad:
                return {"outcome": "guard_regressed",
                        "reason": f"the candidate fails cases that the base passes: {cand_bad} (suite is also non-discriminating)"}
            return {"outcome": "non_discriminating",
                    "reason": "the base already passes every finding case (not a regression suite); the candidate was evaluated "
                              "(--candidate-always) and passes every case, so it shows no regression on this suite"}
        return {"outcome": "non_discriminating",
                "reason": "the base already passes every finding case; this suite does not capture the problem and is not a regression suite"}
    if not attempts:
        return {"outcome": "base_only", "reason": f"{len(base_fail_f)} finding case(s) fail on the base; no candidate given"}
    last = attempts[-1]
    cand_fail_g = failed(last["per_case"], g_ids)
    if cand_fail_g:
        return {"outcome": "guard_regressed", "reason": f"the candidate breaks guards: {cand_fail_g}"}
    cand_fail_f = failed(last["per_case"], f_ids)
    if cand_fail_f:
        return {"outcome": "not_fixed", "reason": f"finding cases still failing on the candidate: {cand_fail_f}"}
    if base.get("verdict") == "absent":
        return {"outcome": "regression_suite_proven",
                "reason": f"the new agent does not exist on the base (all {len(f_ids)} finding cases fail by absence, not measured) "
                          "and every finding case and guard passes natively on the candidate"}
    return {"outcome": "regression_suite_proven",
            "reason": f"{len(base_fail_f)}/{len(f_ids)} finding cases fail on the base and all pass on the candidate; guards pass on both"}


def has_generated_probes(bundle: dict) -> bool:
    return any(p["kind"] == "generated_contains" for p in bundle.get("probes", []))


def native_binding(bundle: dict, base_native: dict, control: dict | None) -> dict:
    """Did agent-core's `evaluate` exercise the CANDIDATE prompt? Only prompt bundles ask. The control is a text-identical version
    bump of the base prompt: with the evaluation bound to the evaluated closure (agent-core PR 50) it passes the same native
    assertions as the base; without it the bumped ref cannot be resolved, the responder falls back to the template
    (`fallback_used` true) and the control FAILS what the base passes. Never guessed: `unknown` when the control is missing or
    infrastructure failed, or when the base itself does not pass the native assertions."""
    if not has_generated_probes(bundle):
        return {"state": "not_applicable"}
    if control is None:
        return {"state": "unknown", "why": "no control run"}
    if control.get("verdict") == "failed_infra" or not (control.get("per_case_native") or {}):
        return {"state": "unknown", "why": "the control run produced no result", "control_proposal_id": control.get("proposal_id")}
    f_ids = bundle["finding_case_ids"]
    base_ok = all(base_native.get(i, {}).get("passed") for i in f_ids)
    ctl_failed = [i for i in f_ids if not (control["per_case_native"].get(i) or {}).get("passed")]
    out = {"control_proposal_id": control.get("proposal_id"), "control": "text-identical version bump of the base prompt",
           "control_failed_native": ctl_failed}
    if not base_ok:
        return {**out, "state": "unknown", "why": "the base does not pass the native assertions: the control proves nothing"}
    return {**out, "state": "native_not_candidate_bound" if ctl_failed else "candidate_bound"}


def coverage(bundle: dict, binding: dict) -> dict:
    """What was measured natively (agent-core scorer), by harness probe, and what was NOT measured: closed codes the dossier renders."""
    mech = bundle["mechanism"]
    native = ["platform_guardrails", "guards"]
    probe: list[str] = []
    not_measured = ["real_customer_effect"]
    if mech == "status_message_gap":
        native.insert(0, "flow_outcome_and_placeholder_render")
        probe.append("state_reflected")
        not_measured.append("native_wording")
    elif mech == "closing_followup":
        native.insert(0, "flow_outcome")
        probe.append("generated_followup")
        not_measured.append("native_wording")
        if binding.get("state") == "candidate_bound":
            native.insert(1, "response_from_model_path")
        else:
            not_measured.append("candidate_prompt_native")
    elif mech == "uncovered_topic":
        native.insert(0, "new_agent_intake_and_handoff")
        not_measured += ["base_by_absence", "routing_recepcion_to_new_agent", "traffic_stealing", "native_wording"]
    assumptions = ["release_settings_assumed"] if mech == "uncovered_topic" else []
    return {"native": native, "harness_probe": probe, "not_measured": not_measured, "assumptions": assumptions}


def _why(a: dict) -> str:
    bad = [i for i in a["failed_cases"] + a["guards_failed"]]
    src = sorted({a["per_case"][i]["source"] for i in bad})
    return f"{len(bad)} casos ({', '.join(src)})"


def story_text(lang: str, d: dict, n_base_fail: int, n_cases: int, attempts: list[dict]) -> str:
    seq = []
    for a in attempts:
        if a["failed_cases"] or a["guards_failed"]:
            seq.append(("intento %d fallo en %s" if lang == "es" else "tentativa %d falhou em %s") % (a["attempt"], _why(a)))
        else:
            seq.append(("intento %d paso" if lang == "es" else "tentativa %d passou") % a["attempt"])
    head = {"es": f"La base falla {n_base_fail} de {n_cases} casos de la suite; ",
            "pt": f"A base falha {n_base_fail} de {n_cases} casos da suite; "}[lang]
    return head + "; ".join(seq) + f". [{d['outcome']}]"


def verdict_story(bundle: dict, base: dict, attempts: list[dict], binding: dict | None = None) -> dict:
    d = decide(bundle, base, attempts)
    binding = binding or {"state": "not_applicable"}
    f_ids = bundle["finding_case_ids"]
    n_base_fail = len(failed(base["per_case"], f_ids)) if base["per_case"] else 0
    proven = d["outcome"] == "regression_suite_proven"
    last = attempts[-1] if attempts else None
    announce = bool(proven and last and last["verdict"] == "pass")
    if proven and binding.get("state") == "native_not_candidate_bound":
        # Native evidence of the candidate prompt is missing: the harness probe alone is NOT enough to announce.
        d = {"outcome": "native_not_candidate_bound",
             "reason": "agent-core `evaluate` did not exercise the candidate prompt (a text-identical version bump failed the native "
                       "assertions that the base passes): the native result says nothing about the candidate. The harness probe alone "
                       f"({n_base_fail}/{len(f_ids)} finding cases fail on the base, all pass on the candidate) is labelled and not enough to announce"}
        announce, proven = False, False
    story = {
        "schema": STORY_SCHEMA, "finding_key": bundle["finding_key"], "finding_id": bundle.get("finding_id"),
        "target": bundle["target"], "mechanism": bundle["mechanism"], "agent": bundle["agent"],
        "suite_id": bundle["suite"]["id"], "suite_is_regression_suite": proven,
        "outcome": d["outcome"], "reason": d["reason"], "announce": announce,
        "base": base, "attempts": attempts,
        "gate_items": (last or base).get("gate_items", []),
        "story_text": {lang: story_text(lang, d, n_base_fail, len(f_ids), attempts) for lang in ("es", "pt")} if attempts else {},
        "model_policy": MODEL_POLICY,
        "native_binding": binding, "coverage": coverage(bundle, binding),
    }
    if binding.get("state") == "native_not_candidate_bound":
        story["probe_only_proven"] = d["outcome"] == "native_not_candidate_bound"
    if bundle.get("new_agent"):
        story["new_agent"] = bundle["new_agent"]
    return story


# ------------------------------------------------------------------------------------------------------------ live
def _attach_module():
    spec = importlib.util.spec_from_file_location("attach_eval_suite", REPO / "scripts" / "dev-stack" / "attach_eval_suite.py")
    mod = importlib.util.module_from_spec(spec)
    sys.modules["attach_eval_suite"] = mod
    spec.loader.exec_module(mod)
    return mod


def native_from_report(rep: dict, case_ids: list[str]) -> dict[str, dict]:
    """Per-scenario pass/fail from agent-core's EvalReport: `runs.cand_on_new.scenarios` (a scenario passes only if all its
    repetitions pass) plus the first failing repetition's reasons from `results`."""
    scen = ((rep.get("runs") or {}).get("cand_on_new") or {}).get("scenarios") or {}
    reasons: dict[str, list[str]] = {}
    for r in rep.get("results") or []:
        sc = r.get("score") or {}
        if r.get("run") == "cand_on_new" and not sc.get("passed", True):
            reasons.setdefault(r["scenario_id"], sc.get("failures") or [])
    out = {}
    for cid in case_ids:
        if cid in scen:
            out[cid] = {"passed": bool(scen[cid]), "reason": "; ".join(reasons.get(cid, []))[:400]}
    return out


def run_draft(api, suite: dict, agent_entity: dict | None, changes: list[dict], title: str, attach) -> dict:
    """Create a manual proposal, put suite+changes in the draft, validate, freeze, evaluate. Never approves."""
    suite, _added = attach.with_default_thresholds(suite, agent_entity or {})
    st, p = api.call("POST", "/proposals", {"agent_id": suite["agent_id"], "origin": "manual", "title": title[:120]})
    if st != 201:
        return {"verdict": "failed_infra", "problem": {"step": "create", "http": st, "body": p}, "per_case_native": {}}
    pid = p["proposal_id"]
    st, g = api.call("GET", f"/proposals/{pid}")
    rev = (g.get("proposal") or g).get("rev", 0)
    draft = list(changes) + [{"kind": "eval_suite", "content": suite, "docs": {
        "description": f'Synthetic regression eval_suite {suite["id"]} for {suite["agent_id"]}',
        "rationale": "Regression suite derived from a detected finding; must fail on the base and pass with the candidate.",
        "changelog": "Adds the suite (new yardstick)."}}]
    for c in changes:  # a patch change needs docs agent-core accepts; keep the proposer's own
        c.setdefault("docs", {"description": "candidate patch", "rationale": "see proposal", "changelog": "patch"})
    res: dict = {"proposal_id": pid}
    st, body = api.call("PUT", f"/proposals/{pid}/draft", {"expected_rev": rev, "changes": draft})
    res["put_draft"] = st
    if st != 200:
        res.update(verdict="failed_infra", problem={"step": "put_draft", "http": st, "code": body.get("code"), "detail": body.get("detail"),
                                                   "violations": body.get("violations")}, per_case_native={})
        return res
    st, body = api.call("POST", f"/proposals/{pid}/validate")
    res["validate"] = {"http": st, "valid": body.get("valid"), "violations": body.get("violations") or None}
    if st != 200 or not body.get("valid", True):
        res.update(verdict="failed_infra", problem={"step": "validate", "http": st, "violations": body.get("violations"),
                                                   "code": body.get("code"), "detail": body.get("detail")}, per_case_native={})
        return res
    st, body = api.call("POST", f"/proposals/{pid}/freeze")
    res["freeze"] = st
    if st != 200:
        res.update(verdict="failed_infra", problem={"step": "freeze", "http": st, "code": body.get("code"), "detail": body.get("detail")},
                   per_case_native={})
        return res
    st, body = api.call("POST", f"/proposals/{pid}/evaluate", {"suite_id": suite["id"], "suite_version": suite["version"]})
    res["evaluate_http"] = st
    rep = body.get("payload") if st == 409 and "payload" in body else body
    if st not in (200, 409) or "verdict" not in (rep or {}):
        res.update(verdict="failed_infra", problem={"step": "evaluate", "http": st, "code": body.get("code"), "detail": body.get("detail")},
                   per_case_native={})
        return res
    res["verdict"] = rep["verdict"]
    res["gate_items"] = [{"metric": i["metric_id"], "phase": i["phase"], "passed": i["passed"], "value": i.get("value"),
                          "reason": i.get("reason") or None} for i in rep.get("items") or []]
    res["per_case_native"] = native_from_report(rep, [s["id"] for s in suite["scenarios"]])
    res["detail"] = rep.get("detail")
    res["auto_bumped"] = body.get("auto_bumped")
    return res


def control_changes(changes: list[dict], base_artifacts: dict) -> list[dict]:
    """The binding control: the candidate's prompt entities with the BASE texts and the candidate's version (a text-identical bump)."""
    out = []
    for c in changes:
        if c.get("kind") != "prompt":
            continue
        content = dict(c["content"])
        content["locales"] = texts_of([], base_artifacts, content["id"], "prompt") or content.get("locales")
        out.append({**c, "content": content})
    return out


def run_control(api, bundle, agent_entity, changes, base_artifacts, attach) -> dict:
    """Raw native run of the control (no probes): used only to decide `native_binding`."""
    for _try in range(INFRA_RETRIES + 1):
        run = run_draft(api, bundle["suite"], agent_entity, control_changes(changes, base_artifacts),
                        f"REG1 control {bundle['suite']['id']}", attach)
        if run["verdict"] != "failed_infra":
            break
        time.sleep(INFRA_BACKOFF_S)
    return run


def evaluate_one(label: str, api, bundle, agent_entity, changes, base_artifacts, attach, attempt: int | None, generate=None,
                 ignore_native: frozenset = frozenset()) -> dict:
    retries = []
    for _try in range(INFRA_RETRIES + 1):
        run = run_draft(api, bundle["suite"], agent_entity, changes, f"REG1 {label} {bundle['suite']['id']}", attach)
        if run["verdict"] != "failed_infra":
            break
        retries.append({"proposal_id": run.get("proposal_id"), "detail": str(run.get("detail") or run.get("problem"))[:200]})
        time.sleep(INFRA_BACKOFF_S)
    per_case = (case_results(bundle, run.get("per_case_native", {}), changes, base_artifacts, generate, ignore_native)
                if run["verdict"] != "failed_infra" else {})
    f_ids, g_ids = bundle["finding_case_ids"], bundle["guard_case_ids"]
    out = {"label": label, "proposal_id": run.get("proposal_id"), "verdict": run["verdict"],
           "native_verdict": run["verdict"], "per_case": per_case,
           "failed_cases": failed(per_case, f_ids) if per_case else [], "guards_failed": failed(per_case, g_ids) if per_case else [],
           "gate_items": run.get("gate_items", []), "problem": run.get("problem"), "detail": run.get("detail"),
           "infra_retries": retries, "native": {k: v["passed"] for k, v in (run.get("per_case_native") or {}).items()}}
    if ignore_native and per_case and not out["failed_cases"] and not out["guards_failed"]:
        out["verdict"] = "probe_only_pass"
    # the case-level verdict also needs the wording probes: a native pass with a failing probe is a fail
    elif per_case and out["verdict"] == "pass" and (out["failed_cases"] or out["guards_failed"]):
        out["verdict"] = "fail"
        out["verdict_note"] = "native evaluate passed; wording probe(s) failed"
    if changes:
        out["candidate_digest"] = hashlib.sha256(json.dumps(changes, sort_keys=True, ensure_ascii=False).encode()).hexdigest()[:16]
    if attempt is not None:
        out["attempt"] = attempt
    return out


def make_generator(gateway: str, env_file: Path | None, model: str, temperature: float = 0.7):
    """Sample the LOCAL llm-gateway /v1/generate the way agent-core's adapter does. The consumer token comes from this process'
    environment (`GATEWAY_TOKEN_AGENT_CORE`) or, if absent, from the env file, into this process only; it is sent as a bearer
    header to the local gateway and is never printed or stored."""
    import os
    import urllib.error
    import urllib.request
    token = os.environ.get("GATEWAY_TOKEN_AGENT_CORE", "")
    if not token and env_file is not None and env_file.exists():
        for line in env_file.read_text(encoding="utf-8-sig").splitlines():
            if line.startswith("GATEWAY_TOKEN_AGENT_CORE="):
                token = line.split("=", 1)[1].strip().strip("\"'")

    def generate(prompt_text: str, inputs: dict, locale: str, n: int) -> list[str]:
        outs = []
        for _ in range(n):
            body = {"prompt": prompt_text, "inputs": inputs, "labels": {"prompt": "reg1-probe"},
                    "profile": {"endpoint_alias": "openrouter", "model": model, "temperature": temperature, "max_tokens": 400,
                                "timeout_s": 40, "structured": "prompted",
                                "price": {"input_per_mtok": "0.1", "output_per_mtok": "0.3"}}}
            req = urllib.request.Request(gateway.rstrip("/") + "/v1/generate", data=json.dumps(body).encode(),
                                         headers={"Authorization": "Bearer " + token, "Content-Type": "application/json"})
            try:
                with urllib.request.urlopen(req, timeout=90) as r:
                    outs.append(str(json.loads(r.read()).get("output", "")))
            except urllib.error.HTTPError as err:
                generate.last_error = f"http {err.code} {err.read()[:160].decode('utf-8', 'replace')}"
                return []  # not measured: the case fails with "not measured", never a silent pass
            except (urllib.error.URLError, OSError, ValueError) as err:
                generate.last_error = type(err).__name__
                return []
        return outs
    generate.last_error = ""
    return generate


def load_candidate(path: Path) -> list[dict]:
    d = json.loads(path.read_text(encoding="utf-8"))
    if isinstance(d, dict):
        d = d.get("changes") or (d.get("proposal") or {}).get("changes")
    if not isinstance(d, list) or not d:
        sys.exit(f"{path}: no `changes` list")
    return d


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--bundle", type=Path, required=True)
    ap.add_argument("--candidate", type=Path, action="append", default=[], help="compiled patch (changes list); repeat for attempt 2")
    ap.add_argument("--base", default="http://127.0.0.1:8001")
    ap.add_argument("--state-dir", type=Path, default=REPO / ".dev-stack")
    ap.add_argument("--token-key", default="admin")
    ap.add_argument("--base-artifacts", type=Path, default=BASE_ARTIFACTS)
    ap.add_argument("--out", type=Path)
    ap.add_argument("--gateway", default="http://127.0.0.1:8092", help="LOCAL llm-gateway used by generated probes")
    ap.add_argument("--env-file", type=Path, default=REPO.parents[1] / "agent-core.env",
                    help="holds GATEWAY_TOKEN_AGENT_CORE (read into this process, never printed)")
    ap.add_argument("--candidate-always", action="store_true",
                    help="evaluate the candidate(s) even when the base passes every case (needs agent-core PR 50: evaluate binds "
                         "the gateway to the candidate closure); a non-discriminating suite stays non_discriminating, never announced")
    ap.add_argument("--probe-model", default=MODEL_POLICY["generation"])
    a = ap.parse_args(argv)
    bundle = json.loads(a.bundle.read_text(encoding="utf-8"))
    base_artifacts = json.loads(a.base_artifacts.read_text(encoding="utf-8"))
    attach = _attach_module()
    if not attach.stack_up(a.base):
        story = {"schema": STORY_SCHEMA, "outcome": "not_exercised", "reason": f"no stack at {a.base}", "announce": False,
                 "suite_id": bundle["suite"]["id"], "finding_key": bundle["finding_key"]}
        print(json.dumps(story, indent=2, ensure_ascii=False))
        return 3
    tok = json.loads((a.state_dir / "tokens.json").read_text(encoding="utf-8"))[a.token_key]
    api = attach.Api(a.base, tok)
    st, ent = api.call("GET", f"/entities/agent/{bundle['agent']}")
    agent_entity = (ent.get("content") or ent.get("spec") or ent) if st == 200 else None
    generate = (make_generator(a.gateway, a.env_file, a.probe_model)
                if any(p["kind"] == "generated_contains" for p in bundle.get("probes", [])) else None)
    base_run = evaluate_one("base", api, bundle, agent_entity, [], base_artifacts, attach, None, generate)
    binding, ignore = {"state": "not_applicable"}, frozenset()
    if has_generated_probes(bundle) and a.candidate:
        control = run_control(api, bundle, agent_entity, load_candidate(a.candidate[0]), base_artifacts, attach)
        binding = native_binding(bundle, {k: {"passed": v} for k, v in base_run["native"].items()},
                                 {"verdict": control["verdict"], "proposal_id": control.get("proposal_id"),
                                  "per_case_native": control.get("per_case_native")})
        if binding["state"] == "native_not_candidate_bound":
            ignore = frozenset(p["case_id"] for p in bundle["probes"] if p["kind"] == "generated_contains")
    attempts = []
    # a candidate is only worth evaluating if the base fails, unless --candidate-always (candidate still needs a regression check)
    if (base_run["failed_cases"] or a.candidate_always) and not base_run["guards_failed"]:
        for n, path in enumerate(a.candidate, 1):
            att = evaluate_one(f"candidate-{n}", api, bundle, agent_entity, load_candidate(path), base_artifacts, attach, n, generate,
                               ignore)
            attempts.append(att)
            if att["verdict"] in ("pass", "probe_only_pass"):
                break
    story = verdict_story(bundle, base_run, attempts, binding)
    text = json.dumps(story, indent=2, ensure_ascii=False, default=str)
    if a.out:
        a.out.parent.mkdir(parents=True, exist_ok=True)
        a.out.write_text(text, encoding="utf-8")
    print(json.dumps({k: story[k] for k in ("suite_id", "outcome", "reason", "announce")}, ensure_ascii=False))
    return 0 if story["outcome"] == "regression_suite_proven" else 1


if __name__ == "__main__":
    sys.exit(main())
