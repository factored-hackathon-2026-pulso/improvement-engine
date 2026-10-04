"""DC0 FIRST RED: the scanner must fail a tracked file carrying an E0 marker; the push scan blocks it.

Markers are assembled at runtime so this file never trips the scanner itself.
"""

from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import dataclass_gate as dc  # noqa: E402

E0_SENTINEL = "E0" + "_ROW"  # assembled: not a literal marker in this source
DATA_E0 = "data" + "=E0"


def test_scanner_fails_tracked_file_with_e0_marker(tmp_path: Path) -> None:
    f = tmp_path / "notes.txt"
    f.write_text(f"hello\n{E0_SENTINEL}: 12,disputa,3\n", "utf-8")
    findings = dc.scan_file(f, root=tmp_path)
    assert [x.rule for x in findings] == ["e0_marker"]
    assert findings[0].line == 2


def test_scanner_passes_clean_file_and_skips_binary(tmp_path: Path) -> None:
    (tmp_path / "a.txt").write_text("treated aggregate, k=25\n", "utf-8")
    (tmp_path / "b.bin").write_bytes(b"\x00\x01" + E0_SENTINEL.encode())
    assert dc.scan_file(tmp_path / "a.txt", root=tmp_path) == []
    assert dc.scan_file(tmp_path / "b.bin", root=tmp_path) == []


def test_marker_variants_detected(tmp_path: Path) -> None:
    for text in (f"{E0_SENTINEL}", "E0" + "-RAW row", "E0" + "_PAYLOAD={}"):
        assert dc.scan_text(text), text


def test_receipt_body_with_data_e0_fails() -> None:
    assert [x.rule for x in dc.scan_receipt_body(f"provider=hosted {DATA_E0}\n")] == ["receipt_data_e0"]
    body = json.dumps({"data": "E0", "provider": "agent_roleplay"})
    assert [x.rule for x in dc.scan_receipt_body(body)] == ["receipt_data_e0"]
    nested = json.dumps({"receipt": {"data_class": "E0"}})
    assert [x.rule for x in dc.scan_receipt_body(nested)] == ["receipt_data_e0"]


def test_receipt_body_with_treated_passes() -> None:
    assert dc.scan_receipt_body(json.dumps({"data": "treated"})) == []


def _git(root: Path, *args: str) -> None:
    subprocess.run(["git", "-C", str(root), *args], check=True, capture_output=True)


def _repo(tmp_path: Path) -> Path:
    _git(tmp_path, "init", "-q")
    return tmp_path


def test_push_scan_blocks_tracked_file_with_e0_marker(tmp_path: Path) -> None:
    root = _repo(tmp_path)
    (root / "ok.md").write_text("fine\n", "utf-8")
    (root / "leak.csv").write_text(f"{E0_SENTINEL},1,2\n", "utf-8")
    _git(root, "add", ".")
    result = dc.push_scan(root)
    assert [(f.path, f.rule) for f in result] == [("leak.csv", "e0_marker")]
    assert dc.main(["push-scan", "--root", str(root)]) == 1


def test_push_scan_ignores_untracked_and_passes_clean(tmp_path: Path) -> None:
    root = _repo(tmp_path)
    (root / "ok.md").write_text("fine\n", "utf-8")
    _git(root, "add", ".")
    (root / "untracked.txt").write_text(E0_SENTINEL, "utf-8")
    assert dc.push_scan(root) == []
    assert dc.main(["push-scan", "--root", str(root)]) == 0


def test_push_scan_checks_receipt_bodies_by_path(tmp_path: Path) -> None:
    root = _repo(tmp_path)
    (root / "outbox").mkdir()
    (root / "outbox" / "r1.receipt.json").write_text(json.dumps({"data": "E0"}), "utf-8")
    _git(root, "add", ".")
    assert [f.rule for f in dc.push_scan(root)] == ["receipt_data_e0"]


def test_push_scan_ignores_staged_deletion(tmp_path: Path) -> None:
    root = _repo(tmp_path)
    (root / "leak.txt").write_text(E0_SENTINEL, "utf-8")
    _git(root, "add", ".")
    (root / "leak.txt").unlink()
    assert dc.push_scan(root) == []
