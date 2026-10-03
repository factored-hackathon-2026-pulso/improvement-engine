"""`EvalBudgetMeter` (plan 17.3.5 Budget): wraps the gateway, sums cost/tokens, counts jobs, enforces a deadline.

Core's `LimitGuard` is bypassed by the harness, so this is the only limiter. The engine absorbs gateway
errors, therefore exhaustion is recorded on the meter and the harness raises `HarnessUnavailable` after the
run (the same probe pattern as upstream). Exhaustion is `failed_infra`, never a pass."""

from __future__ import annotations

import threading
from collections.abc import Callable
from dataclasses import dataclass
from datetime import UTC, datetime
from decimal import Decimal
from typing import Any

from agent_core.domain import EntityRef, GatewayError, GatewayErrorKind, JsonValue, Locale
from agent_core.ports import GenerationResult
from agent_core.registry import HarnessUnavailable


@dataclass(frozen=True)
class BudgetLimits:
    budget_ref: str
    cost_usd_max: Decimal | None = None
    tokens_max: int | None = None
    jobs_max: int | None = None
    deadline: datetime | None = None


class EvalBudgetMeter:
    def __init__(self, limits: BudgetLimits, now: Callable[[], datetime] = lambda: datetime.now(UTC)) -> None:
        self.limits = limits
        self._now = now
        self._lock = threading.Lock()
        self.jobs = 0
        self.cost_usd = Decimal(0)
        self.tokens_in = 0
        self.tokens_out = 0
        self.usage_known = True
        self.exhausted: str | None = None

    def _mark(self, reason: str) -> None:
        if self.exhausted is None:
            self.exhausted = reason

    def start_job(self) -> None:
        """Called before every harness run; raises when the job count or the deadline is exhausted."""
        with self._lock:
            if self.limits.deadline is not None and self._now() >= self.limits.deadline:
                self._mark("deadline")
            elif self.limits.jobs_max is not None and self.jobs >= self.limits.jobs_max:
                self._mark("jobs_max")
            if self.exhausted is not None:
                raise HarnessUnavailable("budget_exhausted")
            self.jobs += 1

    def check_after_run(self) -> None:
        with self._lock:
            if self.exhausted is not None:
                raise HarnessUnavailable("budget_exhausted")

    def record(self, result: GenerationResult) -> None:
        with self._lock:
            self.cost_usd += result.cost_usd
            self.tokens_in += result.tokens_in
            self.tokens_out += result.tokens_out
            if not result.usage_known:
                self.usage_known = False
            lim = self.limits
            if lim.cost_usd_max is not None and self.cost_usd > lim.cost_usd_max:
                self._mark("cost_usd_max")
            if lim.tokens_max is not None and self.tokens_in + self.tokens_out > lim.tokens_max:
                self._mark("tokens_max")
            if lim.deadline is not None and self._now() >= lim.deadline:
                self._mark("deadline")

    def blocked(self) -> bool:
        with self._lock:
            return self.exhausted is not None

    def usage(self) -> dict[str, Any] | None:
        """`None` when any call lacked usage (reported as `usage=null`, `cost_known=false`)."""
        with self._lock:
            if not self.usage_known:
                return None
            return {"cost_usd": str(self.cost_usd), "tokens_in": self.tokens_in, "tokens_out": self.tokens_out,
                    "jobs": self.jobs}

    @property
    def cost_known(self) -> bool:
        return self.usage_known

    def wrap(self, gateway: Any) -> Any:
        return _MeteredGateway(gateway, self)


class _MeteredGateway:
    def __init__(self, inner: Any, meter: EvalBudgetMeter) -> None:
        self._inner, self._meter = inner, meter

    def generate(self, prompt: EntityRef, inputs_model_view: dict[str, JsonValue], locale: Locale,
                 schema: dict[str, JsonValue] | None = None) -> GenerationResult:
        if self._meter.blocked():
            raise GatewayError(GatewayErrorKind.unavailable)  # absorbed by the engine; the harness probes
        result: GenerationResult = self._inner.generate(prompt, inputs_model_view, locale, schema)
        self._meter.record(result)
        return result
