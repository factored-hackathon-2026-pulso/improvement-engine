"""`POST core-tasks/invoke` (plan 17.3.3, the eleven steps) and `GET core-tasks/{id}`.

Never a direct `TurnEngine` call: Core is reached through `CoreRuns` (in-process ASGI). Any failure after the
`sent` commit is `unknown`, never `failed`."""

from __future__ import annotations

import asyncio
import hashlib
from collections.abc import Mapping
from dataclasses import dataclass, field
from datetime import UTC, datetime, timedelta
from typing import Any

from agent_core.domain import canonical_bytes
from pydantic import ValidationError

from pulso_core_runtime.credentials.issuer import PrincipalSigner, bot_principal
from pulso_core_runtime.invoke import errors
from pulso_core_runtime.invoke.context import InvocationContext, InvocationRegistry, use_binding
from pulso_core_runtime.invoke.core_client import CoreResponse, CoreRuns
from pulso_core_runtime.invoke.errors import BridgeError
from pulso_core_runtime.invoke.models import (
    MAX_INPUT_BYTES,
    STAGES,
    CoreTaskInvocation,
    idempotency_key_for,
    request_digest,
    sha256_text,
)
from pulso_core_runtime.invoke.pin import RELEASE_DRIFT, ReleaseChecker
from pulso_core_runtime.invoke.projection import FactProjector
from pulso_core_runtime.reconcile.reconciler import Reconciler, RunReader
from pulso_core_runtime.store.receipts import Receipt, ReceiptStore

NON_TERMINAL_REENTRY = frozenset({"sent", "binding_confirmed", "unknown", "manual_reconcile"})


@dataclass
class InvokeSettings:
    max_inflight: int = 8
    bridge_instance_id: str = "bridge-1"
    principal_ttl: timedelta = timedelta(minutes=15)
    context_ttl: timedelta = timedelta(minutes=20)
    prepared_stale: timedelta = timedelta(seconds=30)
    # Stage -> declared input slots (L3b `stages/catalog.py`). None = not enforced yet.
    stage_slots: Mapping[str, frozenset[str]] | None = None
    # Stage -> roles on the run principal. Only the writer carries `constructor` (registry role check).
    stage_roles: Mapping[str, tuple[str, ...]] = field(default_factory=lambda: {
        "writer": ("constructor",), "scout": ("stage_task",), "verifier": ("stage_task",),
        "builder_design": ("stage_task",)})


@dataclass(frozen=True)
class InvokeOutcome:
    status: int
    body: dict[str, Any]


def _failed_state(inv: CoreTaskInvocation) -> str:
    """After `sent`, a failed writer may already have changed the registry: it is never provably `terminal_failed`."""
    return "manual_reconcile" if inv.stage == "writer" else "terminal_failed"


def _commitment(dto: Any) -> Any:
    if dto is None:
        return None
    from pulso_core_runtime.tools.context import RegistryMutationCommitment

    return RegistryMutationCommitment(
        mode=dto.mode, proposal_id=dto.proposal_id, expected_rev=dto.expected_rev,
        base_release_id=dto.base_release_id, evaluate_enabled=dto.evaluate_enabled,
        evaluation_context_ref=dto.evaluation_context_ref, create_agent_id=dto.create_agent_id,
        create_origin=dto.create_origin, create_title=dto.create_title, put_draft_digest=dto.put_draft_digest,
        operations=tuple(dto.operations))


def _commitment_json(c: Any) -> dict[str, Any] | None:
    if c is None:
        return None
    return {"mode": c.mode, "proposal_id": c.proposal_id, "expected_rev": c.expected_rev,
            "base_release_id": c.base_release_id, "evaluate_enabled": c.evaluate_enabled,
            "evaluation_context_ref": c.evaluation_context_ref, "create_agent_id": c.create_agent_id,
            "create_origin": c.create_origin, "create_title": c.create_title,
            "put_draft_digest": c.put_draft_digest, "operations": list(c.operations)}


def error_body(exc: BridgeError, trace_id: str = "") -> dict[str, Any]:
    return {"schema_version": "1", "code": exc.code, "retryable": exc.retryable, "trace_id": trace_id,
            "details": exc.details}


def _state_body(r: Receipt) -> dict[str, Any]:
    stored = r.receipt or {}
    body: dict[str, Any] = {"schema_version": "1", "state": r.state, "core_run_id": r.core_run_id,
                            "reason": r.reason, "outcome": r.outcome, "task_binding_ref": r.task_binding_ref}
    for k in ("receipt", "result", "proven_no_effect", "adopted_writes"):
        if k in stored:
            body[k] = stored[k]
    return body


def _status_for(r: Receipt) -> int:
    return 200 if r.terminal else 202


class InvokeService:
    def __init__(self, *, store: ReceiptStore, core: CoreRuns, releases: ReleaseChecker, signer: PrincipalSigner,
                 runs: RunReader, registry: InvocationRegistry, settings: InvokeSettings | None = None,
                 projector: FactProjector | None = None, reconciler: Reconciler | None = None,
                 now: Any = lambda: datetime.now(UTC)) -> None:
        self._store, self._core, self._releases, self._signer = store, core, releases, signer
        self._runs, self._registry = runs, registry
        self._settings = settings or InvokeSettings()
        self._projector = projector
        self._reconciler = reconciler or Reconciler(store=store, runs=runs, projector=projector)
        self._now = now
        self._inflight = 0
        self._live: set[tuple[str, str]] = set()  # keys whose Core call is in flight in this process

    # ---- public ----------------------------------------------------------------------------------------

    async def invoke(self, tenant_claim: str | None, idem_key: str | None, raw: dict[str, Any]) -> InvokeOutcome:
        try:
            return await self._invoke(tenant_claim, idem_key, raw)
        except BridgeError as exc:
            return InvokeOutcome(exc.status, error_body(exc))

    async def read(self, tenant_claim: str | None, task_id: str) -> InvokeOutcome:
        """Read-only: never starts or mutates anything. A foreign tenant's id is 404."""
        try:
            if not tenant_claim:
                raise BridgeError("pulso:not_found", 404)
            receipt = await asyncio.to_thread(self._store.get_by_run, tenant_claim, task_id)
            if receipt is None:
                receipt = await asyncio.to_thread(self._store.get, tenant_claim, task_id)
            if receipt is None:
                raise BridgeError("pulso:not_found", 404)
            body = _state_body(receipt)
            if receipt.state == "terminal_ok" and "result" in (receipt.receipt or {}):
                return InvokeOutcome(200, body)
            if not receipt.terminal:
                body["code"] = "pulso:task_unknown" if receipt.state == "unknown" else "pulso:task_in_progress"
            return InvokeOutcome(200, body)
        except BridgeError as exc:
            return InvokeOutcome(exc.status, error_body(exc))

    # ---- the eleven steps ------------------------------------------------------------------------------

    async def _invoke(self, tenant_claim: str | None, idem_key: str | None, raw: dict[str, Any]) -> InvokeOutcome:
        # 1-2. authenticate happened in the route; tenant claim, body, digest, stage, input, slots
        try:
            inv = CoreTaskInvocation.model_validate(raw)
        except ValidationError as exc:
            fields = sorted({".".join(str(p) for p in e["loc"]) for e in exc.errors()})
            raise BridgeError("pulso:invalid_request", 422, details={"fields": fields}) from None
        if tenant_claim is None or inv.tenant_id != tenant_claim:
            raise BridgeError("pulso:tenant_mismatch", 403)
        digest = request_digest(raw)
        if inv.request_digest is not None and inv.request_digest != digest:
            raise BridgeError("pulso:invalid_request", 422, details={"fields": ["request_digest"]})
        key = idempotency_key_for(inv.tenant_id, inv.job_id, inv.stage, inv.attempt, inv.logical_key)
        if not idem_key or idem_key != key:
            raise BridgeError("pulso:invalid_request", 422, details={"fields": ["Idempotency-Key"]})
        if inv.stage not in STAGES:
            raise errors.stage_unknown(inv.stage)
        if len(canonical_bytes({"input": inv.input, "refs": inv.input_artifact_refs})) > MAX_INPUT_BYTES:
            raise errors.input_too_large()
        declared = (self._settings.stage_slots or {}).get(inv.stage)
        if declared is not None and (extra := set(inv.input) - declared):
            raise errors.unknown_input_slot(sorted(extra))

        # 3. single-flight on (tenant, key)
        ref = sha256_text(f"{inv.tenant_id}|{key}")
        role_label = "writer" if inv.stage == "writer" else "task"
        principal_id = f"pulso-bot:{inv.tenant_id}:{role_label}:{inv.stage}"
        receipt, created = await asyncio.to_thread(
            self._store.begin, tenant_id=inv.tenant_id, key=key, digest=digest, stage=inv.stage, job_id=inv.job_id,
            attempt=inv.attempt, release_id=inv.release_id, task_binding_ref=ref, principal_id=principal_id)
        if not created:
            if receipt.request_digest != digest:
                raise errors.digest_conflict()
            return await self._reenter(receipt)

        # 4. pre-pin checks
        code = await asyncio.to_thread(self._releases.check, inv.release_id, inv.agent_id, inv.agent_version,
                                       inv.closure_digest)
        if code is not None:
            moved = await asyncio.to_thread(self._store.transition, inv.tenant_id, key, "terminal_failed",
                                            reason=code.removeprefix("pulso:"))
            raise errors.release_error(code) if moved is not None else BridgeError(code, 409)

        # capacity: nothing was sent yet, so a busy bridge leaves no row behind (retry with the same key is clean)
        if self._inflight >= self._settings.max_inflight:
            await asyncio.to_thread(self._store.discard_prepared, inv.tenant_id, key)
            raise errors.bridge_busy()
        self._inflight += 1
        self._live.add((inv.tenant_id, key))
        try:
            return await self._execute(inv, key, digest, ref, principal_id)
        finally:
            self._inflight -= 1
            self._live.discard((inv.tenant_id, key))

    async def _reenter(self, receipt: Receipt) -> InvokeOutcome:
        if receipt.terminal:
            return InvokeOutcome(200, _state_body(receipt))
        if receipt.state == "prepared":
            stale = receipt.updated_at is not None and self._now() - receipt.updated_at > self._settings.prepared_stale
            if not stale:  # a live invocation owns it (or is about to send): never race it
                return InvokeOutcome(202, _state_body(receipt))
            await asyncio.to_thread(self._reconciler.reconcile, receipt)  # crashed before `sent`: nothing was sent
            fresh = await asyncio.to_thread(self._store.get, receipt.tenant_id, receipt.idempotency_key)
            return InvokeOutcome(_status_for(fresh or receipt), _state_body(fresh or receipt))
        if (receipt.tenant_id, receipt.idempotency_key) in self._live:
            return InvokeOutcome(202, _state_body(receipt))  # the original call is alive: never demote it
        # sent / binding_confirmed / unknown / manual_reconcile: re-read, never re-execute
        result = await asyncio.to_thread(self._reconciler.reconcile, receipt)
        fresh = await asyncio.to_thread(self._store.get, receipt.tenant_id, receipt.idempotency_key)
        out = fresh or receipt
        body = _state_body(out)
        body.setdefault("reason", result.reason)
        if result.proven_no_effect:
            body["proven_no_effect"] = True
        return InvokeOutcome(_status_for(out), body)

    async def _execute(self, inv: CoreTaskInvocation, key: str, digest: str, ref: str,
                       principal_id: str) -> InvokeOutcome:
        tenant = inv.tenant_id
        now = self._now()
        exp = now + self._settings.principal_ttl
        roles = list(self._settings.stage_roles.get(inv.stage, ()))
        attrs = {"tenant": tenant, "job": inv.job_id, "stage": inv.stage, "attempt": str(inv.attempt),
                 "grant_ref": inv.lab_grant_ref, "task_binding_ref": ref, "pin_release_id": inv.release_id}
        # 5. run principal, signed with the bridge principal key
        bearer = self._signer.sign(bot_principal(principal_id=principal_id, roles=roles, attrs=attrs, now=now,
                                                 exp=exp))
        # 6. frozen context registered, then `sent` committed BEFORE the call
        ctx = InvocationContext(
            tenant_id=tenant, job_id=inv.job_id, stage=inv.stage, attempt=inv.attempt, binding_ref=ref,
            command_key=key, request_digest=digest, bridge_instance_id=self._settings.bridge_instance_id,
            expires_at=now + self._settings.context_ttl,
            memory_snapshot_ref=inv.memory_snapshot_ref, extract_manifest_ref=inv.extract_manifest_ref,
            commitment=_commitment(inv.registry_mutation_commitment), inputs=dict(inv.input))
        try:
            self._registry.register(ctx)
        except ValueError:  # same key re-entering after a lost CAS: keep the first frozen context
            pass
        extra = {"grant_ref": inv.lab_grant_ref, "pin_release_id": inv.release_id, "budget": inv.budget,
                 "cutoff": inv.cutoff, "deadline": inv.deadline, "expires_at": ctx.expires_at.isoformat()}
        await asyncio.to_thread(self._store.save_context, ref, tenant, inv.job_id, key,
                                {"tenant_id": tenant, "job_id": inv.job_id, "stage": inv.stage,
                                 "attempt": inv.attempt, **extra,
                                 "operations": list(ctx.commitment.operations) if ctx.commitment else [],
                                 "evaluation_context_ref": ctx.evaluation_context_ref,
                                 # the sealed commitment, so a crashed writer's adopted writes can be verified
                                 "commitment": _commitment_json(ctx.commitment)}, ctx.expires_at)
        sent = await asyncio.to_thread(self._store.transition, tenant, key, "sent")
        if sent is None:  # lost the CAS: someone else owns this key
            self._registry.remove(ref)
            current = await asyncio.to_thread(self._store.get, tenant, key)
            assert current is not None
            return await self._reenter(current)

        try:
            with use_binding(ref):
                # 7. Core through the full M9 route
                body: dict[str, Any] = {"agent": f"{inv.agent_id}@{inv.agent_version}", "subject": None,
                                        "input": inv.input}
                if inv.lang:
                    body["lang"] = inv.lang
                try:
                    resp = await self._core.start_run(bearer, key, body)
                except Exception:  # 11. timeout / disconnect / anything after `sent` -> unknown
                    return await self._mark_unknown(tenant, key, "core_call_failed")
                return await self._after_response(inv, key, ref, resp)
        except BridgeError:
            raise
        except Exception:
            return await self._mark_unknown(tenant, key, "post_send_exception")

    async def _mark_unknown(self, tenant: str, key: str, reason: str) -> InvokeOutcome:
        moved = await asyncio.to_thread(self._store.transition, tenant, key, "unknown", reason=reason)
        current = moved or await asyncio.to_thread(self._store.get, tenant, key)
        assert current is not None
        return InvokeOutcome(_status_for(current), _state_body(current))

    async def _terminal(self, inv: CoreTaskInvocation, key: str, ref: str, state: str, reason: str, *,
                        run_id: str | None = None, outcome: str | None = None,
                        receipt: dict[str, Any] | None = None, http: int | None = None) -> InvokeOutcome:
        moved = await asyncio.to_thread(self._store.transition, inv.tenant_id, key, state, reason=reason,
                                        core_run_id=run_id, outcome=outcome, receipt=receipt)
        current = moved or await asyncio.to_thread(self._store.get, inv.tenant_id, key)
        assert current is not None
        if current.terminal:
            self._registry.remove(ref)
            await asyncio.to_thread(self._store.delete_context, ref)
        out = InvokeOutcome(http or _status_for(current), _state_body(current))
        if http and http >= 400:
            out.body.setdefault("code", f"pulso:{reason}")
        return out

    async def _after_response(self, inv: CoreTaskInvocation, key: str, ref: str, resp: CoreResponse) -> InvokeOutcome:
        status = resp.status
        if status >= 500:
            return await self._mark_unknown(inv.tenant_id, key, f"core_http_{status}")
        if status == 409:  # Core saw this (principal, key) with another body: we cannot tell who ran what
            moved = await asyncio.to_thread(self._store.transition, inv.tenant_id, key, "manual_reconcile",
                                            reason="core_idempotency_conflict")
            current = moved or await asyncio.to_thread(self._store.get, inv.tenant_id, key)
            assert current is not None
            return InvokeOutcome(409, {**_state_body(current), "code": "pulso:digest_conflict"})
        if status != 201:
            http = 503 if status == 429 else 403 if status in (401, 403) else 409 if status == 404 else 422
            return await self._terminal(inv, key, ref, _failed_state(inv), f"core_rejected_{status}", http=http)
        data = resp.body
        run_id = str(data.get("run_id", ""))
        # 9. release drift: the result is discarded
        if data.get("release") != inv.release_id:
            return await self._terminal(inv, key, ref, _failed_state(inv), RELEASE_DRIFT.removeprefix("pulso:"),
                                        run_id=run_id or None, http=409)
        outcome = str(data.get("outcome"))
        if outcome not in ("completed", "failed"):
            # Core committed a terminal run that is neither completed nor failed (e.g. escalated): known and not
            # a success, so the stage fails closed with the Core outcome recorded; no facts are promoted.
            return await self._terminal(inv, key, ref, _failed_state(inv), "unexpected_outcome", run_id=run_id,
                                        outcome=outcome, receipt={"core_outcome": outcome,
                                                                  "core_status": data.get("status")})
        current = await asyncio.to_thread(self._store.get, inv.tenant_id, key)
        if current is not None and current.state == "manual_reconcile":
            return InvokeOutcome(202, _state_body(current))  # binding unproven: stays for reconcile
        # 10. ReadRunResult with the whitelist, persist the receipt, drop the context
        envelope: dict[str, Any] | None = None
        if self._projector is not None:
            try:
                run = await asyncio.to_thread(self._runs.load_run, run_id)
                envelope = await asyncio.to_thread(
                    self._projector.project, inv.stage, run_id, run, status=str(data.get("status")), outcome=outcome,
                    binding_ref=ref)
            except BridgeError as exc:
                return await self._terminal(inv, key, ref, "terminal_failed", exc.code.removeprefix("pulso:"),
                                            run_id=run_id, outcome=outcome, http=exc.status)
            except Exception:
                return await self._mark_unknown(inv.tenant_id, key, "projection_failed")
        meter = await asyncio.to_thread(self._store.meter_get, inv.tenant_id, inv.job_id, inv.stage, inv.attempt)
        receipt = {
            "schema_version": "1", "core_run_id": run_id, "release_id": inv.release_id, "outcome": outcome,
            "trace_id": data.get("trace_id"), "idempotency_key_digest": sha256_text(key),
            "request_digest": current.request_digest if current else "", "task_binding_ref": ref,
            "input_commitment": hashlib.sha256(canonical_bytes(
                {"input": inv.input, "refs": inv.input_artifact_refs})).hexdigest(),
            "output_refs": [], "output_digest": (envelope or {}).get("output_digest"), "audit_refs": [],
            "budget": {"known": bool(meter and meter["usage_known"] and meter["calls"] > 0)}}
        if outcome == "completed" and (current is None or current.state != "binding_confirmed"):
            # A completed run whose binding was never confirmed cannot be trusted as ok: reconcile by hand.
            return await self._terminal(inv, key, ref, "manual_reconcile", "binding_unconfirmed", run_id=run_id,
                                        outcome=outcome, http=202)
        state = "terminal_ok" if outcome == "completed" else _failed_state(inv)
        return await self._terminal(inv, key, ref, state, "completed" if state == "terminal_ok" else "run_failed",
                                    run_id=run_id, outcome=outcome, receipt={"receipt": receipt, "result": envelope})

