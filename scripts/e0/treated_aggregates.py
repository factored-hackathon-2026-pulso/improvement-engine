"""E0 treated aggregates for the maturity model. LOCAL ONLY: reads the E0 parquet (needs pyarrow), writes AGGREGATES
(counts per case type, k >= 10, no ids, no free text) as JSON under the git-ignored `output/`. Never commit the output.

    python scripts/e0/treated_aggregates.py [--e0 D:/.codex/factored/pulso_muestra_e0/datos] [--out output/e0_treated_aggregates.json]

E0 has 0 suggestion rows: draft acceptance (and stage 3 -> agent) is NOT computable from E0 and is declared as such.
Case type = E0 `case.topic` mapped to the platform case types below; other topics are dropped.
"""
import json
import sys
from collections import defaultdict
from pathlib import Path

TOPIC_TO_TYPE = {"cobro_duplicado": "cobro_indebido", "disputar_cargo": "cargo_no_reconocido", "problema_app": "problema_app"}
ALLOWED_KEYS = ["type_id", "copilot_questions", "repeat_q_cases", "tool_applicable", "tool_used"]


def aggregate(cases, queries, tool_calls, suggestion_rows, k=10):
    """Pure. cases: [{case_id, topic}], queries: [{case_id, query_signature}], tool_calls: [{case_id, actor_role}]."""
    if k < 10:
        raise ValueError("k must be >= 10")
    type_of = {c["case_id"]: TOPIC_TO_TYPE[c["topic"]] for c in cases if c["topic"] in TOPIC_TO_TYPE}
    n_cases = defaultdict(int)
    for t in type_of.values():
        n_cases[t] += 1
    sig_cases = defaultdict(lambda: defaultdict(set))  # type -> signature -> case ids
    asked = defaultdict(set)
    for q in queries:
        t = type_of.get(q["case_id"])
        if t:
            sig_cases[t][q["query_signature"]].add(q["case_id"])
            asked[t].add(q["case_id"])
    analyst_tool = {tc["case_id"] for tc in tool_calls if tc.get("actor_role") == "analyst"}
    rows, suppressed = [], 0
    for t in sorted(n_cases):
        if n_cases[t] < k:
            suppressed += 1
            continue
        sigs = sig_cases[t]
        lead = max(sigs.values(), key=len) if sigs else set()
        rows.append({
            "type_id": t,
            "copilot_questions": len(asked[t]),
            "repeat_q_cases": len(lead),
            "tool_applicable": len(lead),
            "tool_used": len(lead & analyst_tool),
        })
    return {
        "source": "e0_treated", "simulated": False, "k_min": k, "suppressed_types": suppressed, "suggestion_rows": suggestion_rows,
        "not_computable": {
            "draft_accept_100": "no_draft_rows: E0 has no suggestion rows" if suggestion_rows == 0 else None,
            "stage_3_to_agent": "depends on draft_accept_100",
        },
        "definitions": {
            "repeat_q_cases": "distinct cases carrying the most frequent copilot query_signature, per type",
            "tool_used": "of those cases, with at least one tool_call by an analyst (proxy for 'proposed tool used')",
        },
        "case_types": rows,
    }


def _read(path, cols):
    import pyarrow.parquet as pq  # local-only dependency

    return pq.read_table(path, columns=cols).to_pylist()


def main(argv):
    e0 = Path("D:/.codex/factored/pulso_muestra_e0/datos")
    out = Path("output/e0_treated_aggregates.json")
    a = iter(argv)
    for x in a:
        if x == "--e0":
            e0 = Path(next(a))
        elif x == "--out":
            out = Path(next(a))
        else:
            sys.exit(f"unknown argument {x}")
    try:
        cases = _read(e0 / "case.parquet", ["case_id", "topic"])
        queries = _read(e0 / "copilot_query.parquet", ["case_id", "query_signature"])
        tools = _read(e0 / "tool_call.parquet", ["case_id", "actor_role"])
        sugg = e0 / "suggestion.parquet"
        n_sugg = len(_read(sugg, ["case_id"])) if sugg.exists() else 0
    except ModuleNotFoundError:
        sys.exit("pyarrow is required (local only): pip install pyarrow")
    result = aggregate(cases, queries, tools, n_sugg)
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(result, indent=1, sort_keys=True), encoding="utf8")
    print(f"wrote {out} ({len(result['case_types'])} case types)")


if __name__ == "__main__":
    main(sys.argv[1:])
