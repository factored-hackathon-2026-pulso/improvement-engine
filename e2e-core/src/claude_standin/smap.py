"""SMAP: S-MAP-lite harness, finding -> builder -> dry-run, over recorded SYNTHETIC builder outputs.

The target of a change is chosen from the ReadBase capability catalogue by the finding's category; the builder
target_ref is a claim that is checked against that choice and never used to pick it. Endings: valid, unlinked,
not_evaluable (all legitimate) and invalid (reported). No quality claim; data_origin is generated_sample.
"""
from . import compile_step as C

# Declared category vocabulary of the catalogue (synthetic labels; categories are data, not code paths).
_ADDRESSES = {"prompt": ["closing_reply_unclear", "followup_wording"], "eval_suite": ["coverage_gap_dispute"]}
_OPS = {"prompt": "replace", "eval_suite": "add"}
UNLINKED_REASON = "no_exact_supported_flow_mapping"
COUNT_KEYS = ("valid", "unlinked", "not_evaluable", "invalid")


def catalogue_from_world(world: dict) -> dict:
    """ReadBase capability catalogue derived from the seeded world slots (never hand-written)."""
    entries = []
    for kind, slot in C._slots(world).items():
        major = C._major(slot["version"])
        entries.append({"target_ref": f"{kind}:{slot['name']}@{major}", "target_kind": kind, "op": _OPS[kind],
                        "new_ref": f"{kind}:{slot['name']}@{major + 1}", "addresses": sorted(_ADDRESSES[kind])})
    return {"contract_version": "smap/0", "data_class": "synthetic", "base_world": world["base_world"],
            "entries": sorted(entries, key=lambda e: e["target_ref"])}


def winning_category(categories: dict) -> str:
    """Category with the most support; ties by label order. Follows the evidence, not the label."""
    return sorted(categories, key=lambda label: (-categories[label], label))[0]


def _candidates(finding: dict, catalogue: dict) -> list:
    return [e for e in catalogue["entries"] if finding.get("category") in e["addresses"]]


def select_target(finding: dict, catalogue: dict):
    c = _candidates(finding, catalogue)
    return c[0]["target_ref"] if len(c) == 1 else None


def e0_mapping(finding: dict, catalogue: dict) -> dict:
    """Honest mapping of an E0 finding. A target exists only on an EXACT match of the finding's category with a
    catalogue entry's declared vocabulary. E0 categories are salted hashed groups (query signature) that no entry
    declares, so the ending is `unlinked`; support, rank and the winning category are never consulted."""
    target = select_target(finding, catalogue)
    if target is None:
        return {"verdict": "unlinked", "reason": UNLINKED_REASON, "target": None}
    return {"verdict": "linked", "reason": None, "target": target}


def design_input(finding: dict, catalogue: dict) -> dict:
    """Design-input producer for the builder: finding facts plus the catalogue-derived candidates only."""
    return {"finding_ref": finding["finding_ref"], "category": finding["category"],
            "evidence_refs": sorted(finding["evidence_refs"]),
            "candidates": [{"target_ref": e["target_ref"], "op": e["op"]} for e in _candidates(finding, catalogue)]}


def _dry_run(entry: dict, world: dict) -> str:
    op = {"op": entry["op"], "target_kind": entry["target_kind"], "target_ref": entry["target_ref"],
          "new_ref": entry["new_ref"], "precondition_digest": C.asset_digest(world, entry["target_ref"])}
    spec = {"contract_version": "engine-steps/0", "step": "compile", "run_id": "run-smap-0001", "data_class": "synthetic",
            "base_bundle_ref": C.bundle_ref(world),
            "change_spec": {"base_bundle_ref": C.bundle_ref(world), "opportunity_ref": "opportunity:smap@1",
                            "workflow_bridge_ref": C.bridge_ref(world), "operations": [op],
                            "expected_mechanism": "recorded synthetic",
                            "affected_routes": [world["replaceable_prompt"]["used_by"]["flow"]],
                            "rollback_ref": C.bundle_ref(world)}}
    return C.compile_change_spec(spec, world)["status"]


def classify(output: dict, finding: dict, catalogue: dict, world: dict) -> dict:
    res = {"verdict": None, "target": None, "dry_run": None, "reason": None}
    di = output.get("design_intent") if isinstance(output, dict) else None
    if not isinstance(di, dict) or di.get("verdict") not in ("linked", "unlinked", "do_nothing", "not_evaluable"):
        return {**res, "verdict": "invalid", "reason": "malformed_output"}
    verdict, claimed = di["verdict"], di.get("target_ref")
    if not set(output.get("evidence_refs", [])) <= set(finding["evidence_refs"]):
        return {**res, "verdict": "invalid", "reason": "invented_evidence"}
    if verdict == "not_evaluable":
        return {**res, "verdict": "not_evaluable", "reason": "builder_not_evaluable"}
    if verdict == "do_nothing":
        return {**res, "verdict": "valid", "reason": "do_nothing"}
    chosen = select_target(finding, catalogue)
    if verdict == "unlinked" or chosen is None:
        return {**res, "verdict": "unlinked", "reason": "no_catalogue_target" if chosen is None else "builder_unlinked"}
    if claimed != chosen:
        return {**res, "verdict": "invalid", "reason": "target_not_from_catalogue_choice"}
    entry = next(e for e in catalogue["entries"] if e["target_ref"] == chosen)
    dr = _dry_run(entry, world)
    if dr != "compiled":
        return {**res, "verdict": "invalid", "reason": "dry_run_denied", "dry_run": dr}
    return {**res, "verdict": "valid", "target": chosen, "dry_run": dr, "reason": "catalogue_target"}


def run_harness(recorded: dict, catalogue: dict, world: dict) -> dict:
    if catalogue != catalogue_from_world(world):
        raise ValueError("catalogue_world_drift: catalogue does not match the seeded world slots")
    results, counts = [], dict.fromkeys(COUNT_KEYS, 0)
    for rec in recorded["outputs"]:
        r = classify(rec["builder_output"], rec["finding"], catalogue, world)
        counts[r["verdict"]] += 1
        results.append({"output_id": rec["output_id"], **r})
    return {"contract_version": "smap/0", "label": recorded["label"], "data_origin": "generated_sample",
            "quality_claims": "forbidden", "counts": counts, "results": results}
