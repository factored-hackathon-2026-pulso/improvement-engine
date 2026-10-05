"""Judge raw evaluation runs into the REG1 verdict story (W11): the engine runs the agent-core protocol itself (registry-writer
`eval`), this script only turns the raw runs into the `reg1.verdict_story/1` JSON with the SAME rules as prove_fails_on_base.py
(wording probes, `decide`, story text). No network, no credentials, no clock: stdin JSON in, stdout JSON out.

    python scripts/regression/judge_story.py < judge_input.json > story.json

Input (the documented contract, schema `w11.judge_input/1`, extended by W13):
    {"bundle": <build_suite bundle>,
     "base_artifacts": {"artifacts": [{"id": "t/estado_pqr", "locales": {"es": "...", "pt": "..."}}]},   # optional: the LIVE texts
     "base": <raw run>, "attempts": [{"attempt": 1, "changes": [<EntityDraft change>, ...], "run": <raw run>}, ...],
     "control": <raw run of a text-identical version bump of the base prompt>,                            # prompt bundles (W13)
     "generated": {"<probe_key>": {"samples": [...]} | {"error": "..."}}}                                   # sample_probes.py (W13)
A raw run is `{label, proposal_id, verdict, gate_items, per_case_native{case: {passed, reason}}, detail, problem, infra_retries}`;
the base of a NEW agent is `{label: "base", verdict: "absent"}` (the agent does not exist there, nothing was run).
When `base_artifacts` is absent the byte-exact fixture baseline of the reasoning crate is used (labelled by the caller).

Output: `reg1.verdict_story/1` (see docs/dev/REGRESSION_SUITES.md) plus `"judge": "w11.judge_story/1"`. Probes of kind
`generated_contains` need a model: the engine collects the samples with sample_probes.py and passes them in `generated`; a probe
without samples is NOT measured and its case fails ("generated probe not measured"), so a prompt finding is never announced on a
probe nobody ran. The `control` decides `native_binding` (see prove_fails_on_base.native_binding): `candidate_bound` or
`native_not_candidate_bound` (agent-core evaluate ignored the candidate prompt: the native result of the candidate is not used,
the harness probe decides alone, the story is labelled and NOT announced). `coverage` lists what was measured natively, by harness
probe, and not measured.
Exit codes: 0 story printed, 2 malformed input.
"""
from __future__ import annotations

import hashlib
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import prove_fails_on_base as prove  # noqa: E402

JUDGE_SCHEMA = "w11.judge_story/1"


def lookup_generator(generated: dict):
    """A `generate` that serves the samples the engine collected (sample_probes.py); no network here."""
    def generate(text: str, inputs: dict, locale: str, n: int) -> list[str]:
        entry = (generated or {}).get(prove.probe_key(text, locale, inputs, n))
        if not entry:
            generate.last_error = "no samples collected for this text"
            return []
        if "error" in entry:
            generate.last_error = str(entry["error"])[:120]
            return []
        return list(entry.get("samples") or [])
    generate.last_error = ""
    return generate


def judged(label: str, run: dict, bundle: dict, changes: list[dict] | None, base_artifacts: dict, attempt: int | None,
           generate=None, ignore_native: frozenset = frozenset()) -> dict:
    """Same post-processing as `prove_fails_on_base.evaluate_one`, from a raw run instead of an HTTP call."""
    absent = run.get("verdict") == "absent"
    infra = run.get("verdict") == "failed_infra"
    per_case = {} if infra else prove.case_results(bundle, run.get("per_case_native") or {}, changes, base_artifacts, generate,
                                                   ignore_native, absent)
    f_ids, g_ids = bundle["finding_case_ids"], bundle["guard_case_ids"]
    out = {"label": label, "proposal_id": run.get("proposal_id"), "verdict": run.get("verdict", "failed_infra"),
           "native_verdict": run.get("verdict", "failed_infra"), "per_case": per_case,
           "failed_cases": prove.failed(per_case, f_ids) if per_case else [],
           "guards_failed": prove.failed(per_case, g_ids) if per_case else [],
           "gate_items": run.get("gate_items") or [], "problem": run.get("problem"), "detail": run.get("detail"),
           "infra_retries": run.get("infra_retries") or []}
    if ignore_native and per_case and not out["failed_cases"] and not out["guards_failed"]:
        out["verdict"] = "probe_only_pass"
        out["verdict_note"] = "native result not used for the prompt cases: evaluate did not exercise the candidate prompt"
    elif per_case and out["verdict"] == "pass" and (out["failed_cases"] or out["guards_failed"]):
        out["verdict"] = "fail"
        out["verdict_note"] = "native evaluate passed; wording probe(s) failed"
    elif per_case and ignore_native and out["verdict"] == "fail" and (out["failed_cases"] or out["guards_failed"]):
        out["verdict_note"] = "native result not used for the prompt cases; the probe or a guard failed"
    if changes:
        out["candidate_digest"] = hashlib.sha256(json.dumps(changes, sort_keys=True, ensure_ascii=False).encode()).hexdigest()[:16]
    if attempt is not None:
        out["attempt"] = attempt
    return out


def judge(doc: dict) -> dict:
    bundle = doc["bundle"]
    base_artifacts = doc.get("base_artifacts") or json.loads(prove.BASE_ARTIFACTS.read_text(encoding="utf-8"))
    generate = lookup_generator(doc.get("generated") or {}) if prove.has_generated_probes(bundle) else None
    base = judged("base", doc["base"], bundle, [], base_artifacts, None, generate)
    binding = prove.native_binding(bundle, doc["base"].get("per_case_native") or {}, doc.get("control"))
    ignore = frozenset(c["case_id"] for c in bundle.get("probes", []) if c["kind"] == "generated_contains")         if binding.get("state") == "native_not_candidate_bound" else frozenset()
    attempts = []
    if base["failed_cases"] and not base["guards_failed"]:  # a candidate is only worth judging if the base fails
        for n, a in enumerate(doc.get("attempts") or [], 1):
            attempts.append(judged(f"candidate-{n}", a["run"], bundle, a.get("changes") or [], base_artifacts, a.get("attempt", n),
                                   generate, ignore))
            if attempts[-1]["verdict"] in ("pass", "probe_only_pass"):
                break
    story = prove.verdict_story(bundle, base, attempts, binding, doc.get("settings"))
    story["judge"] = JUDGE_SCHEMA
    return story


def main() -> int:
    try:
        doc = json.loads(sys.stdin.buffer.read().decode("utf-8"))
        story = judge(doc)
    except (ValueError, KeyError, TypeError) as e:
        print(json.dumps({"error": "malformed_input", "why": type(e).__name__ + ": " + str(e)[:120]}))
        return 2
    sys.stdout.buffer.write(json.dumps(story, ensure_ascii=False, default=str).encode("utf-8"))
    return 0


if __name__ == "__main__":
    sys.exit(main())
