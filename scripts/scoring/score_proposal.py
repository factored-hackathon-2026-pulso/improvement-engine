#!/usr/bin/env python3
"""Rubric scorer for improvement proposals (12 criteria, 0/1/2, see ARTIFACT_ANATOMY_AND_RUBRIC).

Deterministic (this module scores them): R3 minimal, R4 protected behaviours (HARD), R5 language policy
H1 (HARD), R6 evaluation exists and is independent (HARD), R7 evidence refs resolve (HARD), R11 no PII
(HARD). Judged (supplied by an optional LLM-judge hook or a human): R1, R2, R8, R9, R10, R12.

Preconditions (failure = verdict `invalid`, not `inadequate`): every anchored patch applies byte-exact to
the base text, and the target agent/entities exist in the registry export (skipped when no export is given).

Verdict: invalid | reject (hard gate < 1, or total < 14) | needs_judge (judged criteria missing) |
revise (14-18, or >= 19 with a zero) | adequate (>= 19, no zero). `human_review` is forced for flow,
decision_model, model_profile, new-agent or release-level changes whatever the score.

Judge hook: a callable `judge(request) -> {criterion: 0|1|2}`; it must be a DIFFERENT model family than
the Builder (enforced by `run_judge`), is sampled twice, never receives builder reasoning, and the module
holds no keys and never reads or prints environment values. Wire a concrete model with `--judge
module:callable`; the callable owns its own credentials.
Python standard library only.
"""
import argparse
import copy
import importlib
import json
import re
import sys

JUDGED = ("R1", "R2", "R8", "R9", "R10", "R12")
MECHANICAL = ("R3", "R4", "R5", "R6", "R7", "R11")
HARD = ("R4", "R5", "R6", "R7", "R11")
ALL = tuple(f"R{i}" for i in range(1, 13))

DENIED_KINDS = {"policy": "protected_authority", "release_settings": "protected_behaviour",
                "injection_ruleset": "protected_behaviour", "language_detection": "protected_behaviour",
                "knowledge_snapshot": "knowledge_snapshot_refused"}
HUMAN_KINDS = {"flow", "decision_model", "model_profile"}
AGENT_PROTECTED_FIELDS = ("invocable_by", "min_auth_level", "subject_kinds")
TOOL_PROTECTED_FIELDS = ("risk_class", "min_auth_level", "readback_by")
MODEL_PROTECTED_FIELDS = ("model", "endpoint_alias")
META_AGENTS = re.compile(r"^(constructor-chat|pulso-.*)$")
# Safety clause markers; a marker present in the base text must survive in the candidate text.
DEFAULT_MARKERS = ("datos_no_confiables", "solo lees", "read-only", "otro cliente")

PII_PATTERNS = (
    ("pii_token", re.compile("⟦[a-z]{1,12}:[0-9]+⟧")),
    ("email", re.compile(r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}")),
    ("digit_run", re.compile(r"\d{6,}")),
)

FAMILIES = (("deepseek", "deepseek"), ("anthropic", "anthropic"), ("claude", "anthropic"), ("openai", "openai"),
            ("gpt", "openai"), ("google", "google"), ("gemini", "google"), ("meta", "meta"),
            ("llama", "meta"), ("mistral", "mistral"), ("qwen", "qwen"), ("cohere", "cohere"), ("xai", "xai"),
            ("grok", "xai"))
REASONING_KEYS = {"reasoning", "builder_reasoning", "chain_of_thought", "scratchpad"}


class PatchError(ValueError):
    pass


class JudgeError(ValueError):
    pass


# --- patches --------------------------------------------------------------------------------------------

def apply_patch(text, hunks):
    """Apply anchored hunks. Each hunk's `anchor + old` must occur exactly once (byte-exact, no
    whitespace tolerance); it is replaced by `anchor + new`. Hunks apply in order on the evolving text."""
    for i, h in enumerate(hunks):
        anchor, old, new = h.get("anchor", ""), h.get("old"), h.get("new")
        if not isinstance(old, str) or not isinstance(new, str) or not isinstance(anchor, str):
            raise PatchError(f"hunk {i}: anchor/old/new must be strings")
        needle = anchor + old
        if not needle:
            raise PatchError(f"hunk {i}: empty anchor and old text")
        n = text.count(needle)
        if n != 1:
            raise PatchError(f"hunk {i}: anchored text found {n} times (expected exactly 1)")
        text = text.replace(needle, anchor + new, 1)
    return text


def _base_text(base, entity_id, locale):
    ent = (base.get("entities") or {}).get(entity_id)
    if ent is None:
        raise PatchError(f"base entity {entity_id!r} not found")
    content = ent.get("content") or {}
    if locale is not None:
        loc = (content.get("locales") or {}).get(locale)
        if not isinstance(loc, str):
            raise PatchError(f"base entity {entity_id!r} has no locale {locale!r}")
        return loc
    t = content.get("text")
    if not isinstance(t, str):
        raise PatchError(f"base entity {entity_id!r} has no text")
    return t


def check_patches(proposal, base):
    problems = []
    for i, ch in enumerate(proposal.get("changes") or []):
        patch = ch.get("patch")
        if not patch:
            continue
        try:
            before = _base_text(base, patch.get("entity_id"), patch.get("locale"))
            after = apply_patch(before, patch.get("hunks") or [])
        except PatchError as e:
            problems.append({"change": i, "code": "patch_does_not_apply", "detail": str(e)})
            continue
        cand = patch.get("candidate_text")
        if cand is not None and after.encode("utf-8") != cand.encode("utf-8"):
            problems.append({"change": i, "code": "candidate_text_mismatch",
                             "detail": "patched base differs from candidate_text"})
        loc, content = patch.get("locale"), ch.get("content") or {}
        if loc is not None and (content.get("locales") or {}).get(loc) not in (None, after):
            problems.append({"change": i, "code": "content_text_mismatch",
                             "detail": "change content differs from the patched base"})
    return {"ok": not problems, "problems": problems}


# --- PII ------------------------------------------------------------------------------------------------

def _strings(obj, path=""):
    if isinstance(obj, str):
        yield path, obj
    elif isinstance(obj, dict):
        for k, v in obj.items():
            yield from _strings(v, f"{path}.{k}")
    elif isinstance(obj, (list, tuple)):
        for i, v in enumerate(obj):
            yield from _strings(v, f"{path}[{i}]")


def scan_pii(proposal):
    """Findings carry the path and the pattern name only, never the matched value."""
    out = []
    for path, s in _strings({k: v for k, v in proposal.items() if k not in ("evidence_refs",)}):
        for name, rx in PII_PATTERNS:
            if rx.search(s):
                out.append({"path": path, "pattern": name})
    return out


# --- protected behaviours / language --------------------------------------------------------------------

def check_protected(proposal, base, markers=DEFAULT_MARKERS):
    v = []
    ents = base.get("entities") or {}
    if META_AGENTS.match(str(proposal.get("agent_id", ""))):
        v.append({"code": "meta_agent", "detail": "self/meta agents are denied"})
    for i, ch in enumerate(proposal.get("changes") or []):
        kind, content = ch.get("kind"), ch.get("content") or {}
        if kind in DENIED_KINDS:
            v.append({"change": i, "code": DENIED_KINDS[kind], "detail": f"kind {kind} is on the deny-list"})
            continue
        b = (ents.get(content.get("id")) or {}).get("content") or {}
        fields = {"agent": AGENT_PROTECTED_FIELDS, "tool": TOOL_PROTECTED_FIELDS,
                  "model_profile": MODEL_PROTECTED_FIELDS}.get(kind, ())
        for f in fields:
            if f in content and f in b and content[f] != b[f]:
                v.append({"change": i, "code": "protected_field_changed", "detail": f"{kind}.{f}"})
        if kind == "agent":
            for m in content.get("metrics") or []:
                if str(m.get("id", "")).startswith("platform_"):
                    v.append({"change": i, "code": "platform_metric", "detail": "platform_ metrics are denied"})
        for loc, new in (content.get("locales") or {}).items():
            old = (b.get("locales") or {}).get(loc)
            if isinstance(old, str) and isinstance(new, str):
                for mk in markers:
                    if mk.lower() in old.lower() and mk.lower() not in new.lower():
                        v.append({"change": i, "code": "safety_clause_removed",
                                  "detail": f"{content.get('id')}[{loc}] lost marker {mk!r}"})
    return v


def check_locales(proposal, base):
    v = []
    supported = set(base.get("supported_locales") or [])
    for i, ch in enumerate(proposal.get("changes") or []):
        kind, content = ch.get("kind"), ch.get("content") or {}
        if kind in ("prompt", "template") and supported:
            locs = content.get("locales") or {}
            if set(locs) != supported:
                v.append({"change": i, "code": "locale_set_mismatch",
                          "detail": f"{sorted(locs)} != {sorted(supported)}"})
            texts = [t for t in locs.values() if isinstance(t, str)]
            if len(locs) > 1 and len(set(texts)) < len(texts):
                v.append({"change": i, "code": "locale_copy", "detail": "two locales carry identical text"})
        if kind == "agent" and "supported_locales" in content and supported - set(content["supported_locales"]):
            v.append({"change": i, "code": "supported_locales_shrunk", "detail": "supported_locales may not shrink"})
        if kind in ("language_detection",) or content.get("unsupported"):
            v.append({"change": i, "code": "unsupported_languages_touched", "detail": "H1: pt is supported"})
    return v


# --- registry, evidence, suite --------------------------------------------------------------------------

def _registry_index(reg):
    ids = set()

    def walk(o):
        if isinstance(o, dict):
            if isinstance(o.get("id"), str):
                ids.add((o.get("kind"), o["id"]))
                ids.add((None, o["id"]))
            for v in o.values():
                walk(v)
        elif isinstance(o, list):
            for v in o:
                walk(v)
        elif isinstance(o, str):
            pass
    walk(reg)
    if isinstance(reg, dict) and isinstance(reg.get("ids"), list):
        ids.update((None, i) for i in reg["ids"] if isinstance(i, str))
    return ids


def check_target(proposal, registry):
    if registry is None:
        return {"ok": None, "status": "skipped", "detail": "no registry export supplied"}
    idx = _registry_index(registry)
    missing = []
    if (None, proposal.get("agent_id")) not in idx:
        missing.append(proposal.get("agent_id"))
    for ch in proposal.get("changes") or []:
        p = ch.get("patch")
        if p and (None, p.get("entity_id")) not in idx:
            missing.append(p.get("entity_id"))
    return {"ok": not missing, "status": "pass" if not missing else "fail", "missing": missing}


def check_evidence(proposal, store, k_min=10):
    refs = proposal.get("evidence_refs") or []
    if not refs:
        return [{"code": "no_evidence_refs", "detail": "at least one resolvable ref is required"}]
    v = []
    for r in refs:
        rec = (store or {}).get(r.get("id"))
        if rec is None:
            v.append({"code": "dangling_ref", "detail": str(r.get("id"))})
            continue
        k = rec.get("k", r.get("k"))
        if not isinstance(k, int) or k < k_min:
            v.append({"code": "k_anonymity", "detail": f"{r.get('id')}: k < {k_min}"})
    return v


def check_suite(proposal):
    s = proposal.get("suite")
    if not isinstance(s, dict):
        return [{"code": "no_suite", "detail": "suite or not_evaluable required"}]
    ne = s.get("not_evaluable")
    if ne:
        return [] if ne.get("reason") and ne.get("alternative") else [
            {"code": "not_evaluable_unjustified", "detail": "reason and alternative required"}]
    v = []
    sealed, cand = s.get("suite_digest_at"), s.get("candidate_digest_at")
    if not sealed or not cand or not (sealed < cand):  # ISO-8601 UTC strings compare lexicographically
        v.append({"code": "suite_not_sealed_first", "detail": "suite digest must predate candidate digest"})
    if not s.get("author") or s.get("author") == proposal.get("author"):
        v.append({"code": "suite_author_not_independent", "detail": "scenario author must differ from change author"})
    removed = set(s.get("base_scenarios") or []) - set(s.get("scenarios") or [])
    if removed:
        v.append({"code": "yardstick_loosened", "detail": f"{len(removed)} base scenario(s) removed"})
    if not s.get("scenarios"):
        v.append({"code": "empty_suite", "detail": "no scenarios"})
    return v


# --- judge hook -----------------------------------------------------------------------------------------

def model_family(name):
    n = str(name or "").lower().strip()
    for part in re.split(r"[/:\s]", n):
        for prefix, fam in FAMILIES:
            if part.startswith(prefix):
                return fam
    return None


def _strip_reasoning(o):
    if isinstance(o, dict):
        return {k: _strip_reasoning(v) for k, v in o.items() if k not in REASONING_KEYS}
    if isinstance(o, list):
        return [_strip_reasoning(v) for v in o]
    return o


def run_judge(judge, request, builder_model, judge_model, samples=2):
    fb, fj = model_family(builder_model), model_family(judge_model)
    if fb is None or fj is None:
        raise JudgeError("cannot establish model families; refusing to judge")
    if fb == fj:
        raise JudgeError(f"judge family {fj!r} equals Builder family; use a different family")
    req = _strip_reasoning(copy.deepcopy(request))
    runs = []
    for _ in range(samples):
        out = judge(copy.deepcopy(req))
        if not isinstance(out, dict) or not out:
            raise JudgeError("judge returned no scores")
        for c, val in out.items():
            if c not in JUDGED:
                raise JudgeError(f"judge may only score {', '.join(JUDGED)}; got {c!r}")
            if val not in (0, 1, 2) or isinstance(val, bool):
                raise JudgeError(f"score for {c} must be 0, 1 or 2")
        runs.append(out)
    scores, escalate = {}, []
    for c in JUDGED:
        vals = [r[c] for r in runs if c in r]
        if len(vals) < len(runs):
            continue
        scores[c] = min(vals)  # conservative
        if max(vals) - min(vals) > 1:
            escalate.append(c)
    return {"scores": scores, "escalate_human": escalate, "judge_family": fj, "builder_family": fb,
            "samples": len(runs)}


# --- scoring --------------------------------------------------------------------------------------------

def _human_review(proposal):
    for ch in proposal.get("changes") or []:
        if ch.get("kind") in HUMAN_KINDS or ch.get("release_level_change") or ch.get("new_agent"):
            return True
    return bool(proposal.get("release_level_change") or proposal.get("new_agent"))


def score(proposal, base, registry=None, evidence=None, judge_scores=None, markers=DEFAULT_MARKERS):
    proposal, base = copy.deepcopy(proposal), copy.deepcopy(base)
    gates = {}
    patches = check_patches(proposal, base)
    gates["patch_applies_byte_exact"] = patches
    target = check_target(proposal, registry)
    gates["target_in_registry"] = target
    protected = check_protected(proposal, base, markers)
    locales = check_locales(proposal, base)
    pii = scan_pii(proposal)
    ev = check_evidence(proposal, evidence)
    suite = check_suite(proposal)
    gates["protected_behaviours"] = {"ok": not protected, "violations": protected}
    gates["language_policy_h1"] = {"ok": not locales, "violations": locales}
    gates["evaluation_independent"] = {"ok": not suite, "violations": suite}
    gates["evidence_resolves"] = {"ok": not ev, "violations": ev}
    gates["no_pii"] = {"ok": not pii, "violations": pii}

    n_changes = len([c for c in proposal.get("changes") or [] if c.get("kind") != "eval_suite"])
    crit = {c: None for c in ALL}
    crit["R3"] = 2 if n_changes <= 3 else 1 if n_changes <= 6 else 0
    crit["R4"] = 0 if protected else 2
    crit["R5"] = 0 if locales else 2
    crit["R6"] = 0 if suite else 2
    crit["R7"] = 0 if ev else 2
    crit["R11"] = 0 if pii else 2
    for c, v in (judge_scores or {}).items():
        if c in JUDGED:
            crit[c] = v

    problems = []
    if not patches["ok"]:
        problems.append("patch_does_not_apply")
    if target.get("ok") is False:
        problems.append("target_not_in_registry")
    scored = [v for v in crit.values() if v is not None]
    total = sum(scored)
    missing = [c for c in JUDGED if crit[c] is None]
    gate_fail = [c for c in HARD if crit[c] is not None and crit[c] < 1]
    if problems:
        verdict = "invalid"
    elif gate_fail:
        verdict = "reject"
    elif missing:
        verdict = "needs_judge"
    elif total < 14:
        verdict = "reject"
    elif total >= 19 and 0 not in scored:
        verdict = "adequate"
    else:
        verdict = "revise"
    return {"proposal_id": proposal.get("proposal_id"), "criteria": crit, "total": total if not missing else None,
            "partial_total": total, "max": 24, "gates": gates, "hard_gate_failures": gate_fail,
            "invalid_reasons": problems, "missing_judged": missing, "verdict": verdict,
            "human_review": _human_review(proposal)}


def _load(path):
    with open(path, encoding="utf-8") as f:
        return json.load(f)


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--proposal", required=True)
    ap.add_argument("--base", required=True, help="JSON {entities: {id: {kind, content}}, supported_locales}")
    ap.add_argument("--registry", help="agent-core registry export (JSON); target check skipped without it")
    ap.add_argument("--evidence", help="JSON {ref_id: {k: int}} store")
    ap.add_argument("--judge", help="module:callable judge hook (owns its own credentials)")
    ap.add_argument("--builder-model")
    ap.add_argument("--judge-model")
    ap.add_argument("--out")
    a = ap.parse_args(argv)
    try:
        proposal, base = _load(a.proposal), _load(a.base)
        registry = _load(a.registry) if a.registry else None
        evidence = _load(a.evidence) if a.evidence else {}
        judged, escalate = None, []
        if a.judge:
            mod, _, fn = a.judge.partition(":")
            judge = getattr(importlib.import_module(mod), fn)
            req = {"proposal": {k: v for k, v in proposal.items()}, "base": base,
                   "rubric_criteria": list(JUDGED)}
            res = run_judge(judge, req, a.builder_model, a.judge_model)
            judged, escalate = res["scores"], res["escalate_human"]
        result = score(proposal, base, registry, evidence, judged)
        if escalate:
            result["judge_escalate_human"] = escalate
            result["human_review"] = True
    except (OSError, ValueError, ImportError, AttributeError) as e:
        print(f"score_proposal: {type(e).__name__}: {e}", file=sys.stderr)
        return 2
    text = json.dumps(result, indent=2, sort_keys=True)
    if a.out:
        with open(a.out, "w", encoding="utf-8") as f:
            f.write(text + "\n")
    else:
        print(text)
    return 0


if __name__ == "__main__":
    sys.exit(main())
