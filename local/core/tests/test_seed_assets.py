"""Unit tests for the idempotent seed core (no Postgres, no Core). Run: uv run --python 3.12 --with pytest pytest local/core/tests"""

from __future__ import annotations

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "init"))

import seed_assets  # noqa: E402


class FakeConn:
    def __init__(self, stored: str | None = None) -> None:
        self.stored, self.writes, self.commits = stored, 0, 0

    def execute(self, sql: str, params: tuple = ()):  # noqa: ANN201
        if sql.lstrip().upper().startswith("SELECT"):
            return self
        self.writes += 1
        self.stored = params[1]
        return self

    def fetchone(self):  # noqa: ANN201
        return (self.stored,) if self.stored is not None else None

    def commit(self) -> None:
        self.commits += 1


def test_first_run_imports_and_records_marker() -> None:
    conn, calls = FakeConn(), []
    status, releases = seed_assets.seed_once(conn, "d1", lambda: calls.append(1) or ["rel-a"])
    assert (status, releases, len(calls), conn.writes, conn.commits) == ("seeded", ["rel-a"], 1, 1, 1)


def test_second_run_with_same_digest_writes_nothing() -> None:
    conn, calls = FakeConn("d1"), []
    status, releases = seed_assets.seed_once(conn, "d1", lambda: calls.append(1) or ["x"])
    assert (status, releases, calls, conn.writes, conn.commits) == ("unchanged", [], [], 0, 0)


def test_changed_assets_reseed() -> None:
    conn = FakeConn("old")
    assert seed_assets.seed_once(conn, "new", lambda: ["r"])[0] == "seeded"
    assert conn.stored == "new"


def test_failed_import_leaves_no_marker() -> None:
    conn = FakeConn()

    def boom() -> list[str]:
        raise RuntimeError("x")

    try:
        seed_assets.seed_once(conn, "d", boom)
    except RuntimeError:
        pass
    assert conn.writes == 0 and conn.stored is None


def test_assets_digest_ignores_tests_and_tools_and_tracks_content(tmp_path: Path) -> None:
    (tmp_path / "worlds").mkdir()
    (tmp_path / "worlds" / "a.yaml").write_text("a: 1")
    (tmp_path / "tests").mkdir()
    (tmp_path / "tests" / "t.py").write_text("x")
    base = seed_assets.assets_digest(tmp_path)
    (tmp_path / "tests" / "t.py").write_text("changed")
    assert seed_assets.assets_digest(tmp_path) == base
    (tmp_path / "worlds" / "a.yaml").write_text("a: 2")
    assert seed_assets.assets_digest(tmp_path) != base


def test_real_assets_digest_is_stable() -> None:
    root = Path(__file__).resolve().parents[3] / "agent-core-assets"
    assert seed_assets.assets_digest(root) == seed_assets.assets_digest(root)


def test_gen_keys_is_idempotent(tmp_path: Path, monkeypatch) -> None:  # noqa: ANN001
    import gen_keys

    monkeypatch.setattr(gen_keys.os, "chown", lambda *a, **k: None, raising=False)
    assert gen_keys.main(tmp_path) == 0
    snapshot = {p.name: p.read_text() for p in tmp_path.iterdir()}
    assert {"identity.json", "staff.json", "service.json", "exporter-control-api.key", "smoke-probe.key"} <= set(snapshot)
    assert gen_keys.main(tmp_path) == 0
    assert {p.name: p.read_text() for p in tmp_path.iterdir()} == snapshot
