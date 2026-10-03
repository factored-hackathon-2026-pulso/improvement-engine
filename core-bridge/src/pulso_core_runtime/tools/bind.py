"""`pulso/bind_context` (read, args={}): confirms the invocation binding with the control-api (CAP-27)."""

from __future__ import annotations

from agent_core.domain.shared import ToolStatus

from pulso_core_runtime.stages.catalog import CATALOG
from pulso_core_runtime.tools._common import Args, Deps, Outcome, err, ok
from pulso_core_runtime.tools.broker import BrokerError, BrokerTimeout, BrokerUnavailable
from pulso_core_runtime.tools.context import InvocationContext

STAGE_FLAGS = {  # LC-7 (is_builder covers both builder stages)
    "scout": ("is_scout",), "verifier": ("is_verifier",), "builder_design": ("is_builder",),
    "writer": ("is_builder", "is_writer")}


def bind_context(deps: Deps, ic: InvocationContext, args: Args, run_id: str) -> Outcome:
    if deps.contexts.is_confirmed(ic.binding_ref):
        return ok(_facts(ic))
    body = {"schema_version": "1", "tenant_id": ic.tenant_id, "job_id": ic.job_id, "command_key": ic.command_key,
            "request_digest": ic.request_digest, "attempt": ic.attempt, "core_run_id": run_id,
            "bridge_instance_id": ic.bridge_instance_id, "task_binding_ref": ic.binding_ref}
    try:
        deps.control.bind(body, idempotency_key=ic.command_key)
    except BrokerError as exc:
        # 409 binding_conflict|digest_mismatch, 404: final no. 5xx: unknown effect -> stays pending, denied.
        if exc.status in (409, 404):
            deps.contexts.deny(ic.binding_ref)
        return err(f"pulso:binding_failed:{exc.code}", ToolStatus.denied)
    except BrokerTimeout:
        return err("pulso:binding_failed:timeout", ToolStatus.denied)
    except BrokerUnavailable:
        return err("pulso:binding_failed:unavailable", ToolStatus.denied)
    deps.contexts.confirm(ic.binding_ref)
    return ok(_facts(ic))


def _facts(ic: InvocationContext) -> dict[str, object]:
    flags = STAGE_FLAGS.get(ic.stage, ())
    out: dict[str, object] = {"binding_state": "confirmed", "tenant_id": ic.tenant_id, "job_id": ic.job_id}
    for name in ("is_scout", "is_verifier", "is_builder", "is_writer"):
        out[name] = name in flags
    # Core 1.3.0 never validates run-input slots, so Flows read the declared inputs from `facts.binding.value.*`.
    # Only the stage's declared slots are exposed; absent ones are null (writer: evaluate_enabled false).
    for slot in CATALOG[ic.stage].input_slots if ic.stage in CATALOG else ():
        out[slot] = ic.inputs.get(slot)
    if ic.stage == "writer":
        out["evaluate_enabled"] = out.get("evaluate_enabled") is True
    return out
