"""INT0: real-Core `CoreHooks` for the E2E-THREAD-01 runner, over the existing bridge client (`codex_standin.bridge.Bridge`).

Real here: `dry_run` (step 5, `POST /core-authoring/dry-run`) and `alias_read` (`GET /core-state/aliases/...`).
NOT provided (stay stand-in, see THREAD01.md): `run_arms` and `publish`. Both need a frozen proposal produced by a
writer stage (scripted model + control-api/gateway doubles) and, for publish, a human-issuer JWS bound to that
proposal; the thread's own draft is not written to Core by any hook yet. No secrets are handled here: the bridge
client owns its signing key.
"""
from __future__ import annotations

import re
from pathlib import Path
from typing import Any, Callable

import yaml

from . import compile_step as cmp

_REF = re.compile(r"^([a-z_]+):([A-Za-z0-9._-]+)@([0-9]+)$")
DOCS = {"description": "thread01 change", "rationale": "recorded synthetic opportunity", "changelog": "thread01"}


def _load(path: Path) -> dict:
    return yaml.safe_load(path.read_text("utf-8"))


def changes_from_ops(world: dict, ops: list[dict]) -> list[dict]:
    """Compiled operations -> Core `ChangeIn` drafts: the base asset content with the version of `new_ref`."""
    slots, out = cmp._slots(world), []
    for op in ops:
        kind = op["target_kind"]
        content = _load(slots[kind]["file"])
        content["version"] = f"{_REF.match(op['new_ref']).group(3)}.0.0"
        out.append({"kind": kind, "content": content, "docs": dict(DOCS)})
    return out


def make_dry_run(bridge: Any, world: dict, *, tenant: str, agent_id: str, base_release_id: str | None) -> Callable:
    def dry_run(ops: list[dict]) -> str:
        body = {"schema_version": "1", "tenant_id": tenant, "agent_id": agent_id, "base_release_id": base_release_id,
                "changes": changes_from_ops(world, ops)}
        r = bridge.call("POST", "/core-authoring/dry-run", "dry_run", tenant, json=body)
        if r.status_code != 200:
            raise RuntimeError(f"core dry-run http {r.status_code}")
        res = r.json()
        if not res.get("valid"):
            raise RuntimeError("core dry-run refused: " + "; ".join(f"{v['rule']}: {v['message']}" for v in res["violations"]))
        h = res["candidate_hash"]
        return h if h.startswith("sha256:") else "sha256:" + h
    return dry_run


def make_alias_read(bridge: Any, *, tenant: str, agent_id: str) -> Callable:
    def alias_read(ctx: Any, alias: str) -> dict:
        r = bridge.call("GET", f"/core-state/aliases/{agent_id}/{alias}", "aliases", tenant)
        if r.status_code != 200:
            raise RuntimeError(f"core alias read http {r.status_code}")
        return {"release_id": r.json()["release_id"], "alias": alias}
    return alias_read
