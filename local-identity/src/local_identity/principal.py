"""Request DTOs, the operation policy and the Core human `Principal` built for a command authorization.

The binding (operation, target, challenge, command) is echoed in `attrs` (string values only) plus a
`binding_digest`; Core ignores these extra attrs, the Codex `HumanAuthorizationPort` recomputes the digest and
compares it with its durable intention. The Principal itself stays exactly the Core shape (no new claims)."""

from __future__ import annotations

import hashlib
import json
from datetime import UTC, datetime
from typing import Annotated, Literal

from pydantic import BaseModel, ConfigDict, Field, StrictInt

Ref = Annotated[str, Field(pattern=r"^[A-Za-z0-9._:\-]{1,200}$")]
Nonce = Annotated[str, Field(pattern=r"^[A-Za-z0-9._:\-]{16,200}$")]
Sha256Hex = Annotated[str, Field(pattern=r"^[0-9a-f]{64}$")]
Revision = Annotated[StrictInt, Field(ge=0)]

# operation -> (target kind, role the actor must hold)
POLICY: dict[str, tuple[str, str]] = {
    "approve": ("proposal", "aprobador"),
    "reject": ("proposal", "aprobador"),
    "publish": ("proposal", "aprobador"),
    "promote": ("release", "aprobador"),
    "revoke": ("release", "admin"),
}
BASE_ROLES = ("constructor", "aprobador")


class _Strict(BaseModel):
    model_config = ConfigDict(extra="forbid", frozen=True)


class ProposalTarget(_Strict):
    kind: Literal["proposal"] = "proposal"
    proposal_id: Ref
    candidate_hash: Sha256Hex
    expected_revision: Revision


class ReleaseTarget(_Strict):
    kind: Literal["release"] = "release"
    release_id: Ref
    agent_id: Ref
    alias: Ref
    expected_revision: Revision


class SessionRequest(_Strict):
    tenant_id: Ref
    actor_ref: Ref
    session_intent_ref: Ref
    nonce: Nonce


class CommandRequest(_Strict):
    tenant_id: Ref
    actor_ref: Ref
    command_ref: Ref
    operation: str
    target: ProposalTarget | ReleaseTarget
    challenge_ref: Ref
    nonce: Nonce


def iso_z(moment: datetime) -> str:
    return moment.astimezone(UTC).strftime("%Y-%m-%dT%H:%M:%S.%f")[:-3] + "Z"


def binding_of(req: CommandRequest) -> dict[str, object]:
    """Everything the approval is bound to (nonce excluded: it is a one-use transport value)."""
    return {
        "tenant_id": req.tenant_id,
        "actor_ref": req.actor_ref,
        "command_ref": req.command_ref,
        "operation": req.operation,
        "challenge_ref": req.challenge_ref,
        "target": req.target.model_dump(mode="json"),
    }


def binding_digest(binding: dict[str, object]) -> str:
    canonical = json.dumps(binding, sort_keys=True, separators=(",", ":"), ensure_ascii=True)
    return hashlib.sha256(canonical.encode()).hexdigest()


def human_principal(req: CommandRequest, *, roles: list[str], now: datetime, exp: datetime) -> dict[str, object]:
    target = req.target.model_dump(mode="json", exclude={"kind"})
    attrs = {
        "actor": "human",
        "tenant": req.tenant_id,
        "issuer": "local-identity",
        "command_ref": req.command_ref,
        "operation": req.operation,
        "challenge_ref": req.challenge_ref,
        "target_kind": req.target.kind,
        "binding_digest": binding_digest(binding_of(req)),
        **{k: str(v) for k, v in target.items()},
    }
    return {
        "type": "builder",
        "id": req.actor_ref,
        "roles": roles,
        "scopes": [],
        "attrs": attrs,
        "auth": {"level": "step_up", "at": iso_z(now), "simulated": True},
        "exp": iso_z(exp),
    }
