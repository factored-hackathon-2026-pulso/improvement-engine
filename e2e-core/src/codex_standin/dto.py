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


def evaluation_context_ref(tenant: str, job: str, binding_ref: str, proposal_id: str, candidate_hash: str,
                           attempt: int) -> str:
    """The ref the bridge derives (contract.json idempotency.admissions.derivation, ADR 0011 item 4); the engine
    copies it into the writer commitment. A unit test evaluates the contract formula and compares."""
    text = f"{tenant}|{job}|{binding_ref}|{proposal_id}|{candidate_hash}|{attempt}"
    return "evc-" + hashlib.sha256(text.encode()).hexdigest()[:40]


def z_timestamp(dt: datetime) -> str:
    """UTC RFC3339 with a literal `Z` (annex D.1)."""
    return dt.astimezone(UTC).strftime("%Y-%m-%dT%H:%M:%SZ")


def admission(*, binding_ref: str, proposal_id: str, candidate_hash: str, suite_id: str, suite_version: str,
              suite_digest: str, budget_ref: str, attempt: int = 1, hours: int = 1) -> dict[str, Any]:
    """Annex D.4 admission request. No `evaluation_context_ref`: the bridge derives it (see `evaluation_context_ref`)."""
    body: dict[str, Any] = {
        "schema_version": "1", "binding_ref": binding_ref, "proposal_id": proposal_id,
        "candidate_hash": candidate_hash, "suite_id": suite_id, "suite_version": suite_version,
        "suite_digest": suite_digest, "evaluation_attempt": attempt, "budget_ref": budget_ref,
        "deadline": z_timestamp(datetime.now(UTC) + timedelta(hours=hours))}
    body["request_digest"] = digest_json({k: v for k, v in body.items() if k != "deadline"})  # lowercase hex
    return body


PROFILES = ("attention_stateful_complementary", "evolution_task")


def arm_request(*, key: str, binding_ref: str, manifest_ref: str, profile: str | None, target: dict[str, Any],
                arm: str = "baseline", repetition: int = 0, seed: int = 7, budget_ref: str = "bud-e2e",
                sandbox_session_ref: str | None = "seed-e2e", hours: int = 1, **extra: Any) -> dict[str, Any]:
    """Annex D.4 ArmRequest (sent with the `Idempotency-Key` header; the body copy is spec-allowed and must be equal).
    `profile=None` is the task-builder bank arm: it has no annex profile, so it rides the deprecated `mode` alias
    (ADR 0011 item 3). `agent_id` is never sent (derived from the target); `evolution_task` carries no session ref."""
    if profile is not None and profile not in PROFILES:
        raise ValueError(f"execution_profile must be one of {PROFILES} (or None for the task_builder alias)")
    body: dict[str, Any] = {
        "schema_version": "1", "idempotency_key": key, "binding_ref": binding_ref, "campaign_ref": "camp-e2e",
        "case_ref": "case-e2e", "arm": arm, "repetition": repetition, "seed": seed, "target": target,
        "scenario_manifest_ref": manifest_ref, "budget_ref": budget_ref,
        "deadline": z_timestamp(datetime.now(UTC) + timedelta(hours=hours))}
    if profile is None:
        body["mode"] = "task_builder"
    else:
        body["execution_profile"] = profile
    if profile != "evolution_task":
        body["sandbox_session_ref"] = sandbox_session_ref
    body.update(extra)
    return body
