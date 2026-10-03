"""`pulso/lab_query` (compute) and `pulso/lab_get_result` (read) over D.3 `/lab/*`.

Result views are three fixed containers by broker `data_class` (`public`, `financial`, `untrusted_text`); any
other class or an absent one -> `error unclassified_column`; `pii_*` -> `error pii_class_received` + leak signal.
`total_rows` null stays null (never rendered as 0)."""

from __future__ import annotations

import hashlib
import re
from typing import Any

from agent_core.domain.shared import ToolStatus

from pulso_core_runtime.tools._common import Args, Deps, Outcome, err, ok, text_arg
from pulso_core_runtime.tools.broker import BrokerTimeout
from pulso_core_runtime.tools.context import InvocationContext

MAX_ROWS = 200
MAX_PAGE_BYTES = 64 * 1024
CONTAINERS = ("public", "financial", "untrusted_text")


def normalise_sql(sql: str) -> str:
    return re.sub(r"\s+", " ", sql).strip().rstrip(";").strip()


def query_key(binding_ref: str, session_ref: str, revision: int, sql: str) -> str:
    raw = "|".join((binding_ref, session_ref, str(revision), normalise_sql(sql)))
    return hashlib.sha256(raw.encode("utf-8")).hexdigest()


def _session(deps: Deps, ic: InvocationContext) -> dict[str, Any] | None:
    session = deps.contexts.session(ic.binding_ref)
    if session is None and ic.extract_manifest_ref:
        opened = deps.broker.lab_open_session(ic.binding_ref, ic.extract_manifest_ref)
        session = {"session_ref": opened["session_ref"], "revision": opened["revision"]}
        deps.contexts.set_session(ic.binding_ref, session)
    return session


def shape_result(deps: Deps, ic: InvocationContext, page: dict[str, Any], query_ref: str | None) -> Outcome:
    columns = page.get("columns", [])
    for col in columns:
        cls = col.get("data_class")
        if isinstance(cls, str) and cls.startswith("pii"):
            deps.leaks.append((ic.binding_ref, "pii_class_received"))
            deps.leak_signal(ic.binding_ref, "pii_class_received")
            return err("pii_class_received")
        if cls not in CONTAINERS:
            return err("unclassified_column")
    rows = list(page.get("rows", []))
    clipped = False
    if len(rows) > MAX_ROWS:
        rows, clipped = rows[:MAX_ROWS], True
    while rows and len(repr(rows).encode("utf-8")) > MAX_PAGE_BYTES:
        rows = rows[: len(rows) // 2]
        clipped = True
    containers: dict[str, dict[str, Any]] = {}
    for cls in CONTAINERS:
        idx = [i for i, c in enumerate(columns) if c["data_class"] == cls]
        if idx:
            containers[cls] = {"columns": [{"name": columns[i]["name"], "type": columns[i].get("type")} for i in idx],
                               "rows": [[r[i] for i in idx] for r in rows]}
    deps.contexts.record_refs(ic.binding_ref, [str(page.get("receipt_ref") or ""), str(query_ref or "")])
    return ok({"containers": containers, "row_count": len(rows),
               "truncated": bool(page.get("truncated")) or clipped,
               "next_cursor": None if clipped else page.get("next_cursor"),
               "total_rows": page.get("total_rows"),  # None = unknown, never 0
               "receipt_ref": page.get("receipt_ref"), "result_digest": page.get("result_digest")})


def lab_query(deps: Deps, ic: InvocationContext, args: Args, run_id: str) -> Outcome:
    sql = text_arg(args, "sql")
    session = _session(deps, ic)
    if session is None:
        return err("pulso:no_extract_manifest")
    key = query_key(ic.binding_ref, session["session_ref"], session["revision"], sql)
    submitted = deps.broker.lab_submit_query(ic.binding_ref, session["session_ref"], key, sql,
                                             session["revision"])
    query_ref = str(submitted["query_ref"])
    try:
        state = deps.broker.lab_wait(ic.binding_ref, query_ref)
    except BrokerTimeout:
        return ToolStatus.timeout, None, "pulso:lab_query_timeout"
    if state.get("state") == "unknown":
        return ToolStatus.timeout, None, "pulso:lab_query_unknown"
    if state.get("state") != "completed" or not state.get("result_ref"):
        return err(f"pulso:lab_query_failed:{state.get('reason_code') or 'unknown'}")
    deps.contexts.record_refs(ic.binding_ref, [str(state.get("receipt_ref") or ""), str(state["result_ref"])])
    page = deps.broker.lab_result(ic.binding_ref, str(state["result_ref"]))
    return shape_result(deps, ic, page, query_ref)


def lab_get_result(deps: Deps, ic: InvocationContext, args: Args, run_id: str) -> Outcome:
    cursor = args.get("cursor")
    page = deps.broker.lab_result(ic.binding_ref, text_arg(args, "result_ref"),
                                  cursor if isinstance(cursor, str) else None)
    return shape_result(deps, ic, page, None)
