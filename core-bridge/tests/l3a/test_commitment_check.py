"""`sealed_commitment_check`: adopted registry writes are verified against the commitment sealed in the
invocation context (operations order, proposal, request hashes) before they are adopted. Pure, no DB."""

from __future__ import annotations

import hashlib
from dataclasses import dataclass, field
from types import SimpleNamespace
from typing import Any

from agent_core.domain.json import canonical_bytes

from pulso_core_runtime.adapters import sealed_commitment_check
from pulso_core_runtime.tools.builder import eval_key, write_key

CMD, STAGE = "cmd-1", "writer"
CREATE = {"agent_id": "agent-x", "origin": "manual", "title": "pulso-key:cmd-1"}


def rh(op: str, payload: Any) -> str:
    return hashlib.sha256(canonical_bytes({"op": op, "payload": payload})).hexdigest()


@dataclass
class Store:
    commitment: dict[str, Any] | None
    operations: list[str] = field(default_factory=lambda: ["create_proposal", "put_draft", "freeze"])

    def context_row(self, ref: str) -> dict[str, Any] | None:
        ctx: dict[str, Any] = {"operations": self.operations}
        if self.commitment is not None:
            ctx["commitment"] = self.commitment
        return {"context": ctx}


def sealed(**over: Any) -> dict[str, Any]:
    base: dict[str, Any] = {
        "mode": "write", "proposal_id": None, "expected_rev": None, "base_release_id": None,
        "evaluate_enabled": True, "evaluation_context_ref": "ctx-1", "create_agent_id": "agent-x",
        "create_origin": "manual", "create_title": "pulso-key:cmd-1", "put_draft_digest": "a" * 64,
        "operations": ["create_proposal", "put_draft", "freeze"]}
    return {**base, **over}


RECEIPT = SimpleNamespace(task_binding_ref="bind-1", idempotency_key=CMD, stage=STAGE)
K = [write_key(CMD, STAGE, n) for n in range(3)]
PRESENT: dict[str, dict[str, Any]] = {}


def writes(**records: dict[str, Any]) -> Any:
    table = {K[int(i)]: r for i, r in ((k.removeprefix("w"), v) for k, v in records.items())}
    return lambda key: table.get(key)


def check(commitment: dict[str, Any] | None, key: str, found: dict[str, Any], probe: Any = None) -> bool:
    fn = sealed_commitment_check(Store(commitment), probe or (lambda k: None))
    return fn(RECEIPT, key, found)


def create_record(pid: str = "p1") -> dict[str, Any]:
    return {"op": "create_proposal", "proposal_id": pid, "rev_after": 0, "request_hash": rh("create_proposal", CREATE)}


def test_matching_create_proposal_is_verified() -> None:
    assert check(sealed(), K[0], create_record())


def test_create_with_other_title_agent_or_origin_is_rejected() -> None:
    for tampered in ({**CREATE, "title": "other"}, {**CREATE, "agent_id": "agent-y"}, {**CREATE, "origin": "auto"}):
        found = {**create_record(), "request_hash": rh("create_proposal", tampered)}
        assert not check(sealed(), K[0], found)


def test_wrong_op_for_the_committed_ordinal_is_rejected() -> None:
    found = {"op": "freeze", "proposal_id": "p1", "rev_after": 1, "request_hash": rh("freeze", {"proposal_id": "p1"})}
    assert not check(sealed(), K[0], found)  # ordinal 0 is create_proposal


def test_put_draft_binds_proposal_and_revision_when_known() -> None:
    commitment = sealed(proposal_id="p9", expected_rev=3, operations=["put_draft"])
    ok = {"op": "put_draft", "proposal_id": "p9", "rev_after": 4, "request_hash": "x" * 64}
    assert check(commitment, K[0], ok)
    assert not check(commitment, K[0], {**ok, "proposal_id": "p-other"})
    assert not check(commitment, K[0], {**ok, "rev_after": 3})  # did not advance past the committed revision


def test_fresh_proposal_later_writes_must_belong_to_the_proposal_the_create_made() -> None:
    probe = writes(w0=create_record("p1"))
    freeze = {"op": "freeze", "proposal_id": "p1", "rev_after": 2, "request_hash": rh("freeze", {"proposal_id": "p1"})}
    assert check(sealed(), K[2], freeze, probe)
    other = {**freeze, "proposal_id": "p2", "request_hash": rh("freeze", {"proposal_id": "p2"})}
    assert not check(sealed(), K[2], other, probe)  # a different proposal than the committed create produced
    assert not check(sealed(), K[2], freeze, lambda k: None)  # create not readable: unverifiable


def test_freeze_with_foreign_request_hash_is_rejected() -> None:
    found = {"op": "freeze", "proposal_id": "p1", "rev_after": 2, "request_hash": "0" * 64}
    assert not check(sealed(proposal_id="p1"), K[2], found)


def test_evaluate_key_needs_an_enabled_evaluation_and_the_committed_proposal() -> None:
    found = {"op": "evaluate", "proposal_id": "p1", "rev_after": 2, "request_hash": "e" * 64}
    assert check(sealed(proposal_id="p1"), eval_key("ctx-1"), found)
    assert not check(sealed(proposal_id="p1", evaluate_enabled=False), eval_key("ctx-1"), found)
    assert not check(sealed(proposal_id="p1"), eval_key("ctx-1"), {**found, "proposal_id": "p2"})
    assert not check(sealed(proposal_id="p1"), eval_key("ctx-1"), {**found, "op": "put_draft"})


def test_evaluate_only_mode_allows_no_mutation() -> None:
    commitment = sealed(mode="evaluate_only", proposal_id="p1", operations=[])
    assert not check(commitment, K[0], create_record())
    assert check(commitment, eval_key("ctx-1"), {"op": "evaluate", "proposal_id": "p1", "rev_after": 1,
                                                  "request_hash": "e" * 64})


def test_unknown_key_missing_commitment_or_garbage_fail_closed() -> None:
    assert not check(sealed(), "pulso-write:unrelated", create_record())
    assert not check(None, K[0], create_record())  # legacy context without a sealed commitment
    assert not check(sealed(), K[0], {})
    assert not check(sealed(), K[0], {"op": "create_proposal"})
