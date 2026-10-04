"""CMPpy: generic ChangeSpec -> DraftPlan compile (stand-in, python) over a seeded base world.

Label: compiler_label = "claude-standin(python)". The world comes from the declaration next to the world
directories (agent-core-assets/worlds/seeded-base.world.yaml); nothing about a finding or category is hardcoded.

Reference mapping (refs match ^[a-z_]+:[A-Za-z0-9._-]+@[0-9]+$): `prompt:<id without 'p/'>@<major>` and
`eval_suite:<id>@<major>`; the world asset currently at <major> of its semver is the only target. Precondition
digest = sha256 of the LF-normalised asset file bytes.

Denied reasons (closed set of the compile.out schema):
  kind_not_supported  op/kind pair not in the world's targetable_kinds (replace_prompt, add_eval_suite)
  missing_precondition precondition_digest differs from the digest of the asset in the world
  outside_bridge      target is not the slot declared by the world, or a route is not the bridge flow
  mutable_reference   no new_ref, or new_ref is not a strictly newer version (would overwrite a published one)

Core dry-run hook: pass dry_run=callable(operations)->"sha256:..." to take the plan digest from the real Core
dry-run instead of the local canonical-JSON digest.
"""
import hashlib
import json
import re
from pathlib import Path
from typing import Callable

import yaml

from .contracts import validate_in

LABEL = "claude-standin(python)"
_KINDS = {("replace", "prompt"): "replace_prompt", ("add", "eval_suite"): "add_eval_suite"}
_REF = re.compile(r"^([a-z_]+):([A-Za-z0-9._-]+)@([0-9]+)$")
_BRIDGE_MAJOR = 1
_WORLDS = Path(__file__).resolve().parents[3] / "agent-core-assets" / "worlds"


def load_world(path) -> dict:
    path = Path(path)
    w = yaml.safe_load(path.read_text("utf-8"))
    w["_dir"] = str(path.parent / w["world"])
    return w


def bundle_ref(world: dict) -> str:
    """Base bundle ref named by the seeded world (`bundle:<world>@1`)."""
    return f"bundle:{world['world']}@{_BRIDGE_MAJOR}"


def bridge_ref(world: dict) -> str:
    """Workflow bridge ref of the flow that uses the replaceable prompt (`bridge:<flow>@1`)."""
    return f"bridge:{world['replaceable_prompt']['used_by']['flow']}@{_BRIDGE_MAJOR}"


def _slots(world: dict) -> dict:
    p = world["replaceable_prompt"]
    s = world["eval_suite_slot"]["current"]
    sid, sver = s.split("@")
    return {
        "prompt": {"name": p["id"].split("/", 1)[-1], "id": p["id"], "version": p["version"], "kind": "prompt",
                   "file": Path(world["_dir"]) / "prompts" / "p" / f"{p['id'].split('/', 1)[-1]}@{p['version']}.yaml"},
        "eval_suite": {"name": sid, "id": sid, "version": sver, "kind": "eval_suite",
                       "file": Path(world["_dir"]) / "eval_suites" / f"{sid}@{sver}.yaml"},
    }


def _major(version: str) -> int:
    return int(version.split(".")[0])


def asset_digest(world: dict, ref: str) -> str:
    m = _REF.match(ref)
    slot = _slots(world)[m.group(1)]
    return "sha256:" + hashlib.sha256(slot["file"].read_bytes().replace(b"\r\n", b"\n")).hexdigest()


def _canonical_digest(ops: list) -> str:
    return "sha256:" + hashlib.sha256(json.dumps(ops, sort_keys=True, separators=(",", ":")).encode()).hexdigest()


def _check(op: dict, world: dict) -> str | None:
    if _KINDS.get((op["op"], op["target_kind"])) not in world["targetable_kinds"]:
        return "kind_not_supported"
    slot = _slots(world)[op["target_kind"]]
    m = _REF.match(op["target_ref"])
    if not m or m.group(1) != op["target_kind"] or m.group(2) != slot["name"] or int(m.group(3)) != _major(slot["version"]):
        return "outside_bridge"
    if op["precondition_digest"] != asset_digest(world, op["target_ref"]):
        return "missing_precondition"
    n = _REF.match(op.get("new_ref", "") or "")
    if not n or n.group(1) != m.group(1) or n.group(2) != m.group(2) or int(n.group(3)) <= int(m.group(3)):
        return "mutable_reference"
    return None


def compile_change_spec(doc: dict, world: dict, dry_run: Callable[[list], str] | None = None) -> dict:
    errs = validate_in("compile", doc)
    if errs:
        raise ValueError(f"compile input invalid: {errs}")
    spec = doc["change_spec"]
    head = {"contract_version": "engine-steps/0", "step": "compile", "run_id": doc["run_id"], "data_class": doc["data_class"],
            "compiler_label": LABEL}
    flow = world["replaceable_prompt"]["used_by"]["flow"]
    reason = None
    for op in spec["operations"]:
        reason = _check(op, world)
        if reason:
            break
    targets = [op["target_ref"] for op in spec["operations"]]
    if not reason and len(set(targets)) != len(targets):
        reason = "mutable_reference"  # two ops would publish the same new version of one target
    if not reason and (any(r != flow for r in spec["affected_routes"])
                       or spec["workflow_bridge_ref"] != f"bridge:{flow}@{_BRIDGE_MAJOR}"
                       or spec["base_bundle_ref"] != doc["base_bundle_ref"]):
        reason = "outside_bridge"
    if reason:
        return {**head, "status": "denied", "denied_reason": reason}
    ops = [{k: op[k] for k in ("op", "target_kind", "target_ref", "new_ref", "precondition_digest")} for op in spec["operations"]]
    digest = dry_run(ops) if dry_run else _canonical_digest(ops)
    if not re.fullmatch(r"sha256:[0-9a-f]{64}", str(digest)):
        raise ValueError(f"dry-run digest malformed: {digest!r}")
    return {**head, "status": "compiled", "draft_plan": {"operations": ops, "digest": digest}}
