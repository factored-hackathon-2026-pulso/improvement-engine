"""SMAP RED/GREEN: S-MAP-lite harness (finding -> builder -> dry-run) over recorded SYNTHETIC builder outputs.

The builder target is chosen from the ReadBase capability catalogue by the finding's category, never from model prose.
Recorded outputs are synthetic placeholders: label recorded/synthetic, no quality claim.
"""
import copy
import importlib.util
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "e2e-core" / "src"))
from claude_standin import compile_step as C  # noqa: E402
from claude_standin import smap as S  # noqa: E402

CORPUS = ROOT / "agent-core-assets" / "corpus"
CAT = json.loads((CORPUS / "smap-catalogue.json").read_text("utf-8")) if (CORPUS / "smap-catalogue.json").exists() else None
REC = json.loads((CORPUS / "smap-recorded-v0.json").read_text("utf-8")) if (CORPUS / "smap-recorded-v0.json").exists() else None
WORLD = C.load_world(ROOT / "agent-core-assets" / "worlds" / "seeded-base.world.yaml")
_spec = importlib.util.spec_from_file_location("engine_run", ROOT / "contracts" / "engine-run" / "engine_run.py")
ER = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(ER)


def finding(cat, ev=("ev-0001",)):
    return {"finding_ref": "sig-0001", "category": cat, "evidence_refs": list(ev)}


def test_relabelling_the_category_changes_or_nulls_the_target():
    a = S.select_target(finding("closing_reply_unclear"), CAT)
    assert a == "prompt:resumen_radicado@1"
    assert S.select_target(finding("coverage_gap_dispute"), CAT) == "eval_suite:disputas-suite@1"
    assert S.select_target(finding("otp_retry_exhaustion"), CAT) is None  # not in the catalogue -> null


def test_permuting_categories_permutes_targets():
    f = [finding("closing_reply_unclear"), finding("coverage_gap_dispute")]
    t = [S.select_target(x, CAT) for x in f]
    g = [dict(f[0], category=f[1]["category"]), dict(f[1], category=f[0]["category"])]
    assert [S.select_target(x, CAT) for x in g] == t[::-1] and t[0] != t[1]


def test_winning_category_follows_evidence_h3():
    assert ER.check_mapping_mutation(S.winning_category, {"x": 3, "y": 9, "z": 1}) == []
    keyed = lambda c: "closing_reply_unclear" if "closing_reply_unclear" in c else sorted(c)[0]  # noqa: E731
    assert ER.check_mapping_mutation(keyed, {"closing_reply_unclear": 1, "b": 9}) != []


def test_catalogue_is_derived_from_the_seeded_world():
    assert S.catalogue_from_world(WORLD) == CAT
    assert CAT["data_class"] == "synthetic"


def test_design_input_producer_lists_only_catalogue_candidates():
    d = S.design_input(finding("closing_reply_unclear"), CAT)
    assert [c["target_ref"] for c in d["candidates"]] == ["prompt:resumen_radicado@1"]
    assert S.design_input(finding("otp_retry_exhaustion"), CAT)["candidates"] == []
    assert "prose" not in json.dumps(d)


def test_model_prose_cannot_pick_the_target():
    f = finding("otp_retry_exhaustion")
    out = {"design_intent": {"verdict": "linked", "target_ref": "prompt:resumen_radicado@1", "mechanism": "x"},
           "evidence_refs": ["ev-0001"]}
    r = S.classify(out, f, CAT, WORLD)
    assert r["verdict"] == "unlinked" and r["target"] is None
    out["design_intent"]["target_ref"] = "eval_suite:disputas-suite@1"
    r = S.classify(out, finding("closing_reply_unclear"), CAT, WORLD)
    assert r["verdict"] == "invalid" and r["target"] is None


def test_valid_output_passes_dry_run_compile():
    out = {"design_intent": {"verdict": "linked", "target_ref": "eval_suite:disputas-suite@1", "mechanism": "x"},
           "evidence_refs": ["ev-0001"]}
    r = S.classify(out, finding("coverage_gap_dispute"), CAT, WORLD)
    assert r["verdict"] == "valid" and r["dry_run"] == "compiled" and r["target"] == "eval_suite:disputas-suite@1"


def test_recorded_corpus_counts():
    assert len(REC["outputs"]) == 10 and REC["label"] == "recorded/synthetic" and "quality" not in json.dumps(REC).lower().replace("no quality claim", "")
    rep = S.run_harness(REC, CAT, WORLD)
    assert rep["counts"] == {"valid": 4, "unlinked": 2, "not_evaluable": 2, "invalid": 2}
    byid = {r["output_id"]: r for r in rep["results"]}
    assert byid["so-06-linked-claim-no-catalogue-route"]["verdict"] == "unlinked"
    assert rep["quality_claims"] == "forbidden" and rep["data_origin"] == "generated_sample"


def test_harness_is_deterministic_and_does_not_mutate_inputs():
    a, b = copy.deepcopy(REC), copy.deepcopy(CAT)
    assert S.run_harness(REC, CAT, WORLD) == S.run_harness(REC, CAT, WORLD)
    assert a == REC and b == CAT


def test_catalogue_drift_against_world_is_detected():
    import pytest
    drift = copy.deepcopy(CAT)
    drift["entries"][0]["target_ref"] = "prompt:resumen_radicado@9"
    with pytest.raises(ValueError, match="catalogue_world_drift"):
        S.run_harness(REC, drift, WORLD)


def test_malformed_builder_output_is_invalid_not_a_crash():
    f = finding("closing_reply_unclear")
    for bad in ({}, {"design_intent": {}}, {"design_intent": {"verdict": "linked"}, "evidence_refs": ["ev-0001"]}):
        r = S.classify(bad, f, CAT, WORLD)
        assert r["verdict"] == "invalid" and r["target"] is None
