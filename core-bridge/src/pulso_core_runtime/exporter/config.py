"""Exporter configuration and shared constants (plan 17.3.6, 16.13.3, 16.15.3)."""

from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass

PIN_SHA = "894fa65575d83420523f33ec1c6919b8965f7ebe"
PIN_CONTRACT_VERSION = "1.3.0"
CONTRACT = "pulso-observations-2"
OBSERVATIONS_PATH = "/internal/v1/platform/observations"
ARTIFACTS_PATH = "/internal/v1/broker/artifacts"
EVAL_MISCONFIGURED = "pulso:eval_db_misconfigured"
OVERPRIVILEGED = "pulso:exporter_overprivileged"
SCHEMA_DRIFT = "pulso:schema_drift"

KIB = 1024
MAX_BATCH_EVENTS = 500
MAX_BATCH_BYTES = 512 * KIB
MAX_ARTIFACT_BYTES = 1024 * KIB
MAX_MATERIAL_BYTES = 64 * 1024 * KIB


@dataclass(frozen=True)
class ExporterConfig:
    tenant_id: str
    instance: str  # "core-<instance>"; sources are "<instance>.audit|registry|outbox"
    expected_runtime_db: str
    expected_eval_db: str
    binding_ref: str
    # ArtifactRef {id, digest, media_type} (annex D). None = bootstrap the pinned schema artifact through the
    # artifact route (artifact_kind=schema, source_schema_ref=null) and use the returned ref.
    source_schema_ref: dict[str, str] | None = None
    registry_schema_ref: dict[str, str] | None = None
    verifier_sha: str = PIN_SHA
    verifier_contract_version: str = PIN_CONTRACT_VERSION
    batch_max_events: int = MAX_BATCH_EVENTS
    batch_max_bytes: int = MAX_BATCH_BYTES
    artifact_max_bytes: int = MAX_ARTIFACT_BYTES
    max_material_bytes: int = MAX_MATERIAL_BYTES
    gap_grace_seconds: float = 120.0
    receipt_every: int = 200
    max_retries: int = 5
    backoff_cap_seconds: float = 60.0
    # A03: every HTTP attempt mints a fresh token for its route class ("observations" | "artifacts" | "cursor").
    token_for: Callable[[str], str] | None = None
    token_provider: Callable[[], str] | None = None  # route-agnostic legacy hook (tests/fixtures without A03 auth)

    def token(self, route: str) -> str | None:
        if self.token_for is not None:
            return self.token_for(route)
        return self.token_provider() if self.token_provider is not None else None

    def source_id(self, kind: str) -> str:
        return f"{self.instance}.{kind}"
