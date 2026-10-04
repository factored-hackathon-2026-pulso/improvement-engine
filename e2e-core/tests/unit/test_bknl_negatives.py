"""BKNL RED/GREEN (offline): every live negative is labelled by the engine's compile step with its named denial reason.
A permissive compile stub (a denied draft slips through as `compiled`) must make `assert_engine_denies` fail."""
from pathlib import Path

import pytest

from claude_standin import bknl as B
from claude_standin import compile_step as C

ROOT = Path(__file__).resolve().parents[3]
WORLD = C.load_world(ROOT / "agent-core-assets" / "worlds" / "seeded-base.world.yaml")
ENGINE_NEGATIVES = [n for n in B.NEGATIVES if n.engine_op is not None]


def permissive(doc: dict, world: dict, dry_run=None) -> dict:
    ops = doc["change_spec"]["operations"]
    return {"status": "compiled", "step": "compile", "draft_plan": {"operations": ops, "digest": "sha256:" + "0" * 64}}


def test_the_four_engine_reasons_are_covered_once() -> None:
    assert sorted(n.expected_reason for n in ENGINE_NEGATIVES) == sorted(
        ["kind_not_supported", "missing_precondition", "outside_bridge", "mutable_reference"])
    assert [n.expected_reason for n in B.NEGATIVES if n.engine_op is None] == ["release_settings_not_allowed"]


@pytest.mark.parametrize("neg", ENGINE_NEGATIVES, ids=lambda n: n.id)
def test_real_compile_labels_each_negative(neg: B.Negative) -> None:
    assert B.assert_engine_denies(neg, WORLD, C.compile_change_spec) == neg.expected_reason


@pytest.mark.parametrize("neg", ENGINE_NEGATIVES, ids=lambda n: n.id)
def test_a_permissive_compile_stub_lets_the_denied_kind_slip_and_the_check_fails(neg: B.Negative) -> None:
    with pytest.raises(AssertionError, match="slipped through"):
        B.assert_engine_denies(neg, WORLD, permissive)
