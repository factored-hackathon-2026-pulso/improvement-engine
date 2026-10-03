"""Metering v2 (adoption doc D1-D5). One `generate` is: policy check -> atomic reservation against the cap -> the
call -> settlement (always applied, never dropped) + one `model_call_ledger` row.

Reservation = `max_tokens x output price + estimated input tokens x input price`, taken from the price table that
will be sent (the runtime pin when configured). It is released on a known outcome and KEPT as `unknown` when the
spend cannot be known (timeout, `usage_known=false`, a provider-side failure without usage) until reconciled: the
gateway has no idempotency, so a timed-out call may still have billed."""

from __future__ import annotations

import json
import math
from decimal import ROUND_CEILING, Decimal
from typing import Any

from pulso_core_runtime.llm.policy import COST_QUANT, MTOK, ModelPolicy, cost_for

SYSTEM_PROMPT_ALLOWANCE = 1000  # tokens: the registry prompt text is not visible to the wrapper
CHARS_PER_TOKEN = 3  # conservative: more tokens estimated than the typical 4 chars/token
UNCAPPED = "1000000000"
PRICE_TOLERANCE = Decimal("0.000001")  # the gateway rounds cost to 6 decimals


def estimate_reservation(input_per_mtok: Decimal, output_per_mtok: Decimal, max_tokens: int, inputs: Any) -> Decimal:
    try:
        chars = len(json.dumps(inputs, sort_keys=True, default=str))
    except (TypeError, ValueError):
        chars = 0
    tokens_in = math.ceil(chars / CHARS_PER_TOKEN) + SYSTEM_PROMPT_ALLOWANCE
    raw = (Decimal(tokens_in) * input_per_mtok + Decimal(max_tokens) * output_per_mtok) / MTOK
    return raw.quantize(COST_QUANT, rounding=ROUND_CEILING)


def _usage(exc: Any) -> tuple[int, int, Decimal | None] | None:
    tin, tout = getattr(exc, "tokens_in", None), getattr(exc, "tokens_out", None)
    cost = getattr(exc, "cost_usd", None)
    if tin is None and tout is None and cost is None:
        return None
    return int(tin or 0), int(tout or 0), Decimal(cost) if cost is not None else None


class SpendMeteringGateway:
    """Charges every live model call to the bridge meter and ledger. The cap comes from the sealed invocation
    `budget.cost_usd_max`; an exhausted or crossed cap refuses with `GatewayError.refused`. Without a cap the spend is
    recorded against an unreachable cap (metering only, Core budgets still apply). `policy` (optional) pins the
    stage model/price; `profiles(prompt)` resolves the ModelProfile the registry will send (None = unknown)."""

    UNCAPPED = UNCAPPED

    def __init__(self, inner: Any, contexts: Any, store: Any, current: Any = None, *,
                 policy: ModelPolicy | None = None, profiles: Any = None) -> None:
        from pulso_core_runtime.tools.context import current_binding

        self._inner, self._contexts, self._store = inner, contexts, store
        self._current = current or current_binding
        self._policy, self._profiles = policy, profiles

    def generate(self, prompt: Any, inputs_model_view: Any, locale: Any, schema: Any = None) -> Any:
        from agent_core.domain.errors import GatewayError, GatewayErrorKind

        ref = self._current()
        if ref is None:
            return self._inner.generate(prompt, inputs_model_view, locale, schema)
        ic = self._contexts.lookup(ref)
        row = self._store.context_row(ref)
        budget = ((row or {}).get("context") or {}).get("budget") or {}
        cap = str(budget.get("cost_usd_max", UNCAPPED))
        scope = (ic.tenant_id, ic.job_id, ic.stage, ic.attempt)
        profile = self._resolve_profile(prompt)
        pinned = self._policy.for_stage(ic.stage) if self._policy is not None else None
        entry: dict[str, Any] = {"binding_ref": ref, "endpoint_alias": getattr(profile, "endpoint_alias", None),
                                 "model_requested": getattr(profile, "model", None)}
        if self._policy is not None:
            reason = self._policy.check(ic.stage, profile)
            if reason is not None:
                self._store.ledger_record(*scope, outcome="policy_denied", reason=reason, **entry)
                raise GatewayError(GatewayErrorKind.refused)
        prices = self._prices(pinned, profile)
        reserve = Decimal(0)
        if prices is not None:
            max_tokens = pinned.max_tokens if pinned is not None else int(profile.max_tokens)
            reserve = estimate_reservation(prices[0], prices[1], max_tokens, inputs_model_view)
        if not self._store.meter_reserve(*scope, amount=format(reserve, "f"), cap_usd=cap):
            self._store.ledger_record(*scope, outcome="budget_exhausted", reserved_usd=reserve, **entry)
            raise GatewayError(GatewayErrorKind.refused)
        entry["reserved_usd"] = reserve
        try:
            result = self._inner.generate(prompt, inputs_model_view, locale, schema)
        except GatewayError as exc:
            self._settle_failure(scope, cap, exc, prices, entry, reserve)
            raise
        except BaseException:
            # Not a typed gateway failure: the spend is unknown, so the reservation stays held.
            self._store.ledger_record(*scope, outcome="error", usage_known=False, **entry)
            raise
        known = bool(getattr(result, "usage_known", True))
        tin, tout = (int(result.tokens_in), int(result.tokens_out)) if known else (0, 0)
        reported = Decimal(getattr(result, "cost_usd", 0) or 0)
        cost = cost_for(prices[0], prices[1], tin, tout) if prices is not None and known else reported
        entry.update(model_reported=getattr(result, "model", None), gateway_cost_usd=reported,
                     price_mismatch=prices is not None and known and abs(cost - reported) > PRICE_TOLERANCE)
        _, over = self._store.model_call_settle(
            *scope, cap_usd=cap, release_usd=reserve if known else Decimal(0), cost_usd=cost, tokens=tin + tout,
            usage_known=known, tokens_in=tin, tokens_out=tout, outcome="ok", **entry)
        if over:
            raise GatewayError(GatewayErrorKind.refused)
        return result

    def _settle_failure(self, scope: tuple[str, str, str, int], cap: str, exc: Any, prices: Any,
                        entry: dict[str, Any], reserve: Decimal) -> None:
        kind = exc.kind.value
        usage = _usage(exc)
        if usage is not None:
            tin, tout, reported = usage
            cost = cost_for(prices[0], prices[1], tin, tout) if prices is not None and (tin or tout) else (
                reported or Decimal(0))
            release, known = reserve, True
            entry.update(gateway_cost_usd=reported)
        else:
            tin = tout = 0
            cost = Decimal(0)
            keeps = kind in ("timeout", "invalid_output", "refused")  # provider may have billed: stays unknown
            release, known = (Decimal(0) if keeps else reserve), not keeps
        entry.update(model_reported=getattr(exc, "model", None))
        self._store.model_call_settle(
            *scope, cap_usd=cap, release_usd=release, cost_usd=cost, tokens=tin + tout, usage_known=known,
            tokens_in=tin, tokens_out=tout, outcome=kind, **entry)

    def _resolve_profile(self, prompt: Any) -> Any | None:
        if self._profiles is None:
            return None
        try:
            return self._profiles(prompt)
        except Exception:  # noqa: BLE001 - unknown profile: policy (if any) fails closed, metering falls back
            return None

    @staticmethod
    def _prices(pinned: Any, profile: Any) -> tuple[Decimal, Decimal] | None:
        if pinned is not None:
            return pinned.input_per_mtok, pinned.output_per_mtok
        try:
            return Decimal(profile.price.input_per_mtok), Decimal(profile.price.output_per_mtok)
        except (AttributeError, TypeError, ValueError, ArithmeticError):
            return None
