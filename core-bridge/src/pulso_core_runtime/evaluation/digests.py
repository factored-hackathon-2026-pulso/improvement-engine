"""Canonical broker payload digests shared by L3b (tools) and L5 (evaluation). See ADR 0002.

Pure module on purpose (hashlib + the pinned canonical JSON only): importable from `tools/` without pulling
in psycopg or the rest of the evaluation package."""

from __future__ import annotations

import hashlib

from agent_core.domain import canonical_bytes


def native_evaluate_digest(*, proposal_id: str, evaluation_context_ref: str, candidate_hash: str, suite_id: str,
                           suite_version: str, suite_digest: str) -> str:
    """`payload_digest` of the broker `native_evaluate` authorization check (the one broad digest).

    Binds the proposal, the admission (`evaluation_context_ref`), the frozen candidate and the exact suite
    (id, version, content digest). Field names are part of the contract: changing them changes every digest."""
    return hashlib.sha256(canonical_bytes({
        "proposal_id": proposal_id, "candidate_hash": candidate_hash, "suite_id": suite_id,
        "suite_version": suite_version, "suite_digest": suite_digest,
        "evaluation_context_ref": evaluation_context_ref})).hexdigest()
