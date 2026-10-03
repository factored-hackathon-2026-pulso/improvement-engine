"""`CoreTaskInvocation` (CAP-25) and digest helpers."""

from __future__ import annotations

import hashlib
from typing import Any, Literal

from agent_core.domain import canonical_bytes
from pydantic import BaseModel, ConfigDict, Field, model_validator

STAGES = ("scout", "verifier", "builder_design", "writer")
MAX_INPUT_BYTES = 256 * 1024
DIGEST_EXCLUDED = ("request_digest", "credentials", "trace")


class RegistryMutationCommitmentDTO(BaseModel):
    """Codex-sealed writer commitment (D.2 writer row, CL-0002/CX-0007, CLQ-10). Not a separate annex-D field:
    it travels inside the digested `CoreTaskInvocation` body, so the same-key/other-body check also covers it.
    `operations` is the ordered list of committed non-evaluate writes; a write's key ordinal is its index."""

    model_config = ConfigDict(extra="forbid")

    mode: Literal["write", "evaluate_only"]
    proposal_id: str | None = Field(default=None, max_length=200)
    expected_rev: int | None = Field(default=None, ge=0)
    base_release_id: str | None = Field(default=None, max_length=200)
    evaluate_enabled: bool = False
    evaluation_context_ref: str | None = Field(default=None, max_length=200)
    create_agent_id: str | None = Field(default=None, max_length=128)
    create_origin: str | None = Field(default=None, max_length=32)
    create_title: str | None = Field(default=None, max_length=512)
    put_draft_digest: str | None = Field(default=None, pattern="^[0-9a-f]{64}$")
    operations: list[Literal["create_proposal", "put_draft", "validate", "freeze", "reopen"]] = Field(
        default_factory=list, max_length=32)


class CoreTaskInvocation(BaseModel):
    model_config = ConfigDict(extra="forbid")

    schema_version: str = Field(pattern="^1$")
    tenant_id: str = Field(min_length=1, max_length=128)
    pulso_run_ref: str = Field(min_length=1, max_length=256)
    job_id: str = Field(min_length=1, max_length=128)
    stage: str
    attempt: int = Field(ge=1, le=1000)
    agent_id: str = Field(min_length=1, max_length=128)
    agent_version: str = Field(pattern=r"^\d+\.\d+\.\d+$")
    release_id: str = Field(min_length=1, max_length=128)
    input: dict[str, Any] = Field(default_factory=dict)
    input_artifact_refs: list[str] = Field(default_factory=list, max_length=64)
    lab_grant_ref: str = Field(min_length=1, max_length=256)
    budget: dict[str, Any] | None = None
    cutoff: str | None = None
    deadline: str | None = None
    logical_key: str = Field(min_length=1, max_length=256)
    lang: str | None = None
    closure_digest: str | None = Field(default=None, pattern="^[0-9a-f]{64}$")
    memory_snapshot_ref: str | None = Field(default=None, max_length=256)  # D.2 context refs, sealed in the digest
    extract_manifest_ref: str | None = Field(default=None, max_length=256)
    registry_mutation_commitment: RegistryMutationCommitmentDTO | None = None  # writer stage only
    # accepted for convenience, excluded from the digest
    request_digest: str | None = None
    credentials: dict[str, Any] | None = None
    trace: dict[str, Any] | None = None

    @model_validator(mode="after")
    def _commitment_is_writer_only(self) -> CoreTaskInvocation:
        if self.registry_mutation_commitment is not None and self.stage != "writer":
            raise ValueError("registry_mutation_commitment is only valid for the writer stage")
        return self


def request_digest(raw_body: dict[str, Any]) -> str:
    """`sha256(JCS(body without request_digest, credentials, trace))`."""
    stripped = {k: v for k, v in raw_body.items() if k not in DIGEST_EXCLUDED}
    return hashlib.sha256(canonical_bytes(stripped)).hexdigest()


def idempotency_key_for(tenant: str, job: str, stage: str, attempt: int, logical_key: str) -> str:
    return hashlib.sha256(f"{tenant}|{job}|{stage}|{attempt}|{logical_key}".encode()).hexdigest()


def sha256_text(text: str) -> str:
    return hashlib.sha256(text.encode()).hexdigest()
