"""Shared fixtures for the agent-core-assets validator suite (Python 3.12, standalone)."""

from __future__ import annotations

import os
import shutil
import sys
from pathlib import Path

import pytest

ASSETS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ASSETS / "tools"))

PIN_SHA = "86a767474042a566a0dbd6ed23588959f27ebdb3"


def find_checkout() -> Path | None:
    env = os.environ.get("AGENT_CORE_CHECKOUT")
    if env:
        return Path(env)
    for parent in ASSETS.parents:
        cand = parent / "references" / "agent-core"
        if cand.is_dir():
            return cand
    return None


@pytest.fixture()
def assets(tmp_path: Path) -> Path:
    """A throwaway copy of the assets tree that tests may mutate."""
    dest = tmp_path / "agent-core-assets"
    shutil.copytree(ASSETS, dest, ignore=shutil.ignore_patterns("tests", "__pycache__", "build", ".pytest_cache"))
    return dest


@pytest.fixture(scope="session")
def checkout() -> Path:
    path = find_checkout()
    if path is None or not path.is_dir():
        pytest.skip("agent-core checkout not found (set AGENT_CORE_CHECKOUT)")
    return path
