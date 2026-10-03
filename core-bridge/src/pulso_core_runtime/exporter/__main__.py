"""`python -m pulso_core_runtime.exporter` — runnable exporter loop (or one-shot with --once/--rescan/--sweep).

Environment (no secrets on the command line; key files hold a base64url 32-byte Ed25519 seed):
  CORE_EXPORT_DATABASE_URL (role exporter_ro), EXPECTED_RUNTIME_DB, EXPECTED_EVAL_DB, PULSO_TENANT_ID,
  PULSO_CORE_INSTANCE, PULSO_INGEST_BASE_URL, PULSO_EXPORTER_BINDING_REF, PULSO_EXPORTER_STATE_DIR,
  PULSO_EXPORTER_KEY_CONTROL_API(_KID), PULSO_EXPORTER_KEY_LAB_BROKER(_KID); optional PULSO_EXPORTER_POLL_S,
  PULSO_EXPORTER_RESCAN_S, PULSO_EXPORTER_SWEEP_S."""

from __future__ import annotations

import argparse
import json
import os
import signal
import sys
from collections.abc import Mapping
from pathlib import Path

import httpx
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey

from ..internal.auth import b64url_decode
from .auth import AudienceKey, ExporterTokenSigner
from .config import ExporterConfig
from .reader import CoreReader, ExporterError
from .runner import Schedule, run_loop
from .service import Exporter
from .state import ExporterState

REQUIRED = ("CORE_EXPORT_DATABASE_URL", "EXPECTED_RUNTIME_DB", "EXPECTED_EVAL_DB", "PULSO_TENANT_ID",
            "PULSO_CORE_INSTANCE", "PULSO_INGEST_BASE_URL", "PULSO_EXPORTER_BINDING_REF", "PULSO_EXPORTER_STATE_DIR",
            "PULSO_EXPORTER_KEY_CONTROL_API", "PULSO_EXPORTER_KEY_LAB_BROKER")


def _key(path: str) -> Ed25519PrivateKey:
    seed = b64url_decode(Path(path).read_text(encoding="ascii").strip())
    if len(seed) != 32:
        raise ValueError("exporter key file must hold a 32-byte seed")
    return Ed25519PrivateKey.from_private_bytes(seed)


def build_from_env(env: Mapping[str, str], client: httpx.Client | None = None) -> tuple[Exporter, Schedule]:
    missing = [k for k in REQUIRED if not env.get(k)]
    if missing:
        raise ValueError("missing environment: " + ", ".join(missing))  # names only, never values
    signer = ExporterTokenSigner(
        {"control-api": AudienceKey(env.get("PULSO_EXPORTER_KEY_CONTROL_API_KID", "exporter-control-api"),
                                    _key(env["PULSO_EXPORTER_KEY_CONTROL_API"])),
         "lab-broker": AudienceKey(env.get("PULSO_EXPORTER_KEY_LAB_BROKER_KID", "exporter-lab-broker"),
                                   _key(env["PULSO_EXPORTER_KEY_LAB_BROKER"]))},
        binding_ref=env["PULSO_EXPORTER_BINDING_REF"], tenant_id=env["PULSO_TENANT_ID"])
    cfg = ExporterConfig(tenant_id=env["PULSO_TENANT_ID"], instance=env["PULSO_CORE_INSTANCE"],
                         expected_runtime_db=env["EXPECTED_RUNTIME_DB"], expected_eval_db=env["EXPECTED_EVAL_DB"],
                         binding_ref=env["PULSO_EXPORTER_BINDING_REF"], token_for=signer.token_for)
    state_dir = Path(env["PULSO_EXPORTER_STATE_DIR"])
    state_dir.mkdir(parents=True, exist_ok=True)
    http = client or httpx.Client(base_url=env["PULSO_INGEST_BASE_URL"], timeout=30.0)
    ex = Exporter(cfg, CoreReader(env["CORE_EXPORT_DATABASE_URL"], cfg), ExporterState(state_dir / "state.sqlite"), http)
    schedule = Schedule(float(env.get("PULSO_EXPORTER_POLL_S", "5")), float(env.get("PULSO_EXPORTER_RESCAN_S", "900")),
                        float(env.get("PULSO_EXPORTER_SWEEP_S", str(24 * 3600))))
    return ex, schedule


def main(argv: list[str] | None = None, env: Mapping[str, str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="pulso-exporter")
    ap.add_argument("--once", action="store_true", help="one poll, then exit")
    ap.add_argument("--rescan", action="store_true", help="one anti-entropy pass (with open-run prefixes), then exit")
    ap.add_argument("--sweep", action="store_true", help="one full-chain sweep, then exit")
    args = ap.parse_args(argv)
    environ = os.environ if env is None else env
    try:
        ex, schedule = build_from_env(environ)
    except (ValueError, OSError) as exc:
        print(f"exporter: configuration error: {type(exc).__name__}: {exc}", file=sys.stderr)
        return 2
    stop = {"flag": False}
    signal.signal(signal.SIGINT, lambda *_: stop.update(flag=True))
    try:
        if args.once or args.rescan or args.sweep:
            rep = ex.sweep() if args.sweep else ex.rescan() if args.rescan else ex.poll_once()
            summary = {"batches_sent": rep.batches_sent, "events_sent": rep.events_sent, "deferred": rep.deferred,
                       "stopped": rep.stopped, "partial_reasons": rep.partial_reasons, **ex.coverage()}
            print(json.dumps(summary, sort_keys=True))
            return 1 if rep.stopped else 0
        stats = run_loop(ex, schedule, stop=lambda: stop["flag"])
        print(json.dumps({"polls": stats.polls, "rescans": stats.rescans, "sweeps": stats.sweeps,
                          "errors": stats.errors}))
        return 0
    except ExporterError as exc:
        print(f"exporter: {exc.code}", file=sys.stderr)
        return 3
    finally:
        ex.close()


if __name__ == "__main__":
    raise SystemExit(main())
