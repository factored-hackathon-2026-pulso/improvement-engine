"""Annex D DTOs the engine stand-in sends: `CoreTaskInvocation`, evaluation admission, arm request; plus the
idempotency key and request digest rules (independent re-implementation: the E2E cross-checks core-bridge)."""

from __future__ import annotations

import hashlib
from datetime import UTC, datetime, timedelta
from typing import Any

import rfc8785

DIGEST_EXCLUDED = ("request_digest", "credentials", "trace")


def idempotency_key(tenant: str, job: str, stage: str, attempt: int, logical_key: str) -> str:
    return hashlib.sha256(f"{tenant}|{job}|{stage}|{attempt}|{logical_key}".encode()).hexdigest()


def request_digest(body: dict[str, Any]) -> str:
    return hashlib.sha256(rfc8785.dumps({k: v for k, v in body.items() if k not in DIGEST_EXCLUDED})).hexdigest()


def digest_json(value: Any) -> str:
    return hashlib.sha256(rfc8785.dumps(value)).hexdigest()


def invocation(*, tenant: str, job: str, stage: str, agent_id: str, release_id: str, logical: str,
               input: dict[str, Any] | None = None, attempt: int = 1, **extra: Any) -> tuple[str, dict[str, Any]]:
    """Returns (Idempotency-Key, body)."""
    body: dict[str, Any] = {
        "schema_version": "1", "tenant_id": tenant, "pulso_run_ref": f"pr-{tenant}", "job_id": job, "stage": stage,
        "attempt": attempt, "agent_id": agent_id, "agent_version": "1.0.0", "release_id": release_id,
        "input": input or {}, "input_artifact_refs": [], "lab_grant_ref": f"grant-{tenant}-{job}",
        "logical_key": logical}
    body.update(extra)
    return idempotency_key(tenant, job, stage, attempt, logical), body


def admission(*, ref: str, binding_ref: str, proposal_id: str, candidate_hash: str, suite_id: str, suite_version: str,
              suite_digest: str, budget_ref: str, attempt: int = 1, hours: int = 1) -> dict[str, Any]:
    body: dict[str, Any] = {
        "schema_version": "1", "evaluation_context_ref": ref, "binding_ref": binding_ref, "proposal_id": proposal_id,
        "candidate_hash": candidate_hash, "suite_id": suite_id, "suite_version": suite_version,
        "suite_digest": suite_digest, "evaluation_attempt": attempt, "budget_ref": budget_ref,
        "deadline": (datetime.now(UTC) + timedelta(hours=hours)).isoformat()}
    body["request_digest"] = digest_json({k: v for k, v in body.items() if k != "deadline"})
    return body
