"""Human decision hook (CLEARLY MARKED STUB). Approve/publish is an authority operation owned by local-identity (built by another
agent). The demo only defines the interface; the default implementation never approves: the run stays at `human_decision_pending`.

Wiring contract: `local-identity` supplies an object with `decide(DecisionRequest) -> DecisionResult`; the demo driver calls it once
after the second evaluation passes, and the translator would then mark approve/publish from REAL receipts only."""

from __future__ import annotations

from dataclasses import dataclass
from typing import Literal, Protocol


@dataclass(frozen=True)
class DecisionRequest:
    run_id: str
    proposal_id: str
    candidate_hash: str
    operation: Literal["approve", "reject"]


@dataclass(frozen=True)
class DecisionResult:
    state: Literal["pending", "approved", "rejected"]
    receipt_ref: str | None = None  # signed local-identity receipt; None while pending


class DecisionHook(Protocol):
    def decide(self, request: DecisionRequest) -> DecisionResult: ...


class PendingHook:
    """TODO(local-identity): replace with the signed-approval adapter. This stand-in never decides anything."""

    def decide(self, request: DecisionRequest) -> DecisionResult:
        return DecisionResult(state="pending")
