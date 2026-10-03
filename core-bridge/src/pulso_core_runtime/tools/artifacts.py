"""`pulso/artifact_get` (read): sealed artifact by reference (D.3 `GET /artifacts/{id}`)."""

from __future__ import annotations

import hashlib
import re
from typing import Any

from agent_core.domain.json import canonical_bytes
from agent_core.domain.shared import ToolStatus

from pulso_core_runtime.tools._common import Args, Deps, Outcome, err, ok, text_arg
from pulso_core_runtime.tools.context import InvocationContext

MAX_ARTIFACT_BYTES = 1 << 20  # broker artefact 1 MiB (D.3)
_HEX64 = re.compile(r"^[0-9a-f]{64}$")


def _digest_of(body: dict[str, Any]) -> str | None:
    if body.get("encoding") == "json":
        return hashlib.sha256(canonical_bytes(body.get("content"))).hexdigest()
    if body.get("encoding") == "utf8" and isinstance(body.get("content"), str):
        return hashlib.sha256(body["content"].encode("utf-8")).hexdigest()
    return None


def artifact_get(deps: Deps, ic: InvocationContext, args: Args, run_id: str) -> Outcome:
    artifact_ref = text_arg(args, "artifact_ref")
    body = deps.broker.artifact_get(ic.binding_ref, artifact_ref)
    meta = body.get("artifact")
    if not isinstance(meta, dict) or "content" not in body:
        return err("pulso:artifact_malformed")
    if meta.get("final_locked") or "final_locked" in str(meta.get("media_type", "")):
        return err("pulso:artifact_final_locked", ToolStatus.denied)  # never for a builder
    byte_length = body.get("byte_length")
    if not isinstance(byte_length, int) or byte_length > MAX_ARTIFACT_BYTES:
        return err("pulso:artifact_too_large")
    declared = str(meta.get("digest", "")).removeprefix("sha256:")
    if _HEX64.match(declared) and _digest_of(body) != declared:
        return err("pulso:artifact_digest_mismatch")
    deps.contexts.record_refs(ic.binding_ref, [artifact_ref, str(meta.get("id", ""))])
    digest = str(meta.get("digest", "")).removeprefix("sha256:") or None
    media = meta.get("media_type") if isinstance(meta.get("media_type"), str) else None
    for rid in {artifact_ref, str(meta.get("id", ""))}:
        deps.contexts.record_artifact(ic.binding_ref, rid, digest=digest, media_type=media)
    return ok({"artifact": {"id": meta.get("id"), "digest": digest,
                            "media_type": meta.get("media_type")},
               "encoding": body.get("encoding"), "content": body["content"], "byte_length": byte_length})
