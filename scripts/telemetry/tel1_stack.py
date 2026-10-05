#!/usr/bin/env python3
"""TEL1 own local stack: the EV2 battery stack (`scripts/battery/demo_core.py`) under TEL1 names and ports.

    PULSO_AGENT_CORE_DIR=<agent-core main checkout> python scripts/telemetry/tel1_stack.py up|down [--purge]|health

Only the container names and ports differ (pulso-tel1-postgres :55561, pulso-tel1-llm-gateway :8261, serve :8061), so
this never touches another lane's containers. Credentials stay in child-process environments (demo_core / stack.py).
"""
import importlib.util
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("demo_core", REPO / "scripts" / "battery" / "demo_core.py")
dc = importlib.util.module_from_spec(spec)
spec.loader.exec_module(dc)
dc.PG, dc.GW, dc.PG_PORT, dc.GW_PORT, dc.PORT = "pulso-tel1-postgres", "pulso-tel1-llm-gateway", 55561, 8261, 8061

if __name__ == "__main__":
    cmd = sys.argv[1] if len(sys.argv) > 1 else "health"
    if cmd == "down":
        dc.down(purge="--purge" in sys.argv)
    elif cmd == "up":
        dc.up()
    else:
        sys.exit(dc.health())
