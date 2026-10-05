#!/usr/bin/env python3
"""KEY MERGE for the integrated rig (G10): one identity-keys and one staff-keys file for agent-core.

    python scripts/integrated-rig/merge_keys.py --state-dir .dev-stack --platform-keys <dir> [--engine-kid pulso-engine-dev-1]

agent-core `--identity-keys` and `--staff-keys` are each ONE file. The platform mints its own Ed25519 keys
(`gen_agent_keys`: kids cc-principal-*, cc-grant-*, cc-staff-*), our dev stack mints the engine `builder` kid and the
dev admin/customer kids (`scripts/dev-stack/identity.py keys`). This tool writes the UNION of both worlds back into
the state dir files (agent-core re-reads them every few seconds, no restart):

  identity-keys.json  principal_keys + delegation_keys   = dev stack keys  U  platform keys
  staff-keys.json     principal_keys                      = dev stack keys (dev admin kid, engine kid)  U  platform staff kid

Refusals (exit 2, a message that names a KID and never a key): a kid already present with a DIFFERENT public key, a
missing input file, an engine kid or a dev admin kid that is not in the result. Public keys only are read and written;
nothing key-shaped is ever printed (only kids and counts). The write is atomic (temp file + replace).
"""
from __future__ import annotations

import argparse
import json
import os
import sys
from pathlib import Path

SECTIONS = {"identity-keys.json": ("principal_keys", "delegation_keys"), "staff-keys.json": ("principal_keys",)}


class MergeError(Exception):
    """A refusal. The message names kids and files, never key material."""


def union(base: dict, extra: dict, sections: tuple, origin: str) -> dict:
    """Union of two public-key documents, section by section. The same kid with a different key is refused."""
    out = {s: dict(base.get(s) or {}) for s in sections}
    for s in sections:
        for kid, pub in (extra.get(s) or {}).items():
            if not isinstance(kid, str) or not isinstance(pub, str) or not kid or not pub:
                raise MergeError(f"{origin}: {s} has a malformed entry")
            if kid in out[s] and out[s][kid] != pub:
                raise MergeError(f"{origin}: kid {kid} in {s} already exists with a different key (rotate with a new suffix)")
            out[s][kid] = pub
    return out


def check_result(identity: dict, staff: dict, engine_kid: str, platform_kids: set) -> None:
    if engine_kid not in staff["principal_keys"]:
        raise MergeError(f"the engine kid {engine_kid} is not in staff-keys: run scripts/dev-stack/identity.py keys first")
    if engine_kid not in identity["principal_keys"]:
        raise MergeError(f"the engine kid {engine_kid} is not in identity-keys")
    if not (set(staff["principal_keys"]) - {engine_kid} - platform_kids):
        raise MergeError("no dev admin kid in staff-keys: the registry import token would stop working")
    have = set(identity["principal_keys"]) | set(identity["delegation_keys"]) | set(staff["principal_keys"])
    missing = platform_kids - have
    if missing:
        raise MergeError("platform kids missing from the result: " + ", ".join(sorted(missing)))


def read_doc(path: Path) -> dict:
    try:
        doc = json.loads(path.read_text(encoding="utf-8"))
    except OSError:
        raise MergeError(f"cannot read {path.name}") from None
    except json.JSONDecodeError:
        raise MergeError(f"{path.name} is not JSON") from None
    if not isinstance(doc, dict):
        raise MergeError(f"{path.name} is not a JSON object")
    return doc


def write_atomic(path: Path, doc: dict) -> None:
    tmp = path.with_name(path.name + ".tmp")
    tmp.write_text(json.dumps(doc, indent=2), encoding="utf-8")
    os.replace(tmp, path)


def merge(state_dir: Path, platform_dir: Path, engine_kid: str) -> dict:
    result = {}
    platform_kids = set()
    for name, sections in SECTIONS.items():
        base, plat = read_doc(state_dir / name), read_doc(platform_dir / name)
        for s in sections:
            platform_kids |= set((plat.get(s) or {}).keys())
        result[name] = union(base, plat, sections, f"platform {name}")
    ident = result["identity-keys.json"]
    check_result({"principal_keys": ident["principal_keys"], "delegation_keys": ident["delegation_keys"]},
                 result["staff-keys.json"], engine_kid, platform_kids)
    for name, doc in result.items():
        write_atomic(state_dir / name, doc)
    return {name: sorted(k for s in doc for k in doc[s]) for name, doc in result.items()}


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--state-dir", type=Path, required=True)
    ap.add_argument("--platform-keys", type=Path, required=True, help="directory written by cc_platform.scripts.gen_agent_keys")
    ap.add_argument("--engine-kid", default="pulso-engine-dev-1")
    a = ap.parse_args(argv)
    try:
        kids = merge(a.state_dir, a.platform_keys, a.engine_kid)
    except MergeError as exc:
        print(f"merge_keys refused: {exc}", file=sys.stderr)
        return 2
    for name, ks in kids.items():
        print(f"{name}: {len(ks)} kids ({', '.join(ks)})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
