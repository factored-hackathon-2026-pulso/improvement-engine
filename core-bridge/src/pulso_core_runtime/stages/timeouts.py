"""M3 timeout plan. Layers: the responder shim holds a call `hold_s` (55 s); the gateway enforces the registry
profile `timeout_s` (60 s, at most 300 s); `HttpLLMGateway` waits `timeout_s + 5 s`; one stage is at most
`cap x hold_s` seconds and must fit inside the Core invoke timeout (CLT0: `PULSO_CORE_INVOKE_TIMEOUT_S`, default
600 s, 1800 s for live DEMO-0 runs). Raising the profile timeout is a registry change (digest cascade), so a
violation is reported, never silently fixed."""

from __future__ import annotations

from dataclasses import dataclass, field

from pulso_core_runtime.stages.catalog import STEP_CAPS

CLIENT_MARGIN_S = 5.0  # agent_core.adapters.llm.http_gateway.CLIENT_MARGIN_S
GATEWAY_MAX_TIMEOUT_S = 300


@dataclass(frozen=True)
class TimeoutPlan:
    client_wait_s: float
    worst_case_s: dict[str, float]
    problems: list[str] = field(default_factory=list)


def timeout_plan(*, profile_timeout_s: float, invoke_timeout_s: float, hold_s: float) -> TimeoutPlan:
    problems: list[str] = []
    if hold_s <= 0:
        problems.append("hold must be positive")
    if profile_timeout_s > GATEWAY_MAX_TIMEOUT_S:
        problems.append(f"profile timeout_s {profile_timeout_s} exceeds the gateway maximum {GATEWAY_MAX_TIMEOUT_S}")
    if profile_timeout_s <= hold_s:
        problems.append(f"profile timeout_s {profile_timeout_s} must exceed the responder hold {hold_s}")
    worst = {stage: cap * hold_s for stage, cap in STEP_CAPS.items()}
    problems += [f"{stage}: worst case {secs:g} s exceeds the Core invoke timeout {invoke_timeout_s:g} s"
                 for stage, secs in worst.items() if secs > invoke_timeout_s]
    return TimeoutPlan(profile_timeout_s + CLIENT_MARGIN_S, worst, problems)
