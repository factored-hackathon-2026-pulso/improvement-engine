"""The report never over-claims: target derivation, doubles union and the independent request-digest cross-check."""

from __future__ import annotations

from codex_standin import PIN_SHA, report
from codex_standin.dto import digest_json
from codex_standin.engine import put_draft_digest

GOOD = {"sim_info_status": 404, "sha": PIN_SHA, "reported_digest": "sha256:a", "expected_digest": "sha256:a",
        "ready": True}


def test_real_local_only_when_every_condition_holds() -> None:
    assert report.target_of(**GOOD) == "real_local"
    for over in ({"sim_info_status": 200}, {"sha": "0" * 40}, {"reported_digest": "sha256:b"},
                 {"reported_digest": None, "expected_digest": None}, {"ready": False}):
        assert report.target_of(**{**GOOD, **over}) == "evidence_target_unproven", over


def test_doubles_are_a_deduplicated_union_of_runtime_containers_and_declared() -> None:
    out = report.union_doubles(["transcript: null", "transcript: null"],
                               [{"piece": "container:platform-sim", "kind": "double_container", "declared_by": "label"}],
                               [{"piece": "llm:scripted", "kind": "fixture", "declared_by": "e2e"},
                                {"piece": "llm:scripted", "kind": "fixture", "declared_by": "e2e"}])
    assert [d["piece"] for d in out] == ["runtime:transcript: null", "container:platform-sim", "llm:scripted"]
    assert all(set(d) == {"piece", "kind", "declared_by"} for d in out)


def test_put_draft_digest_matches_core_bridge_when_importable() -> None:
    changes = [{"kind": "prompt", "content": {"id": "p/x"}, "docs": {}}]
    assert put_draft_digest(None, None, changes) == digest_json({"proposal_id": None, "expected_rev": None,
                                                                 "changes": changes})
    try:
        from pulso_core_runtime.tools.builder import put_draft_digest as theirs
    except ImportError:  # core-bridge/src not on PYTHONPATH (run.ps1 adds it)
        return
    assert theirs(None, None, changes) == put_draft_digest(None, None, changes)
