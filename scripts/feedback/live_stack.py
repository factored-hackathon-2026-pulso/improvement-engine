#!/usr/bin/env python3
"""OWN local agent-core stack for the FDBK1 live test, isolated by PULSO_STACK_PREFIX (default `pulso-fdbk1`).

    PULSO_STACK_PREFIX=pulso-fdbk1 python scripts/feedback/live_stack.py up|down [--purge]|health

Thin wrapper over scripts/battery/demo_core.py (same imports, same env-file handling: values go into child-process
environments only and are never printed): it only renames the containers (`<prefix>-postgres`, `<prefix>-llm-gateway`),
moves the ports (PG 55472, gateway 8120, agent-core 8012; override with PULSO_FDBK1_PG_PORT / _GW_PORT / _CORE_PORT)
and uses its own state directory `.dev-stack/<prefix>/`. It does not touch the singleton stack of stack.py or the
other lanes. Local test data only: never point it at a shared environment.
"""
from __future__ import annotations

import importlib.util
import os
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("demo_core", HERE.parent / "battery" / "demo_core.py")
dc = importlib.util.module_from_spec(spec)
spec.loader.exec_module(dc)

PREFIX = os.environ.get("PULSO_STACK_PREFIX", "pulso-fdbk1")
dc.PG, dc.GW = f"{PREFIX}-postgres", f"{PREFIX}-llm-gateway"
dc.PG_PORT = int(os.environ.get("PULSO_FDBK1_PG_PORT", "55472"))
dc.GW_PORT = int(os.environ.get("PULSO_FDBK1_GW_PORT", "8120"))
dc.PORT = int(os.environ.get("PULSO_FDBK1_CORE_PORT", "8012"))
dc.STATE = dc.ds.STATE / PREFIX
dc.AGENTS = os.environ.get("PULSO_FDBK1_AGENTS", "recepcion,disputas,consultas")

if __name__ == "__main__":
    cmd = sys.argv[1] if len(sys.argv) > 1 else "health"
    if cmd == "down":
        dc.down(purge="--purge" in sys.argv)
    elif cmd == "up":
        dc.up()
    else:
        sys.exit(dc.health())
