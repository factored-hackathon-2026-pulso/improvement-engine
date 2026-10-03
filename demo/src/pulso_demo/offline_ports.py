"""SYNTHETIC registry/authorizer for `--offline` (no stack): every id starts with `offline-`, nothing here touches Core or the issuer.
They exist so the console preview shows the whole approve -> publish -> staging path; the report labels it `offline_human`/`offline_core`."""

from __future__ import annotations

import itertools
from typing import Any

from pulso_demo.human_flow import RegistryError


class OfflineBearer:
    def __init__(self, jws: str) -> None:
        self._jws, self.kid = jws, "offline-human-kid"

    def reveal(self) -> str:
        return self._jws

    def __repr__(self) -> str:
        return "OfflineBearer(<redacted>)"


class OfflineAuthorizer:
    def __init__(self) -> None:
        self.intentions: list[dict[str, Any]] = []
        self.issued: list[str] = []
        self._used: set[str] = set()
        self._n = itertools.count(1)

    def open(self, operation: str, target: dict[str, Any]) -> dict[str, str]:
        i = next(self._n)
        self.intentions.append({"intention_id": f"offline-int-{i}", "operation": operation, "target": dict(target)})
        return {"intention_id": f"offline-int-{i}", "command_ref": f"offline-cmd-{i}"}

    def authorize(self, intention_id: str) -> OfflineBearer:
        if intention_id in self._used:
            raise RuntimeError("intention already consumed")
        self._used.add(intention_id)
        jws = f"offline.jws.{intention_id}"
        self.issued.append(jws)
        return OfflineBearer(jws)


class OfflineRegistry:
    def __init__(self, proposal_id: str, candidate_hash: str, *, base_release: str = "offline-release-base", fail_publish: bool = False,
                 ignore_alias_move: bool = False) -> None:
        self.pid, self.hash, self.fail_publish, self.ignore = proposal_id, candidate_hash, fail_publish, ignore_alias_move
        self.state, self.rev = "evaluated", 3
        self.aliases: dict[str, str | None] = {"staging": base_release, "prod": base_release}
        self.calls: list[tuple[str, ...]] = []
        self.staging_at_approve: str | None = None
        self.approved_hash: str | None = None
        self.release: str | None = None

    def proposal(self, proposal_id: str) -> dict[str, Any]:
        self.calls.append(("proposal", proposal_id))
        return {"state": self.state, "rev": self.rev, "candidate_hash": self.hash}

    def alias(self, name: str) -> str | None:
        self.calls.append(("alias", name))
        return self.aliases[name]

    def approve(self, proposal_id: str, bearer: Any, candidate_hash: str) -> dict[str, Any]:
        self.calls.append(("approve", proposal_id))
        if candidate_hash != self.hash:
            raise RegistryError(409, "candidate_changed")
        if self.state != "evaluated":
            raise RegistryError(409, "illegal_transition")
        self.staging_at_approve, self.approved_hash = self.aliases["staging"], candidate_hash
        self.state, self.rev = "approved", self.rev + 1
        return {"decision": "approved", "candidate_hash": candidate_hash}

    def publish(self, proposal_id: str, bearer: Any, idempotency_key: str) -> str:
        self.calls.append(("publish", proposal_id))
        if self.fail_publish:
            raise RegistryError(500, "internal")
        self.release = f"offline-release-{self.hash[:8]}"
        if not self.ignore:
            self.aliases["staging"] = self.release
        return self.release

    def promote(self, release_id: str, bearer: Any) -> None:
        self.calls.append(("promote", release_id))
        self.aliases["prod"] = release_id
