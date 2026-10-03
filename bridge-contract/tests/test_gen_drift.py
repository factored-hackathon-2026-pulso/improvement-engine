"""Drift gate: the published contract is GENERATED from core-bridge/src. These tests fail when the checked-in
artifacts no longer equal what `gen.py` derives from the source (first RED: `gen.py` did not exist)."""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path

import gen

ROOT = Path(__file__).resolve().parents[1]


def test_check_mode_is_clean_for_the_checked_in_artifacts() -> None:
    proc = subprocess.run([sys.executable, str(ROOT / "gen.py"), "--check"], capture_output=True, text=True, check=False,
                          cwd=ROOT, env={**__import__("os").environ, "PYTHONPATH": str(ROOT.parent / "core-bridge" / "src")})
    assert proc.returncode == 0, proc.stdout + proc.stderr


def test_changing_a_source_constant_changes_the_generated_contract(monkeypatch) -> None:
    before = gen.build()
    from pulso_core_runtime.invoke import models
    monkeypatch.setattr(models, "MAX_INPUT_BYTES", models.MAX_INPUT_BYTES + 1)
    assert gen.build() != before, "a limit changed in src but the generated contract did not notice"
