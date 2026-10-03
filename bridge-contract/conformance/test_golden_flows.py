"""Replays the golden flows (examples/flows/*.json) against the target and compares the normalised exchanges.
Record/refresh the goldens from the REAL runtime with CONTRACT_RECORD=1 (real target only)."""

from __future__ import annotations

import os

import pytest

from conformance.flows import FLOWS, run_flow
from conformance.worlds import World


@pytest.mark.parametrize("name", list(FLOWS))
def test_golden_flow(world: World, name: str) -> None:
    record = os.environ.get("CONTRACT_RECORD") == "1"
    if record and world.target != "real":
        pytest.fail("goldens are recorded from the real runtime only")
    run_flow(world, name, record)
