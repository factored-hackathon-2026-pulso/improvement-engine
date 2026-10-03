"""Exact-bytes NDJSON chain artifact (plan 16.15.3 / Q12).

Material = the unmodified stored single-line `event_json` of seq 0..N, LF between lines and one final LF; no BOM,
no blank lines, no CRLF rewriting. A multiline stored serialisation fails preparation (never silently rewritten).
Digest = SHA256 of the raw bytes (not JCS). Artifacts are <= 1 MiB; larger chains are complete-line chunks plus an
immutable manifest whose own digest is SHA256-JCS while `material_digest` is the raw digest of the concatenation."""

from __future__ import annotations

import hashlib
from dataclasses import dataclass
from typing import Any

from agent_core.domain import canonical_bytes


class MaterialError(ValueError):
    """Preparation failure: unsupported serialisation or limits exceeded (the receipt stays unknown)."""


@dataclass(frozen=True)
class Chunk:
    from_seq: int
    to_seq: int
    data: bytes

    @property
    def digest(self) -> str:
        return sha256_raw(self.data)


def sha256_raw(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def jcs_digest(value: Any) -> str:
    return hashlib.sha256(canonical_bytes(value)).hexdigest()


def line_for(event_json: str) -> bytes:
    if "\n" in event_json or "\r" in event_json or event_json.startswith("﻿") or not event_json:
        raise MaterialError("unsupported multiline or empty native serialisation")
    return event_json.encode("utf-8")


def build_ndjson(event_jsons: list[str]) -> bytes:
    return b"".join(line_for(j) + b"\n" for j in event_jsons)


def plan_chunks(event_jsons: list[str], max_bytes: int, max_total: int) -> list[Chunk]:
    chunks: list[Chunk] = []
    buf = bytearray()
    start = 0
    total = 0
    for i, j in enumerate(event_jsons):
        line = line_for(j) + b"\n"
        if len(line) > max_bytes:
            raise MaterialError("single event larger than the artifact limit")
        total += len(line)
        if total > max_total:
            raise MaterialError("material exceeds the verification limit")
        if len(buf) + len(line) > max_bytes:
            chunks.append(Chunk(start, i - 1, bytes(buf)))
            buf, start = bytearray(), i
        buf += line
    if buf:
        chunks.append(Chunk(start, len(event_jsons) - 1, bytes(buf)))
    return chunks


def manifest_for(chunks: list[Chunk], refs: list[str]) -> dict[str, Any]:
    material = b"".join(c.data for c in chunks)
    return {"media_type": "application/x-ndjson",
            "chunks": [{"ref": r, "digest": c.digest, "from_seq": c.from_seq, "to_seq": c.to_seq,
                        "bytes": len(c.data)} for c, r in zip(chunks, refs, strict=True)],
            "total_events": chunks[-1].to_seq + 1 if chunks else 0, "total_bytes": len(material),
            "material_digest": sha256_raw(material)}
