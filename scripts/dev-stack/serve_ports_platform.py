"""EVT3: calibration for `agentcore serve --calibration` that also knows the advisor suggester (`copiloto-sugerencias`).

The e2e demo calibration (`testing.e2e_demo:calibration`) only holds the thresholds of the consultas/disputas demo agents. The decision
model `sin-sugerencia@1` of the suggester takes its thresholds from `cal-sugerencias-provisional` (PROVISIONAL, hand made by agent-core,
not calibrated on data): without it every suggester run ends `failed` at the `clasificar` node before any model call, so no regression
suite for `p/sugerir` could ever be measured. This module = the e2e demo artifacts + that one artifact read from the agent-core
checkout's own fixture (single source of truth, nothing copied). Pass it with PULSO_SERVE_CALIBRATION=serve_ports_platform:calibration.
"""
from __future__ import annotations

from pathlib import Path

from agent_core.composition.serve_ports import DemoContext
from agent_core.decision.calibration.artifact import CalibrationArtifact, InMemoryCalibrationSource

import testing
from testing import e2e_demo

SUGGESTER_CALIBRATION = "cal-sugerencias-provisional"


def suggester_artifact() -> CalibrationArtifact:
    root = Path(testing.__file__).resolve().parents[1]
    path = root / "tests" / "fixtures" / "copiloto-sugerencias" / "calibrations" / f"{SUGGESTER_CALIBRATION}.json"
    return CalibrationArtifact.from_json(path.read_text(encoding="utf-8"))


def calibration(ctx: DemoContext) -> InMemoryCalibrationSource:
    source = e2e_demo.calibration(ctx)
    source.add(suggester_artifact())
    return source
