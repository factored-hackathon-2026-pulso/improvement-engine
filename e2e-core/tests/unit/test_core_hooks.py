"""INT0: real-Core CoreHooks (dry_run, alias_read) over the bridge client; unit-tested against a fake bridge."""
import json
from types import SimpleNamespace

import pytest

from claude_standin import compile_step as C
from claude_standin import core_hooks as H
from claude_standin import thread01 as T
from test_cmppy_compile import PROMPT_REF, SUITE_REF, WORLD, add, rep


class FakeBridge:
    def __init__(self, valid=True, alias_status=200):
        self.calls, self.valid, self.alias_status = [], valid, alias_status

    def call(self, method, path, op, tenant, json=None, **kw):
        self.calls.append((method, path, op, tenant, json))
        if op == "alias_read" or op == "aliases":
            return SimpleNamespace(status_code=self.alias_status, text="",
                                   json=lambda: {"release_id": "rel-base", "alias": path.rsplit("/", 1)[-1]})
        body = ({"valid": True, "candidate_hash": "ab" * 32, "violations": []} if self.valid else
                {"valid": False, "candidate_hash": None, "violations": [{"rule": "REG-X", "message": "no"}]})
        return SimpleNamespace(status_code=200, text="", json=lambda: body)


def test_dry_run_builds_entity_drafts_from_the_world_and_returns_the_candidate_hash():
    b = FakeBridge()
    hook = H.make_dry_run(b, WORLD, tenant="t1", agent_id="atencion", base_release_id="rel-base")
    ops = [{k: o[k] for k in ("op", "target_kind", "target_ref", "new_ref", "precondition_digest")} for o in (rep(), add())]
    assert hook(ops) == "sha256:" + "ab" * 32
    method, path, op, tenant, body = b.calls[0]
    assert (method, path, op, tenant) == ("POST", "/core-authoring/dry-run", "dry_run", "t1")
    assert body["agent_id"] == "atencion" and body["base_release_id"] == "rel-base" and body["schema_version"] == "1"
    kinds = [c["kind"] for c in body["changes"]]
    assert kinds == ["prompt", "eval_suite"]
    assert body["changes"][0]["content"]["id"] == "p/resumen_radicado" and body["changes"][0]["content"]["version"] == "2.0.0"
    assert body["changes"][1]["content"]["version"] == "2.0.0"


def test_dry_run_refusal_raises_with_the_core_violations():
    hook = H.make_dry_run(FakeBridge(valid=False), WORLD, tenant="t1", agent_id="atencion", base_release_id=None)
    with pytest.raises(RuntimeError, match="REG-X"):
        hook([rep()])


def test_alias_read_returns_release_and_alias():
    b = FakeBridge()
    read = H.make_alias_read(b, tenant="t1", agent_id="atencion")
    assert read(None, "staging") == {"release_id": "rel-base", "alias": "staging"}
    assert b.calls[0][:3] == ("GET", "/core-state/aliases/atencion/staging", "aliases")


def test_alias_read_http_error_raises():
    with pytest.raises(RuntimeError, match="404"):
        H.make_alias_read(FakeBridge(alias_status=404), tenant="t1", agent_id="atencion")(None, "staging")


def test_dry_run_hook_flips_step_5_to_real_narrow_in_the_thread(tmp_path):
    hook = H.make_dry_run(FakeBridge(), WORLD, tenant="t1", agent_id="atencion", base_release_id="rel-base")
    cfg = T.ThreadConfig(workdir=tmp_path / "w", exe="D:/cargo-targets/claude-ed0/debug/improvement-engine.exe",
                         queue_dir=T.ROOT / "e2e-core/tests/fixtures/thread01_queue",
                         hooks=T.CoreHooks(dry_run=hook))
    steps = {s["n"]: s for s in T.run_thread(cfg)["steps"]}
    assert steps[5]["status"] == "real-narrow" and steps[5]["detail"]["draft_plan"]["digest"] == "sha256:" + "ab" * 32
    assert steps[6]["status"] == "stand-in" and steps[9]["status"] == "stand-in"
