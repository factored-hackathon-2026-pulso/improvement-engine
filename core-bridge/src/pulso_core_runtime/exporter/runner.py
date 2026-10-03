"""Exporter scheduling: poll_once every `poll_interval`, rescan (anti-entropy + open-run prefixes) every
`rescan_interval`, full-chain sweep every `sweep_interval` (24 h). Clock/sleep are injected for tests."""

from __future__ import annotations

import time
from collections.abc import Callable
from dataclasses import dataclass, field

from .reader import ExporterError
from .service import Exporter


@dataclass(frozen=True)
class Schedule:
    poll_interval: float = 5.0
    rescan_interval: float = 900.0
    sweep_interval: float = 24 * 3600.0


@dataclass
class LoopStats:
    polls: int = 0
    rescans: int = 0
    sweeps: int = 0
    errors: list[str] = field(default_factory=list)  # closed error codes only, never payloads


def run_loop(ex: Exporter, schedule: Schedule = Schedule(), *, monotonic: Callable[[], float] = time.monotonic,
             sleep: Callable[[float], None] = time.sleep, stop: Callable[[], bool] = lambda: False,
             max_iterations: int | None = None) -> LoopStats:
    stats = LoopStats()
    start = monotonic()
    next_rescan, next_sweep = start + schedule.rescan_interval, start
    i = 0
    while not stop() and (max_iterations is None or i < max_iterations):
        i += 1
        now = monotonic()
        try:
            if now >= next_sweep:
                ex.sweep()
                stats.sweeps += 1
                next_sweep = now + schedule.sweep_interval
            ex.poll_once()
            stats.polls += 1
            if now >= next_rescan:
                ex.rescan()
                stats.rescans += 1
                next_rescan = now + schedule.rescan_interval
        except ExporterError as exc:  # eval misconfiguration / schema drift / over-privilege: keep alive, retry
            stats.errors.append(exc.code)
        sleep(schedule.poll_interval)
    return stats
