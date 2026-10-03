"""gen_wire pin resolution: pin.json when present (fail closed on a malformed one), else the built-in fallback."""

from __future__ import annotations

import hashlib
import importlib.util
import json
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]
FALLBACK_SHA = "789d6c89b2fca90fc10e2abf157da51dc81c5d51"
pytestmark = pytest.mark.wire


@pytest.fixture(scope="module")
def gw():
    spec = importlib.util.spec_from_file_location("gen_wire", ROOT / "scripts" / "gen_wire.py")
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def _write_pin(repo: Path, payload) -> None:
    d = repo / "contracts" / "agent_core"
    d.mkdir(parents=True)
    (d / "pin.json").write_text(payload if isinstance(payload, str) else json.dumps(payload), encoding="utf-8")


def test_fallback_when_no_pin_json(gw, tmp_path) -> None:
    pin = gw.resolve_pin(tmp_path)
    assert pin["sha"] == FALLBACK_SHA and pin["contract_version"] == "1.3.0" and pin["source"] == "fallback"


def test_pin_json_wins(gw, tmp_path) -> None:
    sha = "a" * 40
    _write_pin(tmp_path, {"repo": "agent-core", "sha": sha, "contract_version": "1.3.0", "manifest_sha256": "b" * 64})
    pin = gw.resolve_pin(tmp_path)
    assert pin["sha"] == sha and pin["source"] == "pin.json" and pin["manifest_sha256"] == "b" * 64


@pytest.mark.parametrize("payload", ["{not json", '{"sha": "short", "contract_version": "1.3.0"}', '{"contract_version": "1.3.0"}',
                                     '{"sha": "%s", "contract_version": "9.9.9"}' % ("a" * 40), "[]"])
def test_malformed_pin_json_fails_closed_never_falls_back(gw, tmp_path, payload) -> None:
    _write_pin(tmp_path, payload)
    with pytest.raises(SystemExit):
        gw.resolve_pin(tmp_path)


def test_manifest_digest_is_enforced_when_pin_declares_it(gw) -> None:
    data = b'{"files": []}\n'
    gw.check_manifest_digest({"manifest_sha256": hashlib.sha256(data).hexdigest()}, data)
    gw.check_manifest_digest({}, data)  # nothing declared: nothing to compare
    with pytest.raises(SystemExit):
        gw.check_manifest_digest({"manifest_sha256": "0" * 64}, data)


def test_committed_manifest_matches_pin_if_present_in_repo() -> None:
    pin_file = ROOT.parent / "contracts" / "agent_core" / "pin.json"
    if not pin_file.exists():
        pytest.skip("no contracts/agent_core/pin.json in this checkout")
    pin = json.loads(pin_file.read_text(encoding="utf-8"))
    manifest = ROOT / "wire" / f"agent_core@{pin['sha'][:7]}" / "MANIFEST.json"
    assert manifest.exists(), "pin.json points to a sha with no committed wire snapshot"
    if pin.get("manifest_sha256"):
        assert hashlib.sha256(manifest.read_bytes()).hexdigest() == pin["manifest_sha256"]
