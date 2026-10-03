"""Run the exporter: `python -m platform_exporter` (environment only, no secrets in files).

PLATFORM_DB_URL                  sqlite file path, or postgresql://... for a read-only role
PULSO_CONTROL_API_URL            base URL of control-api
PULSO_TENANT_ID, PLATFORM_INSTANCE, PULSO_BINDING_REF
PULSO_EXPORTER_KEY_CONTROL_API   path of the 0400 key file (base64url 32-byte Ed25519 seed) the image entrypoint
                                 materialises from PULSO_EXPORTER_KEY_CONTROL_API_SEED; REQUIRED (fail closed)
PULSO_EXPORTER_KEY_LAB_BROKER    optional path of the lab-broker audience key (artifacts route only); without it
                                 an artifact upload fails closed, the control-api key is never reused
PULSO_EXPORTER_KEY_*_KID         optional kids (default exporter-control-api / exporter-lab-broker)
EXPORTER_STATE_PATH              local SQLite state (own volume), default ./platform-exporter-state.sqlite
PULSO_SERVICE_TOKEN              DEV OVERRIDE ONLY: honoured only together with PULSO_DEV_STATIC_TOKEN=1, replaces
                                 the key file; tokens are otherwise minted per HTTP attempt (auth.py)"""

from __future__ import annotations

import os
import sys
import time
from collections.abc import Mapping

import httpx

from .auth import AudienceKey, RuntimeConfigInvalid, ServiceTokenSigner, load_seed_file
from .config import ExporterConfig
from .service import Exporter
from .source import PostgresSource, SqliteSource
from .state import ExporterState

CONTROL_VAR, LAB_VAR = "PULSO_EXPORTER_KEY_CONTROL_API", "PULSO_EXPORTER_KEY_LAB_BROKER"
REQUIRED = ("PLATFORM_DB_URL", "PULSO_CONTROL_API_URL", "PULSO_TENANT_ID", "PLATFORM_INSTANCE", "PULSO_BINDING_REF")


def _token_provider(env: Mapping[str, str]):
    static, dev = env.get("PULSO_SERVICE_TOKEN"), env.get("PULSO_DEV_STATIC_TOKEN") == "1"
    if static and not dev:
        raise RuntimeConfigInvalid("PULSO_SERVICE_TOKEN is only honoured with PULSO_DEV_STATIC_TOKEN=1 (dev override)")
    if dev:
        if not static:
            raise RuntimeConfigInvalid("PULSO_DEV_STATIC_TOKEN=1 requires PULSO_SERVICE_TOKEN")
        return lambda _route: static
    path = env.get(CONTROL_VAR)
    if not path:
        raise RuntimeConfigInvalid(f"{CONTROL_VAR} is not set (path of the control-api audience key file)")
    keys = {"control-api": AudienceKey(env.get(CONTROL_VAR + "_KID", "exporter-control-api"),
                                       load_seed_file(path, var=CONTROL_VAR))}
    if env.get(LAB_VAR):
        keys["lab-broker"] = AudienceKey(env.get(LAB_VAR + "_KID", "exporter-lab-broker"),
                                         load_seed_file(env[LAB_VAR], var=LAB_VAR))
    return ServiceTokenSigner(keys, binding_ref=env["PULSO_BINDING_REF"], tenant_id=env["PULSO_TENANT_ID"]).token_for


def build_from_env(env: Mapping[str, str], client: httpx.Client | None = None) -> tuple[Exporter, float]:
    missing = [k for k in REQUIRED if not env.get(k)]
    if missing:
        raise RuntimeConfigInvalid("missing environment: " + ", ".join(missing))
    token_for = _token_provider(env)  # fails closed before anything is opened
    url = env["PLATFORM_DB_URL"]
    source = PostgresSource(url) if url.startswith(("postgres://", "postgresql://")) else SqliteSource(url)
    cfg = ExporterConfig(tenant_id=env["PULSO_TENANT_ID"], instance=env["PLATFORM_INSTANCE"],
                         binding_ref=env["PULSO_BINDING_REF"], token_for=token_for)
    http = client or httpx.Client(base_url=env["PULSO_CONTROL_API_URL"], timeout=30.0)
    state = ExporterState(env.get("EXPORTER_STATE_PATH", "platform-exporter-state.sqlite"))
    return Exporter(cfg, source, state, http), 5.0


def main() -> int:
    try:
        ex, poll_s = build_from_env(os.environ)
    except RuntimeConfigInvalid as exc:
        print(str(exc), file=sys.stderr)  # names variables only, never a value
        return 2
    next_rescan = time.monotonic()
    try:
        while True:
            ex.poll_once()
            if time.monotonic() >= next_rescan:
                ex.rescan()
                next_rescan = time.monotonic() + 900
            time.sleep(poll_s)
    except RuntimeConfigInvalid as exc:  # e.g. an artifact upload without a lab-broker key: stop loudly, fail closed
        print(str(exc), file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
