"""`CoreTaskInvocation` (CAP-25) and digest helpers."""

from __future__ import annotations

import hashlib
from typing import Any

from agent_core.domain import canonical_bytes
from pydantic import BaseModel, ConfigDict, Field

STAGES = ("scout", "verifier", "builder_design", "writer")
MAX_INPUT_BYTES = 256 * 1024
DIGEST_EXCLUDED = ("request_digest", "credentials", "trace")


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
    # accepted for convenience, excluded from the digest
    request_digest: str | None = None
    credentials: dict[str, Any] | None = None
    trace: dict[str, Any] | None = None


def request_digest(raw_body: dict[str, Any]) -> str:
    """`sha256(JCS(body without request_digest, credentials, trace))`."""
    stripped = {k: v for k, v in raw_body.items() if k not in DIGEST_EXCLUDED}
    return hashlib.sha256(canonical_bytes(stripped)).hexdigest()


def idempotency_key_for(tenant: str, job: str, stage: str, attempt: int, logical_key: str) -> str:
    return hashlib.sha256(f"{tenant}|{job}|{stage}|{attempt}|{logical_key}".encode()).hexdigest()


def sha256_text(text: str) -> str:
    return hashlib.sha256(text.encode()).hexdigest()
