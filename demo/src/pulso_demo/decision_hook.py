"""Decision hook: the seam between the demo driver and the human authority (plan steps 8-9).

`DecisionHook.decide(DecisionRequest) -> DecisionResult` is called once, after the revised candidate passed BOTH gates. Implementations:
* `PendingHook` never decides anything (the run stays at `human_decision_pending`);
* `human_flow.FlowHook` runs the approval flow (durable intention -> human gate -> command-authorization JWS from the local human issuer
  -> Core registry approve -> publish -> staging alias read). `DecisionResult.detail` carries the receipt trail (ids and hashes only,
  never a credential)."""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any, Literal, Protocol


@dataclass(frozen=True)
class DecisionRequest:
    run_id: str
    proposal_id: str
    candidate_hash: str
    operation: Literal["approve", "reject"]


@dataclass(frozen=True)
class DecisionResult:
    state: Literal["pending", "approved", "rejected"]
    receipt_ref: str | None = None  # release id once published; None while pending
    detail: dict[str, Any] = field(default_factory=dict)  # stage, trail, aliases, mode, simulated_human (see human_flow)


class DecisionHook(Protocol):
    def decide(self, request: DecisionRequest) -> DecisionResult: ...


class PendingHook:
    """Never decides anything: the human step stays pending."""

    def decide(self, request: DecisionRequest) -> DecisionResult:
        return DecisionResult(state="pending", detail={"stage": "requested", "trail": [], "mode": "none", "simulated_human": False})
