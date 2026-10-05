#!/usr/bin/env python3
"""Compare pass-1 rubric labels with row-level judge scores. Stdlib only."""

import argparse
import json
import re
import sys

JUDGED = ("R1", "R2", "R8", "R9", "R10", "R12")
ALL = tuple(f"R{i}" for i in range(1, 13))
HARD = ("R4", "R5", "R6", "R7", "R11")
EMAIL = re.compile(r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}")
DIGITS = re.compile(r"\d{6,}")
CANARY = re.compile(r"⟦[a-z]{1,12}:[0-9]+⟧")


def _strings(value):
    if isinstance(value, str):
        yield value
    elif isinstance(value, dict):
        for item in value.values():
            yield from _strings(item)
    elif isinstance(value, list):
        for item in value:
            yield from _strings(item)


def _kappa(a, b):
    if not a:
        return None
    observed = sum(x == y for x, y in zip(a, b)) / len(a)
    labels = (0, 1, 2)
    expected = sum((a.count(k) / len(a)) * (b.count(k) / len(b)) for k in labels)
    if expected == 1:
        return None
    return round((observed - expected) / (1 - expected), 6)


def validate_golden(golden, artifact_index):
    """Return structural/provenance errors; do not inspect or score proposal quality."""
    errors = []
    items = golden.get("proposals") if isinstance(golden, dict) else None
    if not isinstance(items, list):
        return ["proposals must be an array"]
    agents = set(artifact_index.get("agents", []))
    entities = {a.get("id") for a in artifact_index.get("artifacts", []) if isinstance(a, dict)}
    evidence_index = {e.get("id"): e.get("k") for e in artifact_index.get("synthetic_evidence", [])
                      if isinstance(e, dict) and isinstance(e.get("id"), str)}
    by_id, by_pair = {}, {}
    for item in items:
        if not isinstance(item, dict):
            errors.append("proposal entry must be an object")
            continue
        pid = item.get("id")
        if not isinstance(pid, str) or not pid:
            errors.append("proposal id must be a non-empty string")
            continue
        if pid in by_id:
            errors.append(f"duplicate proposal id {pid}")
        by_id[pid] = item
        pair_id, variant = item.get("pair_id"), item.get("variant")
        if not isinstance(pair_id, str) or variant not in ("good", "bad"):
            errors.append(f"{pid}: pair_id/variant invalid")
        else:
            by_pair.setdefault(pair_id, []).append(item)
        prop = item.get("proposal")
        if not isinstance(prop, dict) or prop.get("id") != pid:
            errors.append(f"{pid}: proposal payload id mismatch")
            continue
        if prop.get("agent_id") not in agents:
            errors.append(f"{pid}: agent_id is not in the fixture index")
        change = prop.get("artifact_change")
        if not isinstance(change, dict):
            errors.append(f"{pid}: artifact_change is required")
        else:
            kind = change.get("kind")
            if kind == "new_agent":
                refs = change.get("related_artifact_ids", [])
                if not refs or any(ref not in entities for ref in refs):
                    errors.append(f"{pid}: new-agent references must resolve to fixture artifact ids")
            elif change.get("target_id") not in entities:
                errors.append(f"{pid}: target_id is not in the fixture index")
            if kind not in ("patch", "template", "new_agent"):
                errors.append(f"{pid}: unsupported artifact kind {kind!r}")
        labels = item.get("labels")
        if not isinstance(labels, dict) or set(labels) != set(ALL):
            errors.append(f"{pid}: labels must contain R1-R12 exactly")
        else:
            for criterion, score in labels.items():
                if isinstance(score, bool) or not isinstance(score, int) or score not in (0, 1, 2):
                    errors.append(f"{pid}: {criterion} must be integer 0, 1, or 2")
        expected = item.get("expected")
        if isinstance(labels, dict) and isinstance(expected, dict):
            if expected != {c: labels[c] for c in JUDGED if c in labels}:
                errors.append(f"{pid}: expected must match the six judge-scored labels")
        else:
            errors.append(f"{pid}: expected judge labels are required")
        for text in _strings(prop):
            scrubbed = CANARY.sub("", text)
            if EMAIL.search(scrubbed) or DIGITS.search(scrubbed):
                errors.append(f"{pid}: proposal text has email or six-digit run; only synthetic canary tokens allowed")
                break
        refs = prop.get("evidence_refs") or []
        if not isinstance(refs, list) or not refs:
            errors.append(f"{pid}: evidence_refs must be a non-empty array")
        else:
            invalid_refs = []
            for ref in refs:
                if not isinstance(ref, dict) or ref.get("id") not in evidence_index:
                    invalid_refs.append(ref.get("id") if isinstance(ref, dict) else "<malformed>")
                elif ref.get("k") != evidence_index[ref["id"]] or evidence_index[ref["id"]] < 10:
                    invalid_refs.append(ref["id"])
            if item.get("variant") == "good" and invalid_refs:
                errors.append(f"{pid}: positive example evidence must resolve with k>=10")
            if item.get("variant") == "bad" and invalid_refs and isinstance(labels, dict) and labels.get("R7") != 0:
                errors.append(f"{pid}: invalid negative-example evidence must be labelled R7=0")
    for pair_id, pair in by_pair.items():
        if len(pair) != 2 or {x.get("variant") for x in pair} != {"good", "bad"}:
            errors.append(f"{pair_id}: must have exactly one good and one bad proposal")
        elif pair[0].get("proposal", {}).get("finding") != pair[1].get("proposal", {}).get("finding"):
            errors.append(f"{pair_id}: matched proposals must address the same finding")
    return errors


def compare(golden, judge_output):
    """Compare row-level scores or pass through legacy aggregate-only summaries honestly."""
    if not isinstance(judge_output, dict) or judge_output.get("format") != "pulso.judge-rows.v1":
        if isinstance(judge_output, dict) and judge_output.get("status") == "not_exercised":
            return {
                "status": "not_exercised", "reason": judge_output.get("reason"),
                "exact_agreement": None, "within_one_agreement": None,
                "hard_gate_agreement": None, "proposal_gate_agreement": None,
                "cohen_kappa_unweighted": None,
                "not_comparable_to_current_golden": True,
            }
        aggregate_fields = ("exact", "within_1", "hard_gate")
        if isinstance(judge_output, dict) and judge_output.get("status") == "exercised" \
                and all(isinstance(judge_output.get(key), (int, float)) for key in aggregate_fields):
            for key in aggregate_fields:
                if isinstance(judge_output[key], bool) or not 0 <= judge_output[key] <= 1:
                    raise ValueError(f"aggregate {key} must be between 0 and 1")
            proposal_gate = judge_output.get("proposal_gate_agreement")
            if proposal_gate is not None and (isinstance(proposal_gate, bool)
                                              or not isinstance(proposal_gate, (int, float))
                                              or not 0 <= proposal_gate <= 1):
                raise ValueError("aggregate proposal_gate_agreement must be null or between 0 and 1")
            return {
                "status": "aggregate_only_not_comparable",
                "source_status": "exercised",
                "source_proposals": judge_output.get("proposals"),
                "source_scored_pairs": judge_output.get("pairs"),
                "exact_agreement": judge_output["exact"],
                "within_one_agreement": judge_output["within_1"],
                "hard_gate_agreement": judge_output["hard_gate"],
                "proposal_gate_agreement": proposal_gate,
                "per_criterion": judge_output.get("per_criterion"),
                "cohen_kappa_unweighted": None,
                "cohen_kappa_note": "Aggregate-only output omits agreeing rows; kappa cannot be reconstructed and these values are not comparable to the current pass-1 labels.",
                "not_comparable_to_current_golden": True,
            }
        raise ValueError("row-level judge output or a recognized legacy aggregate summary is required")
    items = golden.get("proposals") or []
    pass1 = {item["id"]: item["expected"] for item in items}
    expected_keys = {(pid, criterion) for pid, scores in pass1.items() for criterion in scores}
    rows = judge_output.get("rows")
    if not isinstance(rows, list):
        raise ValueError("row-level judge output must contain a rows array")
    judged, seen = {}, set()
    for row in rows:
        if not isinstance(row, dict):
            raise ValueError("judge row must be an object")
        pid, criterion, score = row.get("id"), row.get("criterion"), row.get("score")
        key = (pid, criterion)
        if key in seen:
            raise ValueError(f"duplicate judge row for {pid}/{criterion}")
        seen.add(key)
        if key not in expected_keys:
            raise ValueError(f"unexpected judge row {pid}/{criterion}")
        if criterion not in JUDGED or isinstance(score, bool) or not isinstance(score, int) or score not in (0, 1, 2):
            raise ValueError(f"invalid judge score for {pid}/{criterion}")
        judged[key] = score
    paired = [(pass1[pid][c], judged[(pid, c)], pid, c)
              for pid, c in expected_keys if (pid, c) in judged]
    n_expected, n = len(expected_keys), len(paired)
    exact = sum(h == j for h, j, _, _ in paired)
    within = sum(abs(h - j) <= 1 for h, j, _, _ in paired)
    zero_gate = sum((h == 0) == (j == 0) for h, j, _, _ in paired)
    by_criterion = {}
    for c in JUDGED:
        subset = [(h, j) for h, j, _, criterion in paired if criterion == c]
        by_criterion[c] = {
            "n": len(subset),
            "exact_agreement": round(sum(h == j for h, j in subset) / len(subset), 4) if subset else None,
            "within_one_agreement": round(sum(abs(h-j) <= 1 for h, j in subset) / len(subset), 4) if subset else None,
            "hard_gate_agreement": round(sum((h == 0) == (j == 0) for h, j in subset) / len(subset), 4) if subset else None,
        }
    pairs, pairwise = {}, []
    for item in items:
        pairs.setdefault(item["pair_id"], {})[item["variant"]] = item
    for pair_id, pair in pairs.items():
        if set(pair) != {"good", "bad"}:
            continue
        good, bad = pair["good"], pair["bad"]
        criteria = JUDGED
        keys = [(pid, c) for pid in (good["id"], bad["id"]) for c in criteria]
        if all(key in judged for key in keys):
            h_delta = sum(good["expected"][c] - bad["expected"][c] for c in criteria)
            j_delta = sum(judged[(good["id"], c)] - judged[(bad["id"], c)] for c in criteria)
            pass1_order = (h_delta > 0) - (h_delta < 0)
            judge_order = (j_delta > 0) - (j_delta < 0)
            pairwise.append({"pair_id": pair_id, "pass1_order": pass1_order,
                             "judge_order": judge_order, "agree": pass1_order == judge_order})
    return {
        "status": "exercised" if n else "not_exercised",
        "label_source": "pass1_unblinded_codex_labels",
        "judge_rows_format": judge_output["format"],
        "n_expected": n_expected,
        "n_compared": n,
        "n_missing_judge_rows": n_expected - n,
        "coverage": round(n / n_expected, 4) if n_expected else None,
        "exact_agreement": round(exact / n, 4) if n else None,
        "within_one_agreement": round(within / n, 4) if n else None,
        # Matches legacy zero-vs-zero on judged criteria; not the proposal hard-gate implementation.
        "hard_gate_agreement": round(zero_gate / n, 4) if n else None,
        "cohen_kappa_unweighted": _kappa([r[0] for r in paired], [r[1] for r in paired]),
        "cohen_kappa_note": "Nominal pooled 0/1/2 labels; repeated criteria within proposals are dependent, so descriptive only.",
        "per_criterion": by_criterion,
        "pairwise_n": len(pairwise),
        "pairwise_winner_agreement": round(sum(p["agree"] for p in pairwise) / len(pairwise), 4) if pairwise else None,
        "pairwise": pairwise,
        "mismatches": [{"id": pid, "criterion": c, "pass1": h, "judge": j}
                       for h, j, pid, c in paired if h != j],
    }


def compare_labelers(pass1, manifest, pass2):
    """Compare full R1-R12 labels from two raters after unblinding opaque IDs."""
    if not isinstance(pass1, dict) or not isinstance(manifest, dict) or not isinstance(pass2, dict):
        raise ValueError("pass1, manifest, and pass2 must be objects")
    if manifest.get("format") == "pulso.blind-label-manifest.v1":
        manifest = manifest.get("items")
        if not isinstance(manifest, dict):
            raise ValueError("blind manifest items must be an object")
    scores2 = pass2.get("scores")
    if not isinstance(scores2, dict) or set(scores2) != set(manifest):
        raise ValueError("pass-2 case IDs must exactly match the blind manifest case IDs")
    if len(manifest) != len(pass1):
        raise ValueError("case IDs and pass-1 proposals must have equal cardinality")
    if any(not isinstance(link, dict) for link in manifest.values()):
        raise ValueError("manifest entries must be objects")
    source_ids = [link.get("source_id") for link in manifest.values()]
    if len(source_ids) != len(set(source_ids)) or set(source_ids) != set(pass1):
        raise ValueError("manifest must map each pass-1 proposal exactly once")

    observed = []
    by_proposal = {}
    by_pair = {}
    for case_id, link in manifest.items():
        source_id, pair_id, variant = link.get("source_id"), link.get("pair_id"), link.get("variant")
        if source_id not in pass1 or variant not in ("good", "bad") or not isinstance(pair_id, str):
            raise ValueError(f"{case_id}: manifest does not resolve to a paired pass-1 proposal")
        first = pass1[source_id].get("scores")
        second = scores2[case_id]
        if not isinstance(first, list) or len(first) != len(ALL):
            raise ValueError(f"{source_id}: pass-1 scores must contain R1-R12")
        if not isinstance(second, list) or len(second) != len(ALL):
            raise ValueError(f"{case_id}: pass-2 scores must contain R1-R12")
        for criterion, a, b in zip(ALL, first, second):
            if any(isinstance(value, bool) or not isinstance(value, int) or value not in (0, 1, 2)
                   for value in (a, b)):
                raise ValueError(f"{case_id}/{criterion}: scores must be integer 0, 1, or 2")
            observed.append((a, b, case_id, criterion))
        by_proposal[case_id] = {"first": first, "second": second}
        by_pair.setdefault(pair_id, {})[variant] = case_id

    n = len(observed)
    exact = sum(a == b for a, b, _, _ in observed)
    within = sum(abs(a - b) <= 1 for a, b, _, _ in observed)
    hard_gate = sum((a == 0) == (b == 0) for a, b, _, criterion in observed if criterion in HARD)
    hard_n = sum(criterion in HARD for _, _, _, criterion in observed)
    first_gates, second_gates = [], []
    for case_id in by_proposal:
        first = by_proposal[case_id]["first"]
        second = by_proposal[case_id]["second"]
        first_gates.append(any(first[ALL.index(c)] == 0 for c in HARD))
        second_gates.append(any(second[ALL.index(c)] == 0 for c in HARD))

    per_criterion = {}
    for criterion in ALL:
        rows = [(a, b) for a, b, _, c in observed if c == criterion]
        per_criterion[criterion] = {
            "n": len(rows),
            "exact_agreement": round(sum(a == b for a, b in rows) / len(rows), 4) if rows else None,
            "within_one_agreement": round(sum(abs(a-b) <= 1 for a, b in rows) / len(rows), 4) if rows else None,
            "cohen_kappa_unweighted": _kappa([a for a, _ in rows], [b for _, b in rows]),
        }
    pairwise = []
    for pair_id, pair in by_pair.items():
        if set(pair) != {"good", "bad"}:
            raise ValueError(f"{pair_id}: blind manifest must map exactly one good and one bad item")
        good_id, bad_id = pair["good"], pair["bad"]
        first = by_proposal[good_id]["first"], by_proposal[bad_id]["first"]
        second = by_proposal[good_id]["second"], by_proposal[bad_id]["second"]
        ixs = [ALL.index(c) for c in JUDGED]
        first_delta = sum(first[0][i] - first[1][i] for i in ixs)
        second_delta = sum(second[0][i] - second[1][i] for i in ixs)
        first_order, second_order = (first_delta > 0) - (first_delta < 0), (second_delta > 0) - (second_delta < 0)
        pairwise.append({"pair_id": pair_id, "first_order": first_order,
                         "second_order": second_order, "agree": first_order == second_order})
    return {
        "status": "exercised", "rater_count": 2,
        "n_proposals": len(by_proposal), "n_compared": n,
        "exact_agreement": round(exact / n, 4) if n else None,
        "within_one_agreement": round(within / n, 4) if n else None,
        "hard_gate_agreement": round(hard_gate / hard_n, 4) if hard_n else None,
        "proposal_hard_gate_agreement": round(sum(a == b for a, b in zip(first_gates, second_gates)) / len(first_gates), 4) if first_gates else None,
        "cohen_kappa_unweighted": _kappa([a for a, _, _, _ in observed], [b for _, b, _, _ in observed]),
        "cohen_kappa_note": "Nominal pooled 0/1/2 labels; repeated criteria within proposals are dependent, so descriptive only.",
        "per_criterion": per_criterion,
        "pairwise_n": len(pairwise),
        "pairwise_winner_agreement": round(sum(p["agree"] for p in pairwise) / len(pairwise), 4) if pairwise else None,
        "pairwise": pairwise,
        "mismatches": [{"case_id": case_id, "criterion": criterion, "pass1": a, "pass2": b}
                       for a, b, case_id, criterion in observed if a != b],
    }


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--golden", help="golden proposal corpus for judge comparison")
    mode.add_argument("--pass1", help="first labeler scores JSON for labeler-agreement mode")
    parser.add_argument("--judge-output", help="row-level or legacy aggregate judge JSON")
    parser.add_argument("--manifest", help="opaque-ID mapping for labeler-agreement mode")
    parser.add_argument("--pass2", help="second blind labeler scores JSON")
    parser.add_argument("--out")
    args = parser.parse_args(argv)
    try:
        if args.pass1:
            if not args.manifest or not args.pass2 or args.judge_output:
                parser.error("--pass1 requires --manifest and --pass2, and cannot be combined with --judge-output")
            with open(args.pass1, encoding="utf-8") as f:
                pass1 = json.load(f)
            with open(args.manifest, encoding="utf-8") as f:
                manifest = json.load(f)
            with open(args.pass2, encoding="utf-8") as f:
                pass2 = json.load(f)
            report = compare_labelers(pass1, manifest, pass2)
        else:
            if not args.judge_output or args.manifest or args.pass2:
                parser.error("--golden requires --judge-output and cannot be combined with labeler inputs")
            with open(args.golden, encoding="utf-8") as f:
                golden = json.load(f)
            with open(args.judge_output, encoding="utf-8") as f:
                judge = json.load(f)
            report = compare(golden, judge)
    except (OSError, ValueError, KeyError, TypeError) as exc:
        print(f"r3-2 agreement: {type(exc).__name__}: {exc}", file=sys.stderr)
        return 2
    text = json.dumps(report, indent=2, sort_keys=True)
    if args.out:
        with open(args.out, "w", encoding="utf-8") as f:
            f.write(text + "\n")
    else:
        print(text)
    return 0


if __name__ == "__main__":
    sys.exit(main())
