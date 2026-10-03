"""Run inside the agent-core env: compute the expected seed state of a merged root.

Usage: agentcore_state.py <root>   -> JSON {agent_id: {release_id, release_hash, entities[], eval_suites[]}}
Mirrors RegistryService.import_seed (load_seed, release_hash, release_id_for, content_hash) with no database.
"""

import json
import sys
from pathlib import Path

from agent_core.domain import EntityKind
from agent_core.registry.candidate import release_hash
from agent_core.registry.entities import content_hash, version_ref
from agent_core.registry.service import RegistryService, release_id_for
from agent_core.registry.yaml_io import load_seed


def main(root: Path) -> None:
    pinned_list, suites = load_seed(root)
    RegistryService._check_seed_suites(pinned_list, suites)  # same pre-write guard as import_seed
    out = {}
    for pinned in pinned_list:
        (agent_id,) = pinned.aliases
        digest = release_hash(pinned.release)
        ents = sorted(({"kind": str(version_ref(e).kind), "id": e.id, "version": e.version,
                        "content_hash": content_hash(e)} for e in pinned.entities),
                      key=lambda d: (d["kind"], d["id"], d["version"]))
        out[agent_id] = {
            "release_id": release_id_for(digest), "release_hash": digest,
            "agent_version": pinned.release.entities[EntityKind.agent][agent_id], "entities": ents,
            "eval_suites": sorted(({"id": s.id, "version": s.version, "content_hash": content_hash(s)}
                                   for s in suites if s.agent_id == agent_id),
                                  key=lambda d: (d["id"], d["version"])),
        }
    sys.stdout.write(json.dumps(out, indent=2, sort_keys=True) + "\n")


if __name__ == "__main__":
    main(Path(sys.argv[1]))
