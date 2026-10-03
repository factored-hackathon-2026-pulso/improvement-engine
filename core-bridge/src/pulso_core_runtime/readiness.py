"""Readiness checks (names only are ever reported by `/readyz`, 503 on any failure)."""

from __future__ import annotations

from collections.abc import Callable
from pathlib import Path
from typing import Any

from pulso_core_runtime.internal.store import schema_ready


def key_files_check(paths: list[Path],
                    verifiers: Callable[[], list[Any]] | None = None) -> Callable[[], bool]:
    """Files present and non-empty AND no verifier reports a failed key reload (a broken reload keeps the last good
    keys, so revoked keys would stay valid while the file looks fine): NF-04."""
    def check() -> bool:
        if not all(p.is_file() and p.stat().st_size > 0 for p in paths):
            return False
        return all(getattr(v, "last_reload_error", None) is None for v in (verifiers() if verifiers else []))
    return check


def eval_db_check(ping: Callable[[], bool]) -> tuple[str, Callable[[], bool]]:
    return ("eval_db", ping)


def bridge_schema_check(dsn: str) -> tuple[str, Callable[[], bool]]:
    return ("bridge_schema", lambda: schema_ready(dsn))


def factories_ok_check(bound: set[str], expected: tuple[str, ...]) -> tuple[str, Callable[[], bool]]:
    return ("factories_ok", lambda: set(expected) <= bound)
