"""Shared pieces of the `pulso/*` handlers."""

from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass, field
from typing import Any

from agent_core.domain import JsonValue
from agent_core.domain.shared import ToolStatus

from pulso_core_runtime.tools.broker import BrokerClient, ControlApiClient
from pulso_core_runtime.tools.context import InvocationContext, InvocationRegistry

Outcome = tuple[ToolStatus, JsonValue, str | None]  # (status, result_full, error)
Args = dict[str, JsonValue]


def _noop_leak(binding_ref: str, signal: str) -> None:
    return None


@dataclass
class Deps:
    contexts: InvocationRegistry
    broker: BrokerClient
    control: ControlApiClient
    leak_signal: Callable[[str, str], None] = _noop_leak
    leaks: list[tuple[str, str]] = field(default_factory=list)


def ok(value: Any) -> Outcome:
    return ToolStatus.ok, value, None


def err(code: str, status: ToolStatus = ToolStatus.error) -> Outcome:
    return status, None, code


def text_arg(args: Args, name: str) -> str:
    value = args.get(name)
    if not isinstance(value, str) or not value:
        raise ValueError(name)
    return value


Handler = Callable[[Deps, InvocationContext, Args, str], Outcome]  # (deps, ic, args, run_id)
