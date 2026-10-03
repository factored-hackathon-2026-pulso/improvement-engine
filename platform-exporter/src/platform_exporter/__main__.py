"""Run the exporter: `python -m platform_exporter` (environment only, no secrets in files).

PLATFORM_DB_URL        sqlite file path, or postgresql://... for a read-only role
PULSO_CONTROL_API_URL  base URL of control-api
PULSO_TENANT_ID, PLATFORM_INSTANCE, PULSO_BINDING_REF
EXPORTER_STATE_PATH    local SQLite state (own volume), default ./platform-exporter-state.sqlite
PULSO_SERVICE_TOKEN    optional static bearer for local runs; production mints per-attempt JWTs (PL-L5)"""

from __future__ import annotations

import os
import time

import httpx

from .config import ExporterConfig
from .service import Exporter
from .source import PostgresSource, SqliteSource
from .state import ExporterState


def main() -> None:
    url = os.environ["PLATFORM_DB_URL"]
    source = PostgresSource(url) if url.startswith(("postgres://", "postgresql://")) else SqliteSource(url)
    token = os.environ.get("PULSO_SERVICE_TOKEN")
    cfg = ExporterConfig(tenant_id=os.environ["PULSO_TENANT_ID"], instance=os.environ["PLATFORM_INSTANCE"],
                         binding_ref=os.environ["PULSO_BINDING_REF"],
                         token_for=(lambda _route: token) if token else None)
    client = httpx.Client(base_url=os.environ["PULSO_CONTROL_API_URL"], timeout=30.0)
    ex = Exporter(cfg, source, ExporterState(os.environ.get("EXPORTER_STATE_PATH", "platform-exporter-state.sqlite")),
                  client)
    next_rescan = time.monotonic()
    while True:
        ex.poll_once()
        if time.monotonic() >= next_rescan:
            ex.rescan()
            next_rescan = time.monotonic() + 900
        time.sleep(5)


if __name__ == "__main__":
    main()
