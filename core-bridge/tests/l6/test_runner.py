"""Scheduling of poll / rescan / 24 h sweep (no Postgres needed: the exporter is a recording stub)."""

from __future__ import annotations

from typing import Any

import pytest

from pulso_core_runtime.exporter import ExporterError, PollReport
from pulso_core_runtime.exporter.report import build_report
from pulso_core_runtime.exporter.runner import Schedule, run_loop

pytestmark = [pytest.mark.l6]


class Rec:
    def __init__(self) -> None:
        self.calls: list[str] = []
        self.fail_polls = 0

    def poll_once(self) -> PollReport:
        self.calls.append("poll")
        if self.fail_polls:
            self.fail_polls -= 1
            raise ExporterError("pulso:schema_drift")
        return PollReport()

    def rescan(self) -> PollReport:
        self.calls.append("rescan")
        return PollReport()

    def sweep(self) -> PollReport:
        self.calls.append("sweep")
        return PollReport()


def test_loop_schedules_poll_rescan_and_24h_sweep() -> None:
    t = {"now": 0.0}
    ex = Rec()
    stats = run_loop(ex, Schedule(10, 100, 86400), monotonic=lambda: t["now"],  # type: ignore[arg-type]
                     sleep=lambda s: t.__setitem__("now", t["now"] + s), max_iterations=8640 + 2)
    assert stats.sweeps == 2 and ex.calls[0] == "sweep"  # sweep at start, then once per 24 h
    assert stats.rescans == 86400 // 100 and stats.polls == 8642
    assert ex.calls.index("rescan") > ex.calls.index("poll")


def test_loop_survives_closed_errors_and_stops() -> None:
    ex = Rec()
    ex.fail_polls = 2
    stats = run_loop(ex, Schedule(1, 10**9, 10**9), monotonic=lambda: 0.0, sleep=lambda s: None,  # type: ignore[arg-type]
                     max_iterations=4)
    assert stats.errors == ["pulso:schema_drift"] * 2 and stats.polls == 2
    n = {"i": 0}

    def stop() -> bool:
        n["i"] += 1
        return n["i"] > 3

    assert run_loop(Rec(), Schedule(1, 10**9, 10**9), monotonic=lambda: 0.0, sleep=lambda s: None,  # type: ignore[arg-type]
                    stop=stop).polls == 3


def test_report_shape() -> None:
    r: dict[str, Any] = build_report({"a": "passed", "b": "failed", "c": "skipped"}, target="fixture", sha="x" * 40,
                                     doubles=["ingest_fixture"])
    assert r["passed"] == ["a"] and r["failed"] == ["b"] and r["not_run"] == ["c"]
    assert r["target"] == "fixture" and r["doubles"] == ["ingest_fixture"] and r["suite"] == "exporter"
