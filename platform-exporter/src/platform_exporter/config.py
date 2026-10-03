"""Exporter configuration and transport constants."""

from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass

CONTRACT = "pulso-observations-2"
OBSERVATIONS_PATH = "/internal/v1/platform/observations"
ARTIFACTS_PATH = "/internal/v1/broker/artifacts"
KIB = 1024


@dataclass(frozen=True)
class ExporterConfig:
    tenant_id: str
    instance: str  # source_id = "<instance>.events"; partition = "tenant.<tenant_id>"
    binding_ref: str
    window_seconds: int = 3600  # UTC-aligned windows over event_time; a window closes at its end + lateness
    allowed_lateness_seconds: int = 0
    start_sequence: int = 1  # first expected event_log.sequence (a lower first row is a gap)
    gap_grace_seconds: float = 120.0  # a sparse hole may still commit: hold back before declaring gap_suspected
    batch_max_events: int = 500
    batch_max_bytes: int = 512 * KIB
    max_retries: int = 5
    backoff_cap_seconds: float = 60.0
    extra_event_types: frozenset[str] = frozenset()  # versioned allow-list additions (e.g. one auth.* type)
    # Contract 1.0.0 interim shape: exporter metadata as `event_type` "exporter.*" instead of kind=exporter_finding.
    # Default False (contract 1.1.0). Set True only for a consumer that has not adopted the 1.1.0 discriminator yet.
    legacy_prefix: bool = False
    source_schema_ref: dict[str, str] | None = None
    token_for: Callable[[str], str] | None = None  # route class "observations" | "artifacts" | "cursor"

    @property
    def source_id(self) -> str:
        return f"{self.instance}.events"

    @property
    def partition(self) -> str:
        return f"tenant.{self.tenant_id}"

    def token(self, route: str) -> str | None:
        return self.token_for(route) if self.token_for is not None else None
