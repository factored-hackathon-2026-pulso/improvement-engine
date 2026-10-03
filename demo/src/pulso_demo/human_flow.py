"""Demo steps 8-9: human decision -> approve -> publish -> staging confirmed (prod only on the explicit promote).

Ports (so the flow is testable offline and the live wiring stays thin):
* `RegistryPort`   - Core registry reads/writes (live: `human_live.LiveRegistry`; offline: `offline_ports.OfflineRegistry`),
* `AuthorizerPort` - durable single-use intention + command-authorization credential (live: the Codex `HumanAuthorizationPort`
  stand-in over the local human issuer; offline: a labelled synthetic one),
* `Gate`           - WHO decides: `ScriptedGate` is a labelled SIMULATED human supervisor, `ManualGate` waits for a real person
  (`python -m pulso_demo.decide`).

Honesty rules: approval fixes operation/hash and is NOT publication (the flow verifies staging did not move at approval); publication is
only claimed after an alias READ; prod moves only with `promote=True`; no credential ever enters the trail (receipts are ids/hashes)."""

from __future__ import annotations

import json
import time
from collections.abc import Callable
from dataclasses import dataclass
from datetime import UTC, datetime
from pathlib import Path
from typing import Any, Protocol

from pulso_demo.decision_hook import DecisionRequest, DecisionResult

REQUEST_FILE = "human-request.json"
DECISION_FILE = "human-decision.json"
ACTOR = "local-supervisor"
AGENT = "pulso-scout"


class RegistryError(Exception):
    def __init__(self, status: int, code: str) -> None:
        super().__init__(f"{code} ({status})")
        self.status, self.code = status, code


class Bearer(Protocol):
    kid: str

    def reveal(self) -> str: ...


class RegistryPort(Protocol):
    def proposal(self, proposal_id: str) -> dict[str, Any]: ...  # {"state","rev","candidate_hash"}
    def alias(self, name: str) -> str | None: ...
    def approve(self, proposal_id: str, bearer: Bearer, candidate_hash: str) -> dict[str, Any]: ...
    def publish(self, proposal_id: str, bearer: Bearer, idempotency_key: str) -> str: ...
    def promote(self, release_id: str, bearer: Bearer) -> None: ...


class AuthorizerPort(Protocol):
    def open(self, operation: str, target: dict[str, Any]) -> dict[str, str]: ...  # durable intention -> {"intention_id","command_ref"}
    def authorize(self, intention_id: str) -> Bearer: ...


@dataclass(frozen=True)
class Choice:
    decision: str  # approve | reject | timeout
    reason: str | None = None


class Gate(Protocol):
    mode: str
    simulated: bool
    label: str

    def wait(self, request: dict[str, Any]) -> Choice: ...


class ScriptedGate:
    """A labelled SIMULATED human supervisor: decides immediately. Never presented as a person."""

    mode, simulated = "scripted", True
    label = "SIMULATED human supervisor (scripted demo mode, not a person)"

    def __init__(self, decision: str = "approve") -> None:
        self.decision = decision

    def wait(self, request: dict[str, Any]) -> Choice:
        return Choice(self.decision, "scripted")


class ManualGate:
    """Pauses the driver until a real person runs `python -m pulso_demo.decide --out <dir> approve|reject`. The decision file must name this
    proposal AND candidate hash (otherwise it is a rejection) and is consumed on read, so a stale approval can never be replayed."""

    mode, simulated = "manual", False
    label = "human supervisor (manual approval via CLI; identity still comes from the local sandbox issuer)"

    def __init__(self, out_dir: Path | str, *, timeout_s: float = 900.0, poll_s: float = 1.0, sleep: Callable[[float], None] = time.sleep) -> None:
        self.dir, self.timeout_s, self.poll_s, self._sleep = Path(out_dir), timeout_s, poll_s, sleep

    def wait(self, request: dict[str, Any]) -> Choice:
        req_path, dec_path = self.dir / REQUEST_FILE, self.dir / DECISION_FILE
        dec_path.unlink(missing_ok=True)  # a decision file that predates this request can never be a decision about it
        req_path.write_text(json.dumps({**request, "how": f"python -m pulso_demo.decide --out {self.dir} approve|reject"}, indent=1), "utf-8")
        deadline = time.monotonic() + self.timeout_s
        try:
            while True:
                if dec_path.exists():
                    try:
                        body = json.loads(dec_path.read_text("utf-8"))
                    except (OSError, ValueError):
                        body = None
                    if body is not None:
                        dec_path.unlink(missing_ok=True)
                        if body.get("proposal_id") != request["proposal_id"] or body.get("candidate_hash") != request["candidate_hash"]:
                            return Choice("reject", "decision_file_mismatch")
                        return Choice("approve" if body.get("decision") == "approve" else "reject", "manual")
                if time.monotonic() >= deadline:
                    return Choice("timeout", "no_decision_before_timeout")
                self._sleep(self.poll_s)
        finally:
            req_path.unlink(missing_ok=True)


def make_gate(mode: str, out_dir: Path | str, *, timeout_s: float = 900.0) -> Gate:
    if mode == "scripted":
        return ScriptedGate()
    if mode == "manual":
        return ManualGate(out_dir, timeout_s=timeout_s)
    raise ValueError(f"unknown human mode {mode!r}")


def _now() -> str:
    return datetime.now(UTC).strftime("%Y-%m-%dT%H:%M:%SZ")


class ApprovalFlow:
    def __init__(self, registry: RegistryPort, authorizer: AuthorizerPort, gate: Gate, *, promote: bool = False, agent_id: str = AGENT,
                 actor: str = ACTOR, on_event: Callable[[list[dict[str, Any]]], None] | None = None) -> None:
        self.reg, self.auth, self.gate, self.promote, self.agent, self.actor, self.on_event = registry, authorizer, gate, promote, agent_id, actor, on_event

    def run(self, req: DecisionRequest) -> DecisionResult:
        trail: list[dict[str, Any]] = []
        d: dict[str, Any] = {"mode": self.gate.mode, "simulated_human": self.gate.simulated, "actor": self.actor, "stage": "requested",
                             "proposal_id": req.proposal_id, "candidate_hash": req.candidate_hash, "release_id": None, "staging_alias": None,
                             "prod_alias": None, "promoted": False, "error": None, "reason": None, "trail": trail}

        def add(event: str, **kw: Any) -> None:
            trail.append({"event": event, "at": _now(), **kw})
            if self.on_event:
                self.on_event(trail)

        def done(state: str) -> DecisionResult:
            return DecisionResult(state=state, receipt_ref=d["release_id"], detail=d)  # type: ignore[arg-type]

        def fail(stage: str, err: str, event: str, state: str = "pending") -> DecisionResult:
            d.update(stage=stage, error=err)
            add(event, error=err)
            return done(state)

        try:
            p = self.reg.proposal(req.proposal_id)
            if p["state"] != "evaluated" or p["candidate_hash"] != req.candidate_hash:
                return fail("failed", f"candidate_changed_or_not_evaluated: state={p['state']}", "decision_refused")
            staging0, prod0 = self.reg.alias("staging"), self.reg.alias("prod")
            target = {"proposal_id": req.proposal_id, "candidate_hash": req.candidate_hash, "expected_revision": p["rev"]}
            it = self.auth.open("approve", target)
            add("decision_requested", operation="approve", proposal_id=req.proposal_id, candidate_hash=req.candidate_hash, proposal_rev=p["rev"],
                intention_id=it["intention_id"], command_ref=it["command_ref"], waiting_for=self.gate.label, staging_alias=staging0, prod_alias=prod0)
            choice = self.gate.wait({"proposal_id": req.proposal_id, "candidate_hash": req.candidate_hash, "operation": "approve",
                                     "intention_id": it["intention_id"]})
            if choice.decision == "timeout":
                d["reason"] = choice.reason
                add("human_timeout", reason=choice.reason)
                return done("pending")
            add("human_decided", decision=choice.decision, actor=self.actor, simulated=self.gate.simulated, label=self.gate.label, mode=self.gate.mode,
                reason=choice.reason)
            if choice.decision != "approve":
                d.update(stage="rejected", reason=choice.reason)
                return done("rejected")
            # ---- approve: the credential is bound to THIS intention (operation, hash, revision); approval is not publication
            bearer = self.auth.authorize(it["intention_id"])
            try:
                self.reg.approve(req.proposal_id, bearer, req.candidate_hash)
            except RegistryError as exc:
                return fail("failed", f"approve_failed:{exc.code}", "approve_failed")
            p2 = self.reg.proposal(req.proposal_id)
            if p2["state"] != "approved" or self.reg.alias("staging") != staging0 or self.reg.alias("prod") != prod0:
                return fail("failed", "approval_state_unexpected", "approve_failed")
            d["stage"] = "approved"
            add("approved", operation="approve", candidate_hash=req.candidate_hash, proposal_rev=p["rev"], intention_id=it["intention_id"],
                command_ref=it["command_ref"], actor=self.actor, credential_kid=bearer.kid, proposal_state=p2["state"], published=False,
                staging_alias=staging0, prod_alias=prod0)
            # ---- publish: a fresh intention for the new proposal revision
            pit = self.auth.open("publish", {"proposal_id": req.proposal_id, "candidate_hash": req.candidate_hash, "expected_revision": p2["rev"]})
            pbearer = self.auth.authorize(pit["intention_id"])
            idem = f"demo-publish-{req.proposal_id}-{req.candidate_hash[:12]}"
            try:
                release = self.reg.publish(req.proposal_id, pbearer, idem)
            except RegistryError as exc:
                d["stage"] = "approved_not_published"
                d["error"] = f"publish_failed:{exc.code}"
                add("publish_failed", error=d["error"], intention_id=pit["intention_id"])
                return done("approved")
            d["release_id"] = release
            add("published", operation="publish", release_id=release, idempotency_key=idem, intention_id=pit["intention_id"],
                command_ref=pit["command_ref"], credential_kid=pbearer.kid, staging_confirmed=False)
            staging, prod = self.reg.alias("staging"), self.reg.alias("prod")  # the 200 is not the proof: the alias read is
            if staging != release:
                d.update(stage="published_unconfirmed", error="staging_alias_not_moved", staging_alias=staging, prod_alias=prod)
                return done("approved")
            d.update(stage="staging_confirmed", staging_alias=staging, prod_alias=prod)
            add("staging_confirmed", release_id=release, staging_alias=staging, prod_alias=prod, prod_unchanged=prod == prod0)
            if self.promote:
                rit = self.auth.open("promote", {"release_id": release, "agent_id": self.agent, "alias": "prod", "expected_revision": 0})
                rbearer = self.auth.authorize(rit["intention_id"])
                try:
                    self.reg.promote(release, rbearer)
                except RegistryError as exc:
                    d["error"] = f"promote_failed:{exc.code}"
                    add("promote_failed", error=d["error"])
                    return done("approved")
                prod2 = self.reg.alias("prod")
                if prod2 != release:
                    d["error"] = "prod_alias_not_moved"
                    add("promote_failed", error=d["error"])
                    return done("approved")
                d.update(stage="promoted", promoted=True, prod_alias=prod2)
                add("promoted", operation="promote", release_id=release, prod_alias=prod2, intention_id=rit["intention_id"],
                    command_ref=rit["command_ref"], explicit=True)
            return done("approved")
        except Exception as exc:  # noqa: BLE001 - infrastructure failure: recorded, never turned into an approval
            err = f"{type(exc).__name__}: {exc}"[:200]
            if d["stage"] in ("approved", "approved_not_published", "published_unconfirmed", "staging_confirmed"):
                d["error"] = err
                add("flow_error", error=err)
                return done("approved")
            return fail("failed", err, "flow_error")


class FlowHook:
    """DecisionHook adapter over an `ApprovalFlow`."""

    def __init__(self, flow: ApprovalFlow) -> None:
        self.flow = flow

    def decide(self, request: DecisionRequest) -> DecisionResult:
        return self.flow.run(request)
