"""`ProtectedBuilderToolExecutor` (plan 17.3.3 "Protected writer").

Wraps (never subclasses) `BuilderToolExecutor(service, actor, ids)` -- whose `_handlers` are private -- with:
per-stage allow-list; `RegistryMutationCommitment` match (else `denied commitment_mismatch`, zero effect);
re-validated broker authorisation; write-key derivation; shaped results. `registry/evaluate` is handled HERE and
never delegated to `BuilderToolExecutor._evaluate`: it needs `evaluate_enabled` and an admitted
`evaluation_context_ref` from the frozen context (never tool args), and runs through an injected
`EvaluationGate` (L5: admission checks iii-vi/viii + per-admission service clone).

Write keys (A02, CLQ-10): `pulso-w:` + sha256("<command_key>|<stage>|<ordinal>")[:48] with `ordinal` the
position of the write within the invocation (first write = 0); the engine's own action id is translated and
`registry/get_write` is translated back, so reconciliation reads the derived key. Evaluate key:
`pulso-eval:<evaluation_context_ref>` (ref <= 200 chars of [A-Za-z0-9_.:-]; invalid -> denied, never truncated)."""

from __future__ import annotations

import hashlib
import re
from typing import Any, Protocol

from agent_core.composition.builder_tools import BUILDER_TOOL_DEFS
from agent_core.domain import EntityRef, JsonValue, ToolDef
from agent_core.domain.json import canonical_bytes
from agent_core.domain.shared import ToolStatus
from agent_core.ports import ToolCallContext, ToolResult
from agent_core.ports.ids import IdKind

from pulso_core_runtime.tools import authcheck
from pulso_core_runtime.tools._common import Outcome
from pulso_core_runtime.tools.broker import BrokerClient
from pulso_core_runtime.tools.context import (
    ContextError,
    InvocationContext,
    InvocationRegistry,
    RegistryMutationCommitment,
    resolve_context,
)

WRITE_PREFIX = "pulso-w:"
EVAL_PREFIX = "pulso-eval:"
_EVAL_REF = re.compile(r"[A-Za-z0-9_.:-]{1,200}\Z")  # `\Z`: a trailing newline is not a ref (call sites use .match)

WRITE_MODE_TOOLS = frozenset({"registry/create_proposal", "registry/put_draft", "registry/freeze",
                              "registry/reopen", "registry/validate", "registry/get_proposal",
                              "registry/get_write"})
EVALUATE = "registry/evaluate"
EVALUATE_ONLY_TOOLS = frozenset({"registry/validate", "registry/get_proposal", "registry/get_write"})
MUTATORS = frozenset({"registry/create_proposal", "registry/put_draft", "registry/freeze", "registry/reopen",
                      EVALUATE})
_CREATED = "created_proposal_id"
_DERIVED = "derived_keys"
_MAX_ORDINALS = 32


class EvaluationGate(Protocol):
    """L5 hook: runs checks (iii)-(vi),(viii) and the per-admission service clone; replays by `key`."""

    def evaluate(self, ic: InvocationContext, proposal_id: str, idempotency_key: str,
                 requested_suite: tuple[str | None, str | None]) -> Outcome: ...


def write_key(command_key: str, stage: str, ordinal: int) -> str:
    return WRITE_PREFIX + hashlib.sha256(f"{command_key}|{stage}|{ordinal}".encode()).hexdigest()[:48]


def eval_key(evaluation_context_ref: str) -> str:
    if not _EVAL_REF.match(evaluation_context_ref):
        raise ValueError("pulso:evaluation_context_ref_invalid")
    return EVAL_PREFIX + evaluation_context_ref


def put_draft_digest(proposal_id: str | None, expected_rev: int | None, changes: Any) -> str:
    return hashlib.sha256(canonical_bytes({"proposal_id": proposal_id, "expected_rev": expected_rev,
                                           "changes": changes})).hexdigest()


def _digest(value: str) -> str:
    return hashlib.sha256(value.encode()).hexdigest()


RELEASE_SETTINGS_KIND = "release_settings"


def _has_release_settings(changes: Any) -> bool:
    return isinstance(changes, list) and any(isinstance(c, dict) and c.get("kind") == RELEASE_SETTINGS_KIND
                                             for c in changes)


def _denied(code: str) -> tuple[ToolStatus, JsonValue, str]:
    return ToolStatus.denied, None, code


class ProtectedBuilderToolExecutor:
    def __init__(self, inner: Any, contexts: InvocationRegistry, broker: BrokerClient, *,
                 gate: EvaluationGate | None = None, ids: Any = None, admissions: Any = None) -> None:
        self._inner, self._contexts, self._broker, self._gate = inner, contexts, broker, gate
        self._ids = ids
        self._admissions = admissions  # `AdmissionStore`-like (`get(ref)`); needed for the canonical digest

    def definition(self, tool: EntityRef) -> ToolDef:
        found = BUILDER_TOOL_DEFS.get(tool.id)
        if found is None or found.version != tool.version:
            raise KeyError(str(tool))
        return found

    def _call_id(self) -> str:
        if self._ids is not None:
            return str(self._ids.new_id(IdKind.call))
        return "call-" + hashlib.sha256(repr(id(object())).encode()).hexdigest()[:16]

    def _result(self, out: tuple[ToolStatus, JsonValue, str | None]) -> ToolResult:
        return ToolResult(status=out[0], result_full=out[1], error=out[2], call_id=self._call_id())

    # -- ToolExecutor ----------------------------------------------------------------------------
    def execute(self, tool: EntityRef, args: dict[str, JsonValue], bound_params: dict[str, str],
                ctx: ToolCallContext, idempotency_key: str | None = None) -> ToolResult:
        definition = BUILDER_TOOL_DEFS.get(tool.id)
        if definition is None or definition.version != tool.version:
            return self._result((ToolStatus.error, None, "unregistered_tool"))
        try:
            ic = resolve_context(ctx, self._contexts)
        except ContextError as exc:
            return self._result(_denied(exc.code))
        if not self._contexts.is_confirmed(ic.binding_ref):
            return self._result(_denied("pulso:binding_unconfirmed"))
        gate_error = self._gate_checks(tool.id, args, ic)
        if gate_error is not None:
            return self._result(gate_error)
        commitment = ic.commitment
        assert commitment is not None  # _gate_checks guarantees it
        if tool.id == EVALUATE:
            return self._result(self._evaluate(ic, commitment, args, idempotency_key))
        return self._delegate(tool, args, bound_params, ctx, idempotency_key, ic, definition)

    # -- checks (zero effect on any failure) ---------------------------------------------------------
    def _gate_checks(self, name: str, args: dict[str, JsonValue],
                     ic: InvocationContext) -> tuple[ToolStatus, JsonValue, str] | None:
        commitment = ic.commitment
        if ic.stage != "writer" or commitment is None:
            return _denied("tool_not_allowed")
        evaluate_ok = commitment.evaluate_enabled and commitment.evaluation_context_ref is not None
        if commitment.mode == "evaluate_only":
            allowed = EVALUATE_ONLY_TOOLS | ({EVALUATE} if evaluate_ok else set())
        else:
            allowed = WRITE_MODE_TOOLS | ({EVALUATE} if evaluate_ok else set())
        if name not in allowed:
            return _denied("tool_not_allowed")
        props = BUILDER_TOOL_DEFS[name].args_schema.get("properties", {})
        if not isinstance(props, dict) or not set(args) <= set(props):
            return _denied("invalid_args")
        if name == "registry/put_draft" and _has_release_settings(args.get("changes")):
            # N-07 (agent-core 789d6c8): `release_settings` can replace the whole interrupt list. Default deny, checked
            # before the commitment so even a committed digest cannot carry it, until a Pulso guardrail exists (D-17).
            return _denied("pulso:release_settings_not_allowed")
        mismatch = self._commitment_mismatch(name, args, ic, commitment)
        if mismatch:
            return _denied("commitment_mismatch")
        if name in MUTATORS and name != EVALUATE:
            denial = authcheck.check(self._contexts, self._broker, ic, name, self._resources(name, args, ic),
                                     self._payload_digest(name, args))
            if denial:
                return _denied(denial)
        return None

    def _expected_pid(self, ic: InvocationContext, c: RegistryMutationCommitment) -> str | None:
        return c.proposal_id or self._contexts.recall(ic.binding_ref, _CREATED)

    def _commitment_mismatch(self, name: str, args: dict[str, JsonValue], ic: InvocationContext,
                             c: RegistryMutationCommitment) -> bool:
        if name == "registry/create_proposal":
            return not (c.proposal_id is None and c.create_title is not None
                        and args.get("agent_id") == c.create_agent_id and args.get("origin") == c.create_origin
                        and args.get("title") == c.create_title)
        if name in ("registry/get_write",):
            return False
        pid = self._expected_pid(ic, c)
        if args.get("proposal_id") != pid or pid is None:
            return True
        if name == "registry/put_draft":
            if c.expected_rev is not None and args.get("expected_rev") != c.expected_rev:
                return True
            if c.put_draft_digest is None:
                return True
            known = c.proposal_id is not None
            digest = put_draft_digest(pid if known else None, c.expected_rev if known else None,
                                      args.get("changes"))
            # Fresh proposals (id unknown at commit time) are bound with null id/rev in the digest.
            return digest != c.put_draft_digest
        return False

    @staticmethod
    def _resources(name: str, args: dict[str, JsonValue], ic: InvocationContext) -> list[str]:
        pid = args.get("proposal_id") or (ic.commitment.proposal_id if ic.commitment else None)
        return [str(args.get("agent_id") or pid or ic.binding_ref)]

    @staticmethod
    def _payload_digest(name: str, args: dict[str, JsonValue]) -> str | None:
        return hashlib.sha256(canonical_bytes({"op": name, "args": args})).hexdigest()

    # -- evaluate (never delegated) -------------------------------------------------------------------
    def _evaluate(self, ic: InvocationContext, c: RegistryMutationCommitment, args: dict[str, JsonValue],
                  engine_key: str | None) -> tuple[ToolStatus, JsonValue, str | None]:
        ref = c.evaluation_context_ref
        assert ref is not None
        try:
            key = eval_key(ref)
        except ValueError as exc:
            return _denied(str(exc))
        pid = args.get("proposal_id")
        if not isinstance(pid, str) or pid != self._expected_pid(ic, c):
            return _denied("commitment_mismatch")
        if self._gate is None:
            return _denied("pulso:evaluation_gate_unavailable")  # fail-closed
        adm = self._admissions.get(ref) if self._admissions is not None else None
        if adm is None:
            return _denied("pulso:admission_missing")
        from pulso_core_runtime.evaluation.digests import native_evaluate_digest

        digest = native_evaluate_digest(
            proposal_id=pid, evaluation_context_ref=ref, candidate_hash=adm.candidate_hash,
            suite_id=adm.suite_id, suite_version=adm.suite_version, suite_digest=adm.suite_digest)
        denial = authcheck.check(self._contexts, self._broker, ic, "native_evaluate", [pid, ref], digest)
        if denial:
            return _denied(denial)
        if engine_key:
            self._remember_key(ic, engine_key, key)
        suite = (args.get("suite_id") if isinstance(args.get("suite_id"), str) else None,
                 args.get("suite_version") if isinstance(args.get("suite_version"), str) else None)
        try:
            return self._gate.evaluate(ic, pid, key, suite)
        except Exception:  # noqa: BLE001 - effect unknown after a gate crash
            return ToolStatus.uncertain, None, "pulso:evaluation_exception"

    # -- delegation -------------------------------------------------------------------------------------
    def _remember_key(self, ic: InvocationContext, engine_key: str, derived: str) -> None:
        self._contexts.remember_in_map(ic.binding_ref, _DERIVED, engine_key, derived)

    def _derive(self, ic: InvocationContext, engine_key: str, op: str) -> str | None:
        operations = ic.commitment.operations if ic.commitment else ()
        ordinal = self._contexts.ordinal(ic.binding_ref, engine_key, op, operations)
        if ordinal is None:
            return None  # write not (or no longer) in the committed operation array
        derived = write_key(ic.command_key, ic.stage, ordinal)
        self._remember_key(ic, engine_key, derived)
        return derived

    @staticmethod
    def _own_keys(ic: InvocationContext) -> frozenset[str]:
        """Keys this command can have produced (crash recovery reads them without the in-memory map)."""
        committed = ic.commitment.operations if ic.commitment else ()
        keys = {write_key(ic.command_key, ic.stage, n) for n in range(len(committed) or _MAX_ORDINALS)}
        ref = ic.evaluation_context_ref
        if ref is not None and _EVAL_REF.match(ref):
            keys.add(eval_key(ref))
        return frozenset(keys)

    def _delegate(self, tool: EntityRef, args: dict[str, JsonValue], bound_params: dict[str, str],
                  ctx: ToolCallContext, engine_key: str | None, ic: InvocationContext,
                  definition: ToolDef) -> ToolResult:
        derived: str | None = None
        send_args = dict(args)
        if definition.is_write:
            if not engine_key:
                return self._result(_denied("pulso:write_without_key"))
            if tool.id == "registry/create_proposal" and                     self._contexts.claim(ic.binding_ref, "create_engine_key", engine_key) != engine_key:
                return self._result(_denied("commitment_mismatch"))  # one proposal per invocation
            derived = self._derive(ic, engine_key, tool.id.removeprefix("registry/"))
            if derived is None:
                return self._result(_denied("commitment_mismatch"))
        elif tool.id == "registry/get_write":
            requested = args.get("idempotency_key")
            mapping = self._contexts.recall(ic.binding_ref, _DERIVED, {})
            if isinstance(requested, str) and requested in mapping:
                derived = mapping[requested]
                send_args["idempotency_key"] = derived
            elif isinstance(requested, str) and requested in self._own_keys(ic):
                derived = requested
            else:  # never read another invocation's (or tenant's) write receipt
                return self._result(_denied("commitment_mismatch"))
        try:
            result: ToolResult = self._inner.execute(tool, send_args, bound_params, ctx, derived if definition.is_write else None)
        except Exception:  # noqa: BLE001
            status = ToolStatus.uncertain if definition.is_write else ToolStatus.error
            return self._result((status, None, "pulso:executor_exception"))
        return self._shape(tool, result, derived, ic)

    def _shape(self, tool: EntityRef, result: ToolResult, derived: str | None, ic: InvocationContext) -> ToolResult:
        value = result.result_full
        if result.status is ToolStatus.ok and tool.id == "registry/create_proposal" and isinstance(value, dict):
            pid = value.get("proposal_id")
            if isinstance(pid, str):
                self._contexts.remember(ic.binding_ref, _CREATED, pid)
        if result.status is ToolStatus.ok and derived and isinstance(value, dict):
            value = {**value, "key_digest": _digest(derived)}
            return result.model_copy(update={"result_full": value})
        return result
