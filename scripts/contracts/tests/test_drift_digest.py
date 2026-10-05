"""Digest-keyed drift gate for the agent-core `contracts/` tree and the support-platform schema sources.

Why: agent-core extended the `ProblemCode` enum (`idempotency_in_progress`) without bumping `contracts/VERSION`
(1.3.0), so a version-keyed gate stays green on a silent change. These tests pin that a content digest catches it.
"""
from __future__ import annotations

import json
import os
import subprocess
import sys
from pathlib import Path

import pytest

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import drift_digest as dd  # noqa: E402

REPO = HERE.parents[2]
MANIFEST = REPO / "scripts" / "contracts" / "pinned_digests.json"
OLD_PIN = "c814c2bad9f154d10c092326558815dca9562be7"


def make_contracts(root: Path, problem_codes=("a", "b"), version="1.3.0") -> Path:
    c = root / "contracts"
    (c / "schemas").mkdir(parents=True)
    (c / "events").mkdir()
    (c / "registry").mkdir()
    (c / "VERSION").write_text(version + "\n", "utf-8")
    (c / "openapi.json").write_text('{"openapi": "3.1.0", "info": {"version": "%s"}}\n' % version, "utf-8")
    (c / "schemas" / "ProblemCode.json").write_text(json.dumps({"enum": list(problem_codes)}) + "\n", "utf-8")
    (c / "events" / "catalog.json").write_text('{"types": ["run.started"]}\n', "utf-8")
    (c / "registry" / "Proposal.json").write_text('{"type": "object"}\n', "utf-8")
    return c


def test_digest_is_deterministic_and_order_independent():
    a = dd.digest_files({"x": b"1", "y": b"2"})
    b = dd.digest_files({"y": b"2", "x": b"1"})
    assert a == b and len(a) == 64


def test_digest_ignores_line_endings():
    assert dd.digest_files({"x": b"a\r\nb\r\n"}) == dd.digest_files({"x": b"a\nb\n"})


def test_digest_depends_on_path_and_content():
    base = dd.digest_files({"x": b"1"})
    assert dd.digest_files({"y": b"1"}) != base
    assert dd.digest_files({"x": b"2"}) != base


def test_silent_enum_extension_changes_digest_although_version_is_unchanged(tmp_path):
    before = make_contracts(tmp_path / "old")
    after = make_contracts(tmp_path / "new", problem_codes=("a", "b", "idempotency_in_progress"))
    d0, v0 = dd.agent_core_digest(before)
    d1, v1 = dd.agent_core_digest(after)
    assert v0 == v1 == "1.3.0"
    assert d0 != d1


def test_version_file_is_reported_but_not_part_of_the_content_digest(tmp_path):
    a = make_contracts(tmp_path / "a", version="1.3.0")
    # same content tree except VERSION: digest equal (VERSION is recorded separately, never trusted alone)
    (a / "VERSION").write_text("9.9.9\n", "utf-8")
    d, v = dd.agent_core_digest(a)
    assert v == "9.9.9"
    assert d == dd.agent_core_digest(make_contracts(tmp_path / "b", version="1.3.0"))[0]


def test_check_passes_when_digest_matches_and_fails_naming_the_changed_file(tmp_path):
    base = make_contracts(tmp_path / "pin")
    digest, version = dd.agent_core_digest(base)
    manifest = {"agent_core": {"pinned": {"sha": "s", "contracts_version": version, "digest": digest,
                                          "files": dd.file_digests(dd.collect_agent_core(base))}}}
    assert dd.check_agent_core(manifest, base) == []
    drifted = make_contracts(tmp_path / "head", problem_codes=("a", "b", "c"))
    problems = dd.check_agent_core(manifest, drifted)
    assert problems and any("schemas/ProblemCode.json" in p for p in problems)
    assert any("digest" in p for p in problems)


def test_check_reports_version_unchanged_when_content_drifted(tmp_path):
    base = make_contracts(tmp_path / "pin")
    digest, version = dd.agent_core_digest(base)
    manifest = {"agent_core": {"pinned": {"sha": "s", "contracts_version": version, "digest": digest,
                                          "files": dd.file_digests(dd.collect_agent_core(base))}}}
    drifted = make_contracts(tmp_path / "head", problem_codes=("a", "b", "c"))
    assert any("without a VERSION change" in p for p in dd.check_agent_core(manifest, drifted))


def test_platform_digest_changes_when_an_enum_source_changes(tmp_path):
    root = tmp_path / "p"
    for rel in dd.PLATFORM_FILES:
        f = root / rel
        f.parent.mkdir(parents=True, exist_ok=True)
        f.write_text("class X: pass\n", "utf-8")
    d0 = dd.platform_digest(root)
    (root / dd.PLATFORM_FILES[1]).write_text("class X: WITH_ASSISTANT = 'with_assistant'\n", "utf-8")
    assert dd.platform_digest(root) != d0


def test_missing_platform_source_is_an_error_not_a_pass(tmp_path):
    with pytest.raises(dd.MissingInput):
        dd.platform_digest(tmp_path)


def test_cli_exit_codes(tmp_path):
    base = make_contracts(tmp_path / "pin")
    digest, version = dd.agent_core_digest(base)
    m = tmp_path / "m.json"
    m.write_text(json.dumps({"agent_core": {"pinned": {"sha": "s", "contracts_version": version, "digest": digest,
                                                       "files": dd.file_digests(dd.collect_agent_core(base))}}}),
                 "utf-8")
    script = str(HERE.parent / "drift_digest.py")
    ok = subprocess.run([sys.executable, script, "--manifest", str(m), "--agent-core", str(base)],
                        capture_output=True, text=True)
    assert ok.returncode == 0, ok.stdout + ok.stderr
    drifted = make_contracts(tmp_path / "head", problem_codes=("a", "z"))
    bad = subprocess.run([sys.executable, script, "--manifest", str(m), "--agent-core", str(drifted)],
                         capture_output=True, text=True)
    assert bad.returncode == 1 and "drift" in (bad.stdout + bad.stderr).lower()
    missing = subprocess.run([sys.executable, script, "--manifest", str(m), "--agent-core", str(tmp_path / "nope")],
                             capture_output=True, text=True)
    assert missing.returncode == 2


# --- the committed manifest -------------------------------------------------------------------------------------

def manifest() -> dict:
    return json.loads(MANIFEST.read_text("utf-8"))


def test_manifest_records_pin_digest_and_head_observation():
    m = manifest()
    pin = m["agent_core"]["pinned"]
    assert pin["sha"] == OLD_PIN or len(pin["sha"]) == 40
    assert len(pin["digest"]) == 64 and pin["contracts_version"] == "1.3.0"
    head = m["agent_core"]["observed_head"]
    assert head["sha"].startswith("56354dd") and head["digest"] != pin["digest"]
    assert head["contracts_version"] == pin["contracts_version"], "the silent extension: same VERSION, other digest"
    assert set(head["changed_files"]) == {"openapi.json", "schemas/ProblemCode.json"}
    plat = m["platform"]["pinned"]
    assert plat["sha"].startswith("5261ecf") and len(plat["digest"]) == 64  # re-pinned from eeb73a8 by SIG1 (contract 1.3.0)
    assert m["platform"]["unreachable"]["a492bfa"]
    assert "inference" in m["platform"]["unreachable"]["closest_old_match"].lower()


def test_pinned_agent_core_digest_matches_the_committed_wire_snapshot():
    """The wire snapshot is a byte copy of the pinned `contracts/`: its digest is the pinned digest, offline."""
    pin = manifest()["agent_core"]["pinned"]
    wire = REPO / "core-bridge" / "wire" / f"agent_core@{pin['sha'][:7]}"
    if not wire.is_dir():
        pytest.fail(f"wire snapshot for the pinned sha is missing: {wire}")
    assert dd.digest_files(dd.collect_agent_core(wire)) == pin["digest"]


@pytest.mark.skipif(not os.environ.get("AGENT_CORE_CHECKOUT"), reason="needs an agent-core checkout")
def test_live_checkout_at_head_is_reported_as_drift_by_digest_not_by_version():
    root = Path(os.environ["AGENT_CORE_CHECKOUT"])
    d, v = dd.agent_core_digest(root / "contracts")
    m = manifest()["agent_core"]
    assert v == m["pinned"]["contracts_version"]
    head = subprocess.run(["git", "-C", str(root), "rev-parse", "HEAD"], capture_output=True, text=True).stdout.strip()
    if head == m["observed_head"]["sha"]:
        assert d == m["observed_head"]["digest"]
    elif head == m["pinned"]["sha"]:
        assert d == m["pinned"]["digest"]


@pytest.mark.skipif(not os.environ.get("PLATFORM_CHECKOUT"), reason="needs a support-platform checkout")
def test_live_platform_checkout_digest_matches_pin():
    root = Path(os.environ["PLATFORM_CHECKOUT"])
    head = subprocess.run(["git", "-C", str(root), "rev-parse", "HEAD"], capture_output=True, text=True).stdout.strip()
    pin = manifest()["platform"]["pinned"]
    if head != pin["sha"]:
        pytest.skip("checkout is not at the pinned platform sha")
    assert dd.platform_digest(root) == pin["digest"]
