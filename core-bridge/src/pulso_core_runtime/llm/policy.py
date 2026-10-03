"""Runtime-pinned model policy: per task stage, the gateway endpoint alias, model and PRICE table are decided by the
runtime configuration, never by a stage agent, a prompt or the registry release alone. The gateway prices a call from
the price the CALLER sends and has no allowlist, so without this pin a poisoned ModelProfile could turn one 1000/500
token call into a 1500 USD one. Cost is always computed from THIS table."""

from __future__ import annotations

from collections.abc import Mapping
from dataclasses import dataclass
from decimal import ROUND_CEILING, Decimal, InvalidOperation
from typing import Any

MTOK = Decimal(1_000_000)
COST_QUANT = Decimal("0.00000001")  # numeric(20,8) in the meter; rounded UP so spend is never under-counted


@dataclass(frozen=True)
class StageModelPolicy:
    endpoint_alias: str
    model: str
    input_per_mtok: Decimal
    output_per_mtok: Decimal
    max_tokens: int


def cost_for(input_per_mtok: Decimal, output_per_mtok: Decimal, tokens_in: int, tokens_out: int) -> Decimal:
    raw = (Decimal(tokens_in) * input_per_mtok + Decimal(tokens_out) * output_per_mtok) / MTOK
    return raw.quantize(COST_QUANT, rounding=ROUND_CEILING)


class ModelPolicy:
    def __init__(self, stages: Mapping[str, StageModelPolicy]) -> None:
        self._stages = dict(stages)

    def for_stage(self, stage: str) -> StageModelPolicy | None:
        return self._stages.get(stage)

    @property
    def stages(self) -> tuple[str, ...]:
        return tuple(sorted(self._stages))

    def check(self, stage: str, profile: Any | None) -> str | None:
        """None when the profile the registry will send matches the pin; else a stable reason code."""
        pinned = self._stages.get(stage)
        if pinned is None:
            return "stage_not_pinned"
        if profile is None:
            return "profile_unresolved"
        try:
            if getattr(profile, "endpoint_alias", None) != pinned.endpoint_alias:
                return "alias_mismatch"
            if getattr(profile, "model", None) != pinned.model:
                return "model_mismatch"
            price = profile.price
            if (Decimal(price.input_per_mtok) != pinned.input_per_mtok
                    or Decimal(price.output_per_mtok) != pinned.output_per_mtok):
                return "price_mismatch"
            if int(profile.max_tokens) > pinned.max_tokens:
                return "max_tokens_exceeds_pin"
        except (AttributeError, InvalidOperation, TypeError, ValueError):
            return "profile_unresolved"
        return None

    @classmethod
    def from_json(cls, raw: Any) -> ModelPolicy:
        """`{"<stage>": {"endpoint_alias", "model", "input_per_mtok", "output_per_mtok", "max_tokens"}}`; prices are
        decimal strings. Raises `ValueError` naming the stage and field (never a value)."""
        if not isinstance(raw, dict) or not raw:
            raise ValueError("must be a non-empty JSON object keyed by stage")
        stages: dict[str, StageModelPolicy] = {}
        for stage, entry in raw.items():
            if not isinstance(entry, dict):
                raise ValueError(f"stage `{stage}` must be an object")  # noqa: TRY004
            unknown = set(entry) - {"endpoint_alias", "model", "input_per_mtok", "output_per_mtok", "max_tokens"}
            if unknown:
                raise ValueError(f"stage `{stage}` has unknown fields {sorted(unknown)}")
            try:
                alias, model = entry["endpoint_alias"], entry["model"]
                pin, pout = Decimal(str(entry["input_per_mtok"])), Decimal(str(entry["output_per_mtok"]))
                max_tokens = int(entry["max_tokens"])
            except (KeyError, InvalidOperation, TypeError, ValueError):
                raise ValueError(f"stage `{stage}` is incomplete or has a malformed field") from None
            if not (isinstance(alias, str) and alias and isinstance(model, str) and model):
                raise ValueError(f"stage `{stage}` needs a non-empty endpoint_alias and model")
            if not (pin.is_finite() and pout.is_finite() and 0 <= pin <= MTOK and 0 <= pout <= MTOK):
                raise ValueError(f"stage `{stage}` price must be between 0 and 1000000 USD/Mtok")
            if not 1 <= max_tokens <= 1_000_000:
                raise ValueError(f"stage `{stage}` max_tokens must be 1..1000000")
            stages[stage] = StageModelPolicy(alias, model, pin, pout, max_tokens)
        return cls(stages)
