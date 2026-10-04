"""SMAP for E0 categories: a hashed E0 group never maps to the catalogue; `unlinked` is the honest ending."""
import sys
from pathlib import Path

import pytest

from claude_standin import compile_step as C
from claude_standin import ed0_lab as L
from claude_standin import smap

ROOT = Path(__file__).resolve().parents[3]
WORLD = C.load_world(ROOT / "agent-core-assets" / "worlds" / "seeded-base.world.yaml")
CAT = smap.catalogue_from_world(WORLD)
SALT = b"synthetic-smap-salt-01"


def _lab(tmp_path, a_hits, b_hits):
    cases = [(f"c{i}", "SIG-A", "w1", i < a_hits) for i in range(30)] + [(f"d{i}", "SIG-B", "w1", i < b_hits) for i in range(20)]
    return L.build_lab(tmp_path / "lab.sqlite", cases, SALT)


def test_first_red_lab_groups_exposes_only_hashes_and_aggregates(tmp_path):
    db = _lab(tmp_path, 12, 5)
    groups = L.lab_groups(db)
    assert set(groups) == {L.group_hash(SALT, "group", "SIG-A"), L.group_hash(SALT, "group", "SIG-B")}
    assert sorted(v["numerator"] for v in groups.values()) == [5, 12]
    assert all(k.startswith("g_") or len(k) == 16 for k in groups)


def test_no_exact_mapping_for_an_e0_group_is_unlinked(tmp_path):
    db = _lab(tmp_path, 12, 5)
    for g, v in L.lab_groups(db).items():
        finding = {"finding_ref": "finding_1", "category": g, "evidence_refs": [v["evidence_ref"]]}
        assert smap.select_target(finding, CAT) is None
        assert smap.design_input(finding, CAT)["candidates"] == []
        m = smap.e0_mapping(finding, CAT)
        assert m == {"verdict": "unlinked", "reason": "no_exact_supported_flow_mapping", "target": None}


def test_mapping_never_follows_the_winning_category(tmp_path):
    """Swap which group wins: the mapping is unchanged because it never keys on support."""
    for a, b in ((12, 5), (5, 12)):
        db = _lab(tmp_path, a, b)
        cats = {g: v["numerator"] for g, v in L.lab_groups(db).items()}
        win = smap.winning_category(cats)
        finding = {"finding_ref": "f", "category": win, "evidence_refs": ["ev_x"]}
        assert smap.e0_mapping(finding, CAT)["verdict"] == "unlinked"


def test_an_exact_catalogue_category_still_links():
    finding = {"finding_ref": "f", "category": "closing_reply_unclear", "evidence_refs": ["ev_x"]}
    assert smap.e0_mapping(finding, CAT)["verdict"] == "linked"


def test_builder_claiming_a_target_for_an_unmapped_group_is_still_unlinked(tmp_path):
    db = _lab(tmp_path, 12, 5)
    g, v = next(iter(L.lab_groups(db).items()))
    finding = {"finding_ref": "f", "category": g, "evidence_refs": [v["evidence_ref"]]}
    out = {"design_intent": {"verdict": "linked", "target_ref": CAT["entries"][0]["target_ref"]},
           "evidence_refs": [v["evidence_ref"]]}
    assert smap.classify(out, finding, CAT, WORLD)["verdict"] == "unlinked"
