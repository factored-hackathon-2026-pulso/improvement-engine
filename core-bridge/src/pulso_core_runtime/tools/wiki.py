"""`pulso/wiki_read|wiki_explore|wiki_transform` over D.3 `/wiki/*`. The memory snapshot comes from the
frozen invocation context, never from tool args."""

from __future__ import annotations

import json
from typing import Any

from agent_core.domain.shared import ToolStatus

from pulso_core_runtime.tools._common import Args, Deps, Outcome, err, ok, text_arg
from pulso_core_runtime.tools.context import InvocationContext

_OPS = {"put", "delete"}


def wiki_read(deps: Deps, ic: InvocationContext, args: Args, run_id: str) -> Outcome:
    if ic.memory_snapshot_ref is None:
        return err("pulso:no_memory_snapshot")
    body = deps.broker.wiki_read(ic.binding_ref, ic.memory_snapshot_ref, [text_arg(args, "path")])
    refs: list[str] = []
    for e in body.get("entries", []):
        refs += [str(e.get("path", "")), *[str(r) for r in e.get("evidence_refs", [])]]
    deps.contexts.record_refs(ic.binding_ref, refs)
    return ok({"entries": body.get("entries", []), "base_digest": body.get("base_digest")})


def wiki_explore(deps: Deps, ic: InvocationContext, args: Args, run_id: str) -> Outcome:
    if ic.memory_snapshot_ref is None:
        return err("pulso:no_memory_snapshot")
    body = deps.broker.wiki_explore(ic.binding_ref, ic.memory_snapshot_ref, "", text_arg(args, "query"))
    return ok({"entries": body.get("entries", []), "next_cursor": body.get("next_cursor"),
               "truncated": bool(body.get("truncated"))})


def wiki_transform(deps: Deps, ic: InvocationContext, args: Args, run_id: str) -> Outcome:
    """`source_ref` is the `base_digest` of a prior read; `transform` is a JSON list of scratch operations
    `{op: put|delete, path, content?, evidence_refs?}` (D.3). The published head is never modified here."""
    if ic.memory_snapshot_ref is None:
        return err("pulso:no_memory_snapshot")
    try:
        ops: Any = json.loads(text_arg(args, "transform"))
    except json.JSONDecodeError:
        return err("invalid_args", ToolStatus.denied)
    if not isinstance(ops, list) or not all(isinstance(o, dict) and o.get("op") in _OPS for o in ops):
        return err("invalid_args", ToolStatus.denied)
    body = deps.broker.wiki_transform(ic.binding_ref, ic.memory_snapshot_ref, text_arg(args, "source_ref"), ops)
    deps.contexts.record_refs(ic.binding_ref, [str(body.get("scratch_ref", "")), str(body.get("diff_ref", ""))])
    return ok({"scratch_ref": body.get("scratch_ref"), "diff_ref": body.get("diff_ref"),
               "manifest_digest": body.get("manifest_digest"), "violations": body.get("violations", [])})
