"""Run inside the agent-core env: dump testing.engine_world.demo_calibration() as JSON (cal-demo)."""
import json
import sys

from testing.engine_world import demo_calibration

cal = demo_calibration()
data = cal.model_dump(mode="json") if hasattr(cal, "model_dump") else cal
sys.stdout.write(json.dumps(data, indent=2, sort_keys=True) + "\n")
