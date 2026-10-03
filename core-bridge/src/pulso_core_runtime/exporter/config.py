"""Exporter configuration and shared constants (plan 17.3.6, 16.13.3, 16.15.3)."""

from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass

PIN_SHA = "86a767474042a566a0dbd6ed23588959f27ebdb3"
PIN_CONTRACT_VERSION = "1.3.0"
CONTRACT = "pulso-observations-2"
OBSERVATIONS_PATH = "/internal/v1/platform/observations"
ARTIFACTS_PATH = "/internal/v1/broker/artifacts"
EVAL_MISCONFIGURED = "pulso:eval_db_misconfigured"
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
    source_schema_ref: str = "schema:core-event@" + PIN_SHA
    registry_schema_ref: str = "schema:core-outbound@" + PIN_SHA
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
    token_provider: Callable[[], str] | None = None

    def source_id(self, kind: str) -> str:
        return f"{self.instance}.{kind}"
