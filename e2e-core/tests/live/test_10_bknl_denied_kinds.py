"""BKNL live: five denied drafts against the REAL Core dry-run (`POST /core-authoring/dry-run` through the bridge, as in
`core_hooks.make_dry_run`). Per negative: the engine's compile step labels it with its named denial reason (the real
compile, or a permissive stub when BKNL_COMPILE=permissive: that run is the RED demonstration and MUST fail), the real Core
answer is classified (accepted | refused | denied), and NOTHING is published (aliases prod/staging, releases, proposals and
draft writes are identical before and after). Read-only against Core: dry-run writes nothing.

Record the answers as replay fixtures: BKNL_RECORD_DIR=<dir> (see tests/fixtures/bknl and test_bknl_replay.py)."""
from __future__ import annotations

import datetime
import json
import os
from pathlib import Path
from typing import Any

import pytest
from claude_standin import bknl as B
from claude_standin import compile_step as C
from claude_standin import core_hooks as H
from claude_standin import thread01 as T
from codex_standin.engine import TENANT

pytestmark = pytest.mark.live
AGENT = "atencion-tarea"
WORLD = C.load_world(T.WORLD_FILE)


def _permissive(doc: dict, world: dict, dry_run: Any = None) -> dict:
    return {"status": "compiled", "step": "compile", "draft_plan": {"operations": doc["change_spec"]["operations"], "digest": "sha256:" + "0" * 64}}


COMPILE = _permissive if os.environ.get("BKNL_COMPILE") == "permissive" else C.compile_change_spec


def snapshot(stack: Any) -> dict:
    ar = H.make_alias_read(stack.bridge, tenant=TENANT, agent_id=AGENT)
    db = stack.runtime_db
    return {"aliases": {a: ar(None, a)["release_id"] for a in ("prod", "staging")},
            "releases": sorted(r[0] for r in db.rows("select release_id from reg_releases")),
            "proposals": db.one("select count(*) from reg_proposals"), "draft_writes": db.one("select count(*) from reg_draft_writes")}


@pytest.mark.parametrize("neg", B.NEGATIVES, ids=lambda n: n.id)
def test_denied_draft_is_labelled_by_the_engine_and_not_accepted_by_the_real_core(stack: Any, neg: B.Negative, effect: Any) -> None:
    before = snapshot(stack)
    engine = None
    if neg.engine_op is not None:
        engine = B.assert_engine_denies(neg, WORLD, COMPILE)  # fails while a denied kind slips through
    body = B.core_request(WORLD, neg, tenant=TENANT, agent_id=AGENT, base_release_id=before["aliases"]["prod"])
    r = stack.bridge.call("POST", "/core-authoring/dry-run", "dry_run", TENANT, json=body)
    try:
        answer = r.json()
    except ValueError:
        answer = None
    core = B.classify_core(r.status_code, answer)
    after = snapshot(stack)
    effect(f"bknl_{neg.id}", {"engine_reason": engine, "core": core})
    rec = os.environ.get("BKNL_RECORD_DIR")
    if rec:
        Path(rec).mkdir(parents=True, exist_ok=True)
        (Path(rec) / f"{neg.id}.json").write_text(json.dumps({
            "provenance": {"image": os.environ.get("E2E_BASE_IMAGE"), "recorded_utc": datetime.datetime.now(datetime.UTC).strftime("%Y-%m-%d"),
                           "command": os.environ.get("BKNL_COMMAND", "e2e-core/run.ps1 -PytestArgs tests/live/test_10_bknl_denied_kinds.py"),
                           "endpoint": "POST /internal/v1/core-authoring/dry-run (bridge, purpose authoring_dry_run)",
                           "redaction": "service JWT and headers are not recorded; tenant and agent ids are the synthetic e2e ones"},
            "negative": neg.id, "expected_reason": neg.expected_reason, "request": body,
            "response": {"status": r.status_code, "body": answer}, "registry_before": before, "registry_after": after,
        }, indent=2, sort_keys=True) + "\n", encoding="ascii")
    assert after == before, "a denied draft changed the registry"
    if neg.core_refuses:
        assert core["outcome"] != "accepted", f"{neg.id}: the real Core accepted the denied draft: {core} {answer}"
    else:  # live finding, pinned: Core has no precondition notion, so the engine compile step is the ONLY guard
        assert core["outcome"] == "accepted", f"{neg.id}: Core now refuses it, update the BK0 matrix: {core}"
