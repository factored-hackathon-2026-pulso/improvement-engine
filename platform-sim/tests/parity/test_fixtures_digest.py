"""The fixtures digest must not depend on the checkout's line endings (git autocrlf) nor on file order."""

from __future__ import annotations

from registry_mock.sim_common import fixtures_digest


def test_digest_is_line_ending_independent(tmp_path) -> None:
    lf, crlf = tmp_path / "lf", tmp_path / "crlf"
    lf.mkdir()
    crlf.mkdir()
    (lf / "a.json").write_bytes(b'{\n  "x": 1\n}\n')
    (crlf / "a.json").write_bytes(b'{\r\n  "x": 1\r\n}\r\n')
    assert fixtures_digest(lf) == fixtures_digest(crlf)


def test_digest_changes_with_content(tmp_path) -> None:
    (tmp_path / "a.json").write_bytes(b"{}\n")
    before = fixtures_digest(tmp_path)
    (tmp_path / "a.json").write_bytes(b'{"x":1}\n')
    assert fixtures_digest(tmp_path) != before


def test_recorded_fixtures_are_lf_only() -> None:
    from registry_mock.sim_common import FIXTURES_DIR

    assert not [p.name for p in FIXTURES_DIR.glob("*.json") if b"\r\n" in p.read_bytes()]
