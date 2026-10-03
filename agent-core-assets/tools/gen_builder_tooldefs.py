"""Generate the `registry/*` ToolDefs of pulso-evolution from BUILDER_TOOL_DEFS (never hand-edited).

Run inside the agent-core env (see assetcheck.agentcore_cmd):
    gen_builder_tooldefs.py <out_dir>          # write <out_dir>/tools/registry/*.yaml
Prints a JSON list of relative paths written.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

from agent_core.composition import BUILDER_TOOL_DEFS
from agent_core.registry.yaml_io import dump_entities


def render() -> dict[str, bytes]:
    return dump_entities(BUILDER_TOOL_DEFS.values())


def main(out: Path) -> None:
    written = []
    for rel, data in render().items():
        target = out / rel
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(data)
        written.append(rel)
    sys.stdout.write(json.dumps(sorted(written)) + "\n")


if __name__ == "__main__":
    main(Path(sys.argv[1]))
