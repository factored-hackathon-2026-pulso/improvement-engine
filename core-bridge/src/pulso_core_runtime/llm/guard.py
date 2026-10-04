"""Spend guard on the single Python ledger: run-level ceiling, kill switch, ledger-vs-gateway reconciliation.

Pure and dependency-free: the store supplies `spent(scope)` (settled + held spend from the bridge meter); the metering
wrapper maps the exceptions here to a `refused` gateway error and a ledger row."""

from __future__ import annotations

import os
import threading
from collections.abc import Callable, Iterable, Mapping
from dataclasses import dataclass
from decimal import Decimal
from pathlib import Path
from typing import Any

KILL_ENV = "PULSO_LLM_KILL"
_TRUE = {"1", "true", "yes", "on"}


class CeilingExceeded(Exception):
    """The call would push the run past its spend ceiling."""


class KillSwitchEngaged(Exception):
    """Operator kill switch is on: no model call may start."""


@dataclass(frozen=True)
class KillSwitch:
    env: Mapping[str, str] | None = None
    file: Path | None = None

    def engaged(self) -> bool:
        env = os.environ if self.env is None else self.env
        if env.get(KILL_ENV, "").strip().lower() in _TRUE:
            return True
        if self.file is None:
            return False
        try:
            os.stat(self.file)
        except FileNotFoundError:
            return False
        except (OSError, ValueError):
            return True  # unreadable marker: fail closed
        return True

    def check(self) -> None:
        if self.engaged():
            raise KillSwitchEngaged("llm kill switch engaged")


class SpendGuard:
    def __init__(self, *, ceiling_usd: Decimal | None, spent: Callable[[tuple], Decimal],
                 kill: KillSwitch | None = None) -> None:
        if ceiling_usd is not None and (not ceiling_usd.is_finite() or ceiling_usd < 0):
            raise ValueError("ceiling_usd must be a finite non-negative decimal")
        self._ceiling, self._spent, self._kill = ceiling_usd, spent, kill
        self.lock = threading.RLock()  # callers hold it across check -> reserve so concurrent calls cannot both pass

    def check(self, scope: tuple, reserve: Decimal) -> None:
        if self._kill is not None:
            self._kill.check()
        if self._ceiling is None:
            return
        spent = self._spent(scope)
        if not (spent.is_finite() and reserve.is_finite() and reserve >= 0) or spent + reserve > self._ceiling:
            raise CeilingExceeded("run spend ceiling would be exceeded")


@dataclass(frozen=True)
class Reconciliation:
    ledger_tokens: int
    gateway_tokens: int
    unknown_rows: int
    tolerance: int

    @property
    def delta(self) -> int:
        return abs(self.ledger_tokens - self.gateway_tokens)

    @property
    def ok(self) -> bool:
        return self.delta <= self.tolerance


def _tokens(rows: Iterable[Mapping[str, Any]]) -> tuple[int, int]:
    total = unknown = 0
    for r in rows:
        if r.get("usage_known", True) is False:
            unknown += 1
            continue
        total += int(r.get("tokens_in") or 0) + int(r.get("tokens_out") or 0)
    return total, unknown


def reconcile_ledger(ledger_rows: Iterable[Mapping[str, Any]], gateway_rows: Iterable[Mapping[str, Any]],
                     tolerance: int = 1) -> Reconciliation:
    """Ledger token sum must equal the gateway-reported sum within `tolerance` tokens (default 1)."""
    lt, unknown = _tokens(ledger_rows)
    gt, _ = _tokens(gateway_rows)
    return Reconciliation(lt, gt, unknown, tolerance)
