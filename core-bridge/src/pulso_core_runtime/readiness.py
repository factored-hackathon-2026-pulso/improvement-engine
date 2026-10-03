"""Readiness checks (names only are ever reported by `/readyz`, 503 on any failure)."""

from __future__ import annotations

from collections.abc import Callable
from pathlib import Path

from pulso_core_runtime.internal.store import schema_ready


def key_files_check(paths: list[Path]) -> Callable[[], bool]:
    def check() -> bool:
        return all(p.is_file() and p.stat().st_size > 0 for p in paths)
    return check


def eval_db_check(ping: Callable[[], bool]) -> tuple[str, Callable[[], bool]]:
    return ("eval_db", ping)


def bridge_schema_check(dsn: str) -> tuple[str, Callable[[], bool]]:
    return ("bridge_schema", lambda: schema_ready(dsn))


def factories_ok_check(bound: set[str], expected: tuple[str, ...]) -> tuple[str, Callable[[], bool]]:
    return ("factories_ok", lambda: set(expected) <= bound)
