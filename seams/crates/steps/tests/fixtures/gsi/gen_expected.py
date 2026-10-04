"""Regenerate cases.expected.json from the reviewed Python reference (e2e-core gate_step.gate_verdict).

Run from the repo root:  python seams/crates/steps/tests/fixtures/gsi/gen_expected.py
cases.json holds the Rust `gate::run` input envelopes; expected is the Python reference output (or
{"error": true} when the reference raises ValueError for an invalid gate input).
"""
import json
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[5]
sys.path.insert(0, str(ROOT / "e2e-core" / "src"))
from claude_standin import gate_step as G  # noqa: E402

cases = json.loads((HERE / "cases.json").read_text("utf-8"))
expected = {}
for c in cases:
    env = c["input"]
    world = {"authors": env["world_authors"]}
    try:
        expected[c["name"]] = G.gate_verdict(env["gate_in"], env["reports"], world)
    except ValueError:
        expected[c["name"]] = {"error": True}
(HERE / "cases.expected.json").write_text(json.dumps(expected, indent=1, sort_keys=True) + "\n", "utf-8")
print(f"{len(expected)} cases")
