"""BKNL offline replay (no Podman): the recorded real Core/bridge answers (tests/fixtures/bknl) plus the real compile step.
Each negative: the engine labels it with its named reason, Core's recorded answer is classified, nothing was published."""
import json
import re
from pathlib import Path

import pytest

from claude_standin import bknl as B
from claude_standin import compile_step as C
from codex_standin.dto import request_digest

ROOT = Path(__file__).resolve().parents[3]
FIX = Path(__file__).resolve().parents[1] / "fixtures" / "bknl"
WORLD = C.load_world(ROOT / "agent-core-assets" / "worlds" / "seeded-base.world.yaml")
BY_ID = {n.id: n for n in B.NEGATIVES}


def load(name: str) -> dict:
    return json.loads((FIX / f"{name}.json").read_text("ascii"))


def test_one_fixture_per_negative_with_provenance_and_no_secrets() -> None:
    assert sorted(p.stem for p in FIX.glob("*.json")) == sorted(BY_ID)
    for p in FIX.glob("*.json"):
        d = json.loads(p.read_text("ascii"))
        assert d["provenance"]["image"] == "localhost/pulso-core-runtime:c814c2b-920f5e3" and d["provenance"]["command"]
        assert re.fullmatch(r"\d{4}-\d{2}-\d{2}", d["provenance"]["recorded_utc"])
        assert not re.search(r"eyJ[A-Za-z0-9_-]{10,}|Bearer|PASSWORD|secret", p.read_text("ascii"), re.I)


@pytest.mark.parametrize("name", sorted(BY_ID))
def test_recorded_answer_is_consistent_with_the_negative(name: str) -> None:
    neg, d = BY_ID[name], load(name)
    assert d["expected_reason"] == neg.expected_reason
    assert d["registry_before"] == d["registry_after"], "nothing may be published"
    assert d["request"]["changes"] == neg.core_changes(WORLD), "fixture drifted from the negative's draft"
    body = d["response"]["body"]
    if body.get("request_digest"):
        assert body["request_digest"] == request_digest(d["request"])
        assert body["proposal_created"] is False
    core = B.classify_core(d["response"]["status"], body)
    if neg.core_refuses:
        assert core["outcome"] in ("refused", "denied"), core
    else:
        assert core["outcome"] == "accepted", core  # recorded finding: the engine is the only guard
    if neg.engine_op is not None:
        assert B.assert_engine_denies(neg, WORLD) == neg.expected_reason


def test_recorded_reasons() -> None:
    got = {n: B.classify_core(load(n)["response"]["status"], load(n)["response"]["body"]) for n in BY_ID}
    assert got["release_settings"] == {"outcome": "denied", "code": "pulso:release_settings_not_allowed", "http": 422}
    assert got["add_prompt"]["rules"] == got["outside_bridge"]["rules"] == ["REG-UNREFERENCED"]
    assert got["overwrite_published"]["rules"] == ["REG-VERSION-TAKEN"]
    assert got["stale_precondition"] == {"outcome": "accepted"}
