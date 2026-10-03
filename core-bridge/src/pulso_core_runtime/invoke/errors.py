"""Typed bridge errors (plan 17.3.3 error table). `code` is `pulso:*`; `status` the HTTP status."""

from __future__ import annotations

from typing import Any


class BridgeError(Exception):
    def __init__(self, code: str, status: int, *, retryable: bool = False, details: dict[str, Any] | None = None):
        super().__init__(code)
        self.code, self.status, self.retryable, self.details = code, status, retryable, details or {}


def unknown_input_slot(slots: list[str]) -> BridgeError:
    return BridgeError("pulso:unknown_input_slot", 400, details={"slots": sorted(slots)})


def input_too_large() -> BridgeError:
    return BridgeError("pulso:input_too_large", 413)


def stage_unknown(stage: str) -> BridgeError:
    return BridgeError("pulso:stage_unknown", 400, details={"stage": stage})


def stage_agent_mismatch(stage: str, agent_id: str) -> BridgeError:
    return BridgeError("pulso:stage_agent_mismatch", 422, details={"stage": stage, "agent_id": agent_id})


def digest_conflict() -> BridgeError:
    return BridgeError("pulso:digest_conflict", 409)


def bridge_busy() -> BridgeError:
    return BridgeError("pulso:bridge_busy", 429, retryable=True)


def release_error(code: str) -> BridgeError:
    return BridgeError(code, 409)


def core_unavailable() -> BridgeError:
    return BridgeError("pulso:core_unavailable", 503, retryable=True)
