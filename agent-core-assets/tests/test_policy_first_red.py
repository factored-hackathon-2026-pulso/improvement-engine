"""FIRST RED (package L4): a precomputed finding or a manifest/expected-state drift must fail validation.

Offline tier: pure PyYAML, no agent-core needed.
"""

from __future__ import annotations

import json
from pathlib import Path

import yaml

import assetcheck


def codes(violations) -> set[str]:
    return {v.code for v in violations}


def test_clean_tree_passes(assets: Path) -> None:
    assert assetcheck.check_all(assets) == []


def _scout_flow(assets: Path) -> Path:
    return assets / "worlds/pulso-evolution/flows/pulso-scout@1.0.0.yaml"


def test_precomputed_finding_key_in_stage_fails(assets: Path) -> None:
    path = _scout_flow(assets)
    doc = yaml.safe_load(path.read_text("utf-8"))
    doc["nodes"][1]["config"]["finding"] = {"winning_case": "abstention-on-dispute"}
    path.write_text(yaml.safe_dump(doc, sort_keys=True), "utf-8")
    assert "precomputed_finding" in codes(assetcheck.check_stages(assets))


def test_precomputed_finding_literal_output_in_stage_fails(assets: Path) -> None:
    path = _scout_flow(assets)
    doc = yaml.safe_load(path.read_text("utf-8"))
    end = next(n for n in doc["nodes"] if n["type"] == "end")
    end["config"]["output_map"] = {"pulso_hypotheses": "literal:stock-out explains the drop"}
    path.write_text(yaml.safe_dump(doc, sort_keys=True), "utf-8")
    assert "precomputed_finding" in codes(assetcheck.check_stages(assets))


def test_precomputed_finding_in_prompt_fails(assets: Path) -> None:
    path = assets / "worlds/pulso-evolution/prompts/p/scout_task@1.0.0.yaml"
    doc = yaml.safe_load(path.read_text("utf-8"))
    doc["locales"]["en"] += " Known winning finding: the refund path is the cause."
    path.write_text(yaml.safe_dump(doc, sort_keys=True, allow_unicode=True), "utf-8")
    assert "precomputed_finding" in codes(assetcheck.check_stages(assets))


def test_world_file_drift_fails_manifest(assets: Path) -> None:
    tpl = assets / "worlds/attention-demo/templates/t/acuse@1.0.0.yaml"
    tpl.write_text(tpl.read_text("utf-8") + "\n# drift\n", "utf-8")
    assert "manifest_drift" in codes(assetcheck.check_manifest(assets))


def test_manifest_release_id_drift_fails(assets: Path) -> None:
    path = assets / "manifest.yaml"
    doc = yaml.safe_load(path.read_text("utf-8"))
    first = sorted(doc["release_ids"])[0]
    doc["release_ids"][first] = "rel-0000000000000000"
    path.write_text(yaml.safe_dump(doc, sort_keys=True), "utf-8")
    assert "expected_state_drift" in codes(assetcheck.check_manifest(assets))


def test_expected_state_entity_drift_fails(assets: Path) -> None:
    path = assets / "expected-state.json"
    doc = json.loads(path.read_text("utf-8"))
    agent = sorted(doc)[0]
    doc[agent]["entities"].pop()
    path.write_text(json.dumps(doc, indent=2, sort_keys=True), "utf-8")
    assert "expected_state_drift" in codes(assetcheck.check_manifest(assets))


def test_pin_sha_drift_fails(assets: Path) -> None:
    path = assets / "manifest.yaml"
    doc = yaml.safe_load(path.read_text("utf-8"))
    doc["pin"]["sha"] = "0" * 40
    path.write_text(yaml.safe_dump(doc, sort_keys=True), "utf-8")
    assert "pin_drift" in codes(assetcheck.check_manifest(assets))
