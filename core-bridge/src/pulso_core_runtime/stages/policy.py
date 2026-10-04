"""M3 stage policy alignment: the per-stage pin (`PULSO_LLM_STAGE_POLICY`) is DERIVED from the registry
ModelProfile the Flows' prompts reference, so alias, model and price can never drift from it (the registry bytes
are never edited to fit the policy: that would move release digests). The writer is a projection and has no pin."""

from __future__ import annotations

from decimal import Decimal
from pathlib import Path
from types import SimpleNamespace
from typing import Any

import yaml

from pulso_core_runtime.llm.policy import ModelPolicy, StageModelPolicy
from pulso_core_runtime.stages.catalog import STEP_CAPS


def load_registry_profile(path: Path | str) -> Any:
    """The ModelProfile YAML as a namespace with the attributes `ModelPolicy.check` reads."""
    raw = yaml.safe_load(Path(path).read_text(encoding="utf-8"))
    price = raw["price"]
    return SimpleNamespace(
        endpoint_alias=raw["endpoint_alias"], model=raw["model"], max_tokens=int(raw["max_tokens"]),
        timeout_s=int(raw["timeout_s"]), temperature=raw["temperature"], structured=raw["structured"],
        price=SimpleNamespace(input_per_mtok=Decimal(str(price["input_per_mtok"])),
                              output_per_mtok=Decimal(str(price["output_per_mtok"]))))


def evolution_policy(profile: Any) -> ModelPolicy:
    pin = StageModelPolicy(profile.endpoint_alias, profile.model, Decimal(profile.price.input_per_mtok),
                           Decimal(profile.price.output_per_mtok), int(profile.max_tokens))
    return ModelPolicy({stage: pin for stage in STEP_CAPS})
