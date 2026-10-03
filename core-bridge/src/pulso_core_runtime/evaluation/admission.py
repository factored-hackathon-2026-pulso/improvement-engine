"""Evaluation admissions (plan 17.3.5, CLQ-09): one admission = one native evaluation.

States: `admitted` -> `consumed` (CAS before any effect) ; `expired` ; `unknown` (consumed but no stored result:
uncertain prior execution, blocks any new run; a retry needs a new admission with `evaluation_attempt+1`).
`AdmissionGate` is the single function used by the in-process Flow path and by the HTTP-campaign wrapper."""

from __future__ import annotations

import json
import re
import threading
from dataclasses import asdict, dataclass, replace
from datetime import UTC, datetime
from typing import Any, Literal, Protocol

import psycopg

from pulso_core_runtime.evaluation.report import ensure_eval_schema

CONTEXT_REF_RE = re.compile(r"^[A-Za-z0-9_.:-]{1,200}$")
EVAL_KEY_PREFIX = "pulso-eval:"
AdmissionState = Literal["admitted", "consumed", "expired", "unknown"]


def valid_context_ref(ref: object) -> bool:
    """<= 200 ASCII chars from [A-Za-z0-9_.:-]; never truncated or normalised (`fullmatch`, no trailing \\n)."""
    return isinstance(ref, str) and CONTEXT_REF_RE.fullmatch(ref) is not None and ref.isascii()


def eval_key(ref: str) -> str:
    if not valid_context_ref(ref):
        raise AdmissionDenied("evaluation_context_invalid", 422)
    return EVAL_KEY_PREFIX + ref


class AdmissionDenied(Exception):
    """`code` is the closed, safe error code; `status` the HTTP status; raised before any harness work."""

    def __init__(self, code: str, status: int = 409) -> None:
        super().__init__(code)
        self.code, self.status = code, status


@dataclass(frozen=True)
class Admission:
    evaluation_context_ref: str
    tenant_id: str
    job_id: str
    binding_ref: str
    proposal_id: str
    candidate_hash: str
    suite_id: str
    suite_version: str
    suite_digest: str
    evaluation_attempt: int
    budget_ref: str
    deadline: datetime
    request_digest: str
    state: AdmissionState = "admitted"

    def to_json(self) -> dict[str, Any]:
        d = asdict(self)
        d["deadline"] = self.deadline.isoformat()
        return d

    @staticmethod
    def from_json(d: dict[str, Any]) -> Admission:
        return Admission(**{**d, "deadline": datetime.fromisoformat(d["deadline"])})


@dataclass(frozen=True)
class InvocationContext:
    """Trusted executor context of one Flow invocation (never tool args, never the model)."""

    tenant_id: str
    job_id: str
    binding_ref: str
    binding_confirmed: bool
    evaluate_enabled: bool
    evaluation_context_ref: str | None
    evaluation_attempt: int | None = None


class AdmissionStore(Protocol):
    def create(self, admission: Admission) -> tuple[Admission, bool]: ...

    def get(self, ref: str) -> Admission | None: ...

    def transition(self, ref: str, expected: AdmissionState, new: AdmissionState) -> bool:
        """Atomic compare-and-set of the state."""
        ...


class InMemoryAdmissionStore:
    def __init__(self) -> None:
        self._rows: dict[str, Admission] = {}
        self._lock = threading.Lock()

    def create(self, admission: Admission) -> tuple[Admission, bool]:
        with self._lock:
            if admission.evaluation_context_ref in self._rows:
                return self._rows[admission.evaluation_context_ref], False
            for a in self._rows.values():
                if (a.tenant_id, a.proposal_id, a.candidate_hash, a.evaluation_attempt) == (
                        admission.tenant_id, admission.proposal_id, admission.candidate_hash,
                        admission.evaluation_attempt):
                    raise AdmissionDenied("evaluation_attempt_exists", 409)
            self._rows[admission.evaluation_context_ref] = admission
            return admission, True

    def get(self, ref: str) -> Admission | None:
        with self._lock:
            return self._rows.get(ref)

    def transition(self, ref: str, expected: AdmissionState, new: AdmissionState) -> bool:
        with self._lock:
            row = self._rows.get(ref)
            if row is None or row.state != expected:
                return False
            self._rows[ref] = replace(row, state=new)
            return True


class PgAdmissionStore:
    def __init__(self, dsn: str) -> None:
        self._dsn = dsn
        ensure_eval_schema(dsn)

    def create(self, admission: Admission) -> tuple[Admission, bool]:
        try:
            with psycopg.connect(self._dsn, autocommit=True) as conn:
                cur = conn.execute(
                    "INSERT INTO pulso_bridge.eval_admissions (evaluation_context_ref, tenant_id, proposal_id, "
                    "candidate_hash, evaluation_attempt, state, request_digest, record) "
                    "VALUES (%s,%s,%s,%s,%s,%s,%s,%s::jsonb) ON CONFLICT (evaluation_context_ref) DO NOTHING",
                    (admission.evaluation_context_ref, admission.tenant_id, admission.proposal_id,
                     admission.candidate_hash, admission.evaluation_attempt, admission.state,
                     admission.request_digest, json.dumps(admission.to_json())))
                created = cur.rowcount == 1
        except psycopg.errors.UniqueViolation:
            raise AdmissionDenied("evaluation_attempt_exists", 409) from None
        row = self.get(admission.evaluation_context_ref)
        assert row is not None
        return row, created

    def get(self, ref: str) -> Admission | None:
        with psycopg.connect(self._dsn, autocommit=True) as conn:
            r = conn.execute("SELECT record, state FROM pulso_bridge.eval_admissions "
                             "WHERE evaluation_context_ref=%s", (ref,)).fetchone()
        return None if r is None else replace(Admission.from_json(r[0]), state=r[1])

    def transition(self, ref: str, expected: AdmissionState, new: AdmissionState) -> bool:
        with psycopg.connect(self._dsn, autocommit=True) as conn:
            cur = conn.execute("UPDATE pulso_bridge.eval_admissions SET state=%s "
                               "WHERE evaluation_context_ref=%s AND state=%s", (new, ref, expected))
            return cur.rowcount == 1


class BrokerPort(Protocol):
    def check(self, *, tenant_id: str, binding_ref: str, scope: str, payload_digest: str) -> bool:
        """`POST /internal/v1/broker/authorizations/check`; False/raise = deny (fail closed)."""
        ...


@dataclass(frozen=True)
class ProposalView:
    """What the gate needs from a fresh `RegistryService.get_proposal`."""

    proposal_id: str
    state: str
    candidate_hash: str | None
    suite_digest: str | None  # content hash of the requested suite version in the registry/candidate


class AdmissionGate:
    def __init__(self, store: AdmissionStore, broker: BrokerPort, *,
                 now: Any = lambda: datetime.now(UTC)) -> None:
        self._store, self._broker, self._now = store, broker, now

    def begin(self, ctx: InvocationContext, proposal_id: str, fresh: ProposalView, *, replay_exists: bool,
              suite_id: str, suite_version: str, native_payload_digest: str) -> Admission:
        """Checks (i)-(vii); the caller does (viii) with `admission.budget_ref`. Zero spend on any denial.

        `replay_exists`: the service already holds a write for `pulso-eval:<ref>`; the same admission then
        replays the stored result (also after expiry/consumption) without a run and without CAS."""
        if not ctx.binding_confirmed:
            raise AdmissionDenied("binding_not_confirmed", 403)
        if not ctx.evaluate_enabled:
            raise AdmissionDenied("evaluate_disabled", 403)
        ref = ctx.evaluation_context_ref
        if ref is None or not valid_context_ref(ref):
            raise AdmissionDenied("evaluation_context_invalid", 422)
        adm = self._store.get(ref)
        if adm is None:
            raise AdmissionDenied("admission_missing", 403)
        if (adm.tenant_id, adm.job_id, adm.binding_ref) != (ctx.tenant_id, ctx.job_id, ctx.binding_ref):
            raise AdmissionDenied("admission_cross_tenant", 403)
        if adm.proposal_id != proposal_id:
            raise AdmissionDenied("admission_proposal_mismatch", 403)
        if ctx.evaluation_attempt is not None and ctx.evaluation_attempt != adm.evaluation_attempt:
            raise AdmissionDenied("admission_attempt_mismatch", 403)
        if replay_exists:
            return adm
        if adm.state == "unknown":
            raise AdmissionDenied("evaluation_unknown", 409)
        if adm.state == "consumed":
            # consumed without a stored result: uncertain prior execution (crash or still running)
            self._store.transition(ref, "consumed", "unknown")
            raise AdmissionDenied("evaluation_unknown", 409)
        if adm.state != "admitted":
            raise AdmissionDenied("admission_not_admitted", 409)
        if self._now() >= adm.deadline:
            self._store.transition(ref, "admitted", "expired")
            raise AdmissionDenied("admission_expired", 409)
        if fresh.state != "candidate" or fresh.candidate_hash != adm.candidate_hash:
            raise AdmissionDenied("candidate_changed", 409)
        if (suite_id, suite_version) != (adm.suite_id, adm.suite_version) or fresh.suite_digest != adm.suite_digest:
            raise AdmissionDenied("suite_mismatch", 409)
        try:
            allowed = self._broker.check(tenant_id=adm.tenant_id, binding_ref=adm.binding_ref,
                                         scope="native_evaluate", payload_digest=native_payload_digest)
        except Exception:  # fail closed: broker down is a denial, never a pass
            allowed = False
        if not allowed:
            raise AdmissionDenied("broker_denied", 403)
        if not self._store.transition(ref, "admitted", "consumed"):
            raise AdmissionDenied("evaluation_unknown", 409)  # lost the CAS to a concurrent call
        return replace(adm, state="consumed")

    def mark_unknown(self, ref: str) -> None:
        self._store.transition(ref, "consumed", "unknown")
