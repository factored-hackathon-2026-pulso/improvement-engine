"""agent-core tier: runs the pinned checkout's own validator/registry code (slow; skipped without a checkout)."""

from __future__ import annotations

import json
import shutil
from pathlib import Path

import pytest
import yaml

import assetcheck
from conftest import PIN_SHA

pytestmark = pytest.mark.agentcore


@pytest.fixture(scope="module")
def merged(tmp_path_factory, checkout):
    root = Path(__file__).resolve().parents[1]
    dest = tmp_path_factory.mktemp("seed") / "seed-root"
    assert assetcheck.merge_worlds(root, dest) == []
    return dest


def test_checkout_is_the_pin(checkout: Path) -> None:
    assert assetcheck.checkout_head(checkout) == PIN_SHA
    assert (checkout / "contracts" / "VERSION").read_text().strip() == "1.3.0"


def test_each_world_and_merged_validate(checkout: Path, merged: Path) -> None:
    root = Path(__file__).resolve().parents[1]
    for target in [*assetcheck.worlds_of(root), merged]:
        code, doc = assetcheck.agentcore_validate(checkout, target)
        assert (code, doc["violations"]) == (0, []), target


def test_attention_demo_is_the_pinned_fixture(checkout: Path) -> None:
    root = Path(__file__).resolve().parents[1]
    world, fixture = root / "worlds/attention-demo", checkout / "tests/fixtures/registry-demo"
    for src in fixture.rglob("*"):
        if src.is_file():
            assert (world / src.relative_to(fixture)).read_bytes().replace(b"\r\n", b"\n") == \
                src.read_bytes().replace(b"\r\n", b"\n"), src


def test_state_matches_expected_state_and_is_deterministic(checkout: Path, merged: Path) -> None:
    root = Path(__file__).resolve().parents[1]
    expected = json.loads((root / "expected-state.json").read_text("utf-8"))
    first = assetcheck.compute_state(checkout, merged)
    assert first == expected
    assert assetcheck.compute_state(checkout, merged) == first
    assert expected["atencion"]["release_id"] == "rel-e26df0070f6be82f"  # verified against the real Core at 894fa65 (locked interrupt)


def test_in_memory_import_semantics(checkout: Path, merged: Path) -> None:
    root = Path(__file__).resolve().parents[1]
    expected = json.loads((root / "expected-state.json").read_text("utf-8"))
    res = assetcheck.import_in_memory(checkout, merged)
    assert res["release_ids"] == {a: s["release_id"] for a, s in expected.items()}
    assert res["reimport_code"] == "illegal_transition"  # -> already_seeded in pulso-bootstrap
    assert res["bot_code"] == "forbidden_role"
    assert res["session_admin_code"] == "step_up_required"
    assert res["prod"] == res["staging"] == sorted(expected)


def test_registry_tooldefs_are_generated_not_hand_edited(checkout: Path, tmp_path: Path) -> None:
    root = Path(__file__).resolve().parents[1]
    written = assetcheck.render_registry_tooldefs(checkout, tmp_path)
    assert written, "generator produced nothing"
    for rel in written:
        committed = root / "worlds/pulso-evolution" / rel
        assert committed.is_file(), rel
        assert yaml.safe_load(committed.read_text("utf-8")) == yaml.safe_load((tmp_path / rel).read_text("utf-8")), rel
    on_disk = {p.relative_to(root / "worlds/pulso-evolution").as_posix()
               for p in (root / "worlds/pulso-evolution/tools/registry").glob("*.yaml")}
    assert on_disk == set(written)


def test_state_drift_is_detected_against_the_real_registry(checkout: Path, merged: Path, tmp_path: Path) -> None:
    root = Path(__file__).resolve().parents[1]
    mutated = tmp_path / "seed-root"
    shutil.copytree(merged, mutated)
    tpl = mutated / "templates/t/acuse@1.0.0.yaml"
    tpl.write_text(tpl.read_text("utf-8").replace("Lo anoto", "Lo apunto"), "utf-8")
    expected = json.loads((root / "expected-state.json").read_text("utf-8"))
    assert assetcheck.compute_state(checkout, mutated) != expected


def test_invalid_world_is_rejected_by_agentcore(checkout: Path, merged: Path, tmp_path: Path) -> None:
    broken = tmp_path / "broken"
    shutil.copytree(merged, broken)
    flow = broken / "flows/pulso-scout@1.0.0.yaml"
    doc = yaml.safe_load(flow.read_text("utf-8"))
    doc["nodes"][1]["config"]["tool"] = "pulso/does_not_exist@1"
    flow.write_text(yaml.safe_dump(doc, sort_keys=True), "utf-8")
    code, out = assetcheck.agentcore_validate(checkout, broken)
    assert code != 0 and out["violations"]
