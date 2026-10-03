"""`TargetLoader.load(target) -> LoadedTarget{eval_target, commitment}` (plan 17.3.5 Targets).

Both kinds build a `SnapshotRegistry`; a live `PostgresRegistry` is never returned, a candidate is never
published and a mismatch never falls back to the base. Any problem -> `TargetError("target_preparation_failed")`
which the arm runner maps to `failed_infra`."""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any

from agent_core.registry import EvalTarget, RegistryError, RegistryStore, SnapshotRegistry
from agent_core.registry.candidate import build_candidate
from agent_core.registry.entities import decode_entity
from agent_core.registry.suite import EvalSuite
from agent_core.registry.validation import DEFAULT_LIMITS, validate_candidate

from pulso_core_runtime.evaluation.report import digest_of

CODE = "target_preparation_failed"


class TargetError(Exception):
    def __init__(self, reason: str) -> None:
        super().__init__(reason)
        self.code, self.reason = CODE, reason


@dataclass(frozen=True)
class LoadedTarget:
    eval_target: EvalTarget
    commitment: str  # sha256 over what was loaded; compared with the request's `target_commitment`


def drafts_digest(drafts: list[Any]) -> str:
    """`sha256(JCS(sorted-by-(kind,id) model_dump(mode="json")))` of the draft plan."""
    ordered = sorted(drafts, key=lambda d: (d.kind, str(d.content.get("id", ""))))
    return digest_of([d.model_dump(mode="json") for d in ordered])


class TargetLoader:
    def __init__(self, store: RegistryStore) -> None:
        self._store = store

    def load(self, target: dict[str, Any]) -> LoadedTarget:
        kind = target.get("kind")
        try:
            if kind == "published_release":
                return self._published(str(target["release_id"]))
            if kind == "frozen_candidate":
                return self._frozen(target)
        except TargetError:
            raise
        except (KeyError, ValueError, TypeError, RegistryError):
            raise TargetError("closure_invalid") from None
        raise TargetError("unknown_target_kind")

    def _published(self, release_id: str) -> LoadedTarget:
        with self._store.transaction() as tx:
            stored = tx.get_release(release_id)
            if stored is None or tx.release_status(release_id) != "active":
                raise TargetError("release_not_active")
            entities = [decode_entity(r.kind, tx.blobs.get(tx.get_version(r).content_hash))  # type: ignore[union-attr]
                        for r in tx.release_refs(release_id)]
        entities = [e for e in entities if not isinstance(e, EvalSuite)]
        rel = stored.release
        return LoadedTarget(EvalTarget("base", rel, SnapshotRegistry(rel, entities)),
                            digest_of({"kind": "published_release", "release_id": release_id,
                                       "release_hash": stored.release_hash}))

    def _frozen(self, t: dict[str, Any]) -> LoadedTarget:
        with self._store.transaction() as tx:
            p = tx.get_proposal(t["proposal_id"], for_update=False)
            if p is None or p.rev != t["expected_rev"] or p.state.value not in ("candidate", "evaluated"):
                raise TargetError("proposal_not_frozen")
            if p.base_release_id != t.get("base_release_id") or p.candidate_hash != t["candidate_hash"]:
                raise TargetError("proposal_mismatch")
            drafts = tx.get_changes(p.proposal_id)
            if drafts_digest(drafts) != t["draft_plan_digest"]:
                raise TargetError("draft_plan_digest_mismatch")
            base, base_entities = None, []
            if p.base_release_id is not None:
                stored = tx.get_release(p.base_release_id)
                if stored is None:
                    raise TargetError("base_release_missing")
                base = stored.release
                loaded = [decode_entity(r.kind, tx.blobs.get(tx.get_version(r).content_hash))  # type: ignore[union-attr]
                          for r in tx.release_refs(p.base_release_id)]
                base_entities = [e for e in loaded if not isinstance(e, EvalSuite)]

            def published(ref: Any) -> str | None:
                v = tx.get_version(ref)
                return v.content_hash if v else None

            cand = build_candidate(agent_id=p.agent_id, base=base, base_entities=base_entities, drafts=drafts,
                                   published_hash=published)
            base_versions = ({(k.value, i): v for k, by in base.entities.items() for i, v in by.items()}
                             if base else {})
            if validate_candidate(cand, base_versions=base_versions, drafted={(d.kind, d.content.get("id")) for d in drafts},  # type: ignore[arg-type]
                                  limits=DEFAULT_LIMITS):
                raise TargetError("candidate_invalid")
        if cand.candidate_hash != t["candidate_hash"]:
            raise TargetError("candidate_hash_mismatch")
        return LoadedTarget(EvalTarget("candidate", cand.release, SnapshotRegistry(cand.release, cand.entities)),
                            digest_of({"kind": "frozen_candidate", "candidate_hash": cand.candidate_hash,
                                       "draft_plan_digest": t["draft_plan_digest"],
                                       "base_release_id": t.get("base_release_id")}))
