"""Pure-read authoring service (V3 CAP-08, CAP-16 L2, CAP-17).

* `alias_state` reads `RegistryService.get_alias` (upstream N-02: pure read, no proposal, no quota).
* `dry_run` runs the SAME code path as Core's `freeze` (`build_candidate` + `validate_candidate`, plus the
  `put_draft` pre-checks `check_draft_limits` / `platform_edits`) inside a read-only store transaction, so it never
  creates a proposal, event, version or quota row. The result is bound to the JCS sha256 of the request body.
  Deterministic: the output depends only on (body, store state), never on a clock or an id source."""

from __future__ import annotations

from collections.abc import Callable, Sequence
from datetime import UTC, datetime
from typing import Any, Literal

import pydantic
from agent_core.flows import Violation
from agent_core.registry.candidate import CandidateError, build_candidate
from agent_core.registry.entities import decode_entity
from agent_core.registry.errors import RegistryError
from agent_core.registry.models import RELEASE_SETTINGS, EntityDraft
from agent_core.registry.suite import EvalSuite
from agent_core.registry.validation import (
    DEFAULT_LIMITS,
    Limits,
    check_draft_limits,
    platform_edits,
    validate_candidate,
)
from pydantic import BaseModel, ConfigDict, Field

from pulso_core_runtime.invoke.errors import BridgeError
from pulso_core_runtime.invoke.models import request_digest

ALIASES = frozenset({"staging", "prod"})
SCHEMA_VERSION = "1"
Problems = list[dict[str, str | None]]


class ChangeIn(BaseModel):
    model_config = ConfigDict(extra="forbid")

    kind: str = Field(min_length=1, max_length=64)
    content: dict[str, Any]
    docs: dict[str, Any]


class DryRunRequest(BaseModel):
    """`CoreAuthoringDryRunRequest` (platform-sim/bridge_mock/schemas). The number of changes is deliberately NOT
    bounded here: exceeding Core's 50 is a `REG-LIMIT` violation (the caller needs the same report as `put_draft`)."""

    model_config = ConfigDict(extra="forbid")

    schema_version: Literal["1"]
    tenant_id: str = Field(min_length=1, max_length=255)
    agent_id: str = Field(min_length=1, max_length=255)
    base_release_id: str | None = Field(max_length=255)
    changes: list[ChangeIn]
    request_digest: str | None = Field(default=None, pattern=r"^[0-9a-f]{64}$")


def _violation(v: Violation) -> dict[str, str | None]:
    return {"rule": v.rule, "path": v.path, "flow": v.flow, "node_id": v.node_id, "message": v.message}


def _v(rule: str, path: str | None, message: str) -> dict[str, str | None]:
    return {"rule": rule, "path": path, "flow": None, "node_id": None, "message": message}


class AuthoringService:
    def __init__(self, store: Any, registry_service: Any, *, runtime_profile: str,
                 limits: Limits = DEFAULT_LIMITS, now: Callable[[], datetime] = lambda: datetime.now(UTC)) -> None:
        self._store, self._service = store, registry_service
        self._profile, self._limits, self._now = runtime_profile, limits, now

    # -- CAP-08 ---------------------------------------------------------------------------------------
    def alias_state(self, agent_id: str, alias: str) -> dict[str, Any]:
        if alias not in ALIASES or not 0 < len(agent_id) <= 255:
            raise BridgeError("pulso:invalid_request", 422)
        try:
            state = self._service.get_alias(agent_id, alias)
        except RegistryError:  # unknown agent, unset alias or dangling release: one indistinguishable answer
            raise BridgeError("pulso:alias_unknown", 404) from None
        except Exception:
            raise BridgeError("pulso:core_unavailable", 503, retryable=True) from None
        return {"schema_version": SCHEMA_VERSION, "agent_id": state.agent_id, "alias": state.alias,
                "release_id": state.release_id, "status": state.status, "source": "core_store",
                "observed_at": self._now().astimezone(UTC).strftime("%Y-%m-%dT%H:%M:%SZ"),
                "runtime_profile": self._profile}

    # -- CAP-16 L2 / CAP-17 -----------------------------------------------------------------------------
    def dry_run(self, claim_tenant: str, raw: Any) -> dict[str, Any]:
        try:
            req = DryRunRequest.model_validate(raw)
        except pydantic.ValidationError:
            raise BridgeError("pulso:invalid_request", 422) from None
        if req.tenant_id != claim_tenant:
            raise BridgeError("pulso:tenant_mismatch", 403)
        digest = request_digest(raw)
        if req.request_digest is not None and req.request_digest != digest:
            raise BridgeError("pulso:invalid_request", 422, details={"fields": ["request_digest"]})
        if any(c.kind == RELEASE_SETTINGS for c in req.changes):  # CAP-23 / N-07: default deny
            raise BridgeError("pulso:release_settings_not_allowed", 422)
        try:
            return self._evaluate(req, digest)
        except BridgeError:
            raise
        except Exception:
            raise BridgeError("pulso:dry_run_unavailable", 503, retryable=True) from None

    def _result(self, digest: str, violations: Sequence[dict[str, str | None]], **ok: Any) -> dict[str, Any]:
        valid = not violations
        return {"schema_version": SCHEMA_VERSION, "valid": valid,
                "candidate_hash": ok.get("candidate_hash") if valid else None,
                "release_hash": ok.get("release_hash") if valid else None,
                "release_id_preview": ok.get("release_id_preview") if valid else None,
                "new_versions": ok.get("new_versions", []) if valid else [],
                "auto_bumped": ok.get("auto_bumped", []) if valid else [],
                "content_hashes": ok.get("content_hashes", {}) if valid else {},
                "violations": list(violations), "request_digest": digest, "proposal_created": False,
                "runtime_profile": self._profile}

    def _evaluate(self, req: DryRunRequest, digest: str) -> dict[str, Any]:
        drafts: list[EntityDraft] = []
        problems: Problems = []
        for i, change in enumerate(req.changes):
            try:
                drafts.append(EntityDraft.model_validate(change.model_dump()))
            except pydantic.ValidationError:
                problems.append(_v("REG-SCHEMA", f"changes/{i}", "the change is not a valid EntityDraft "
                                   "(kind, content with text `id` and `version`, docs)"))
        if problems:
            return self._result(digest, problems)
        limit = check_draft_limits(drafts, self._limits)  # same pre-checks as `put_draft`
        if limit:
            return self._result(digest, [_violation(v) for v in limit])
        edits = platform_edits(drafts)
        if edits:
            return self._result(digest, [_v("REG-PLATFORM-EDIT", path, "platform guardrails are not edited by a "
                                            "proposal") for path in edits])

        with self._store.transaction() as tx:  # read-only: nothing below writes
            base, entities = None, []
            if req.base_release_id is not None:
                stored = tx.get_release(req.base_release_id)
                if stored is None or stored.agent_id != req.agent_id:
                    raise BridgeError("pulso:base_release_unknown", 404)
                base = stored.release
                loaded = [decode_entity(r.kind, tx.blobs.get(tx.get_version(r).content_hash))
                          for r in tx.release_refs(req.base_release_id)]
                entities = [e for e in loaded if not isinstance(e, EvalSuite)]

            def published(ref: Any) -> str | None:
                stored_version = tx.get_version(ref)
                return stored_version.content_hash if stored_version else None

            try:
                cand = build_candidate(agent_id=req.agent_id, base=base, base_entities=entities, drafts=drafts,
                                       published_hash=published)
                base_versions = ({(k.value, i): v for k, by in base.entities.items() for i, v in by.items()}
                                 if base else {})
                found = validate_candidate(cand, base_versions=base_versions,
                                           drafted={(d.kind, d.id) for d in drafts}, limits=self._limits)
                if found:
                    raise CandidateError(found)
            except CandidateError as exc:
                return self._result(digest, [_violation(v) for v in exc.violations])
        return self._result(
            digest, [], candidate_hash=cand.candidate_hash, release_hash=cand.release_hash,
            release_id_preview="rel-" + cand.candidate_hash[:16],
            new_versions=[r.model_dump(mode="json") for r in cand.new_versions],
            auto_bumped=[r.model_dump(mode="json") for r in cand.auto_bumped],
            content_hashes={str(ref): h for ref, h in sorted(cand.content_hashes.items(), key=lambda kv: str(kv[0]))})
