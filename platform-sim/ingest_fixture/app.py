"""In-memory ingest fixture implementing plan 16.13.3 / 16.13.5 / 16.15.3 (CL-0008 P5-P7) server semantics.

DOUBLE: not Codex's real control-api. It enforces the agreed transport rules so the exporter can be tested:
unknown fields rejected, JCS batch digest recomputed server-side, Idempotency-Key == batch_digest, 202 only after
commit, 200 duplicate returns the committed receipt without re-applying a checkpoint, fast_poll CAS on
expected_cursor_revision (stale -> 409 before any write), rescan never advances the checkpoint, event ledger dedup
identity (tenant, source_id, kind, level, native_event_id), source digest conflict -> 409, 1 MiB artifacts."""

from __future__ import annotations

import hashlib
import re
import threading
from dataclasses import dataclass, field
from typing import Any, Literal

import rfc8785
from fastapi import FastAPI, Header, Request
from fastapi.responses import JSONResponse
from pydantic import BaseModel, ConfigDict, ValidationError

CURSOR_RE = re.compile(r"^[A-Za-z0-9_.:-]{1,128}$")
HEX64 = re.compile(r"^[0-9a-f]{64}$")
MAX_BATCH_BYTES = 512 * 1024
MAX_BATCH_EVENTS = 500
MAX_ARTIFACT_BYTES = 1024 * 1024


class _Strict(BaseModel):
    model_config = ConfigDict(extra="forbid")


class Observation(_Strict):
    kind: Literal["core_event", "platform_event"]
    level: Literal["engine_event", "outbound_event"] | None
    source_event: dict[str, Any] | None
    native_event_id: str
    source_event_digest: str
    source_event_ref: str | None
    source_schema_ref: str
    source_run_ref: str | None
    source_sequence: int | None
    episode_ref: str | None = None
    goal_ref: str | None = None
    layer_mapping_ref: str | None = None
    observed_at: str
    trace_refs: list[str]
    coverage_marker: Literal["complete_run", "open_run", "gap", "late"] | None


class Receipt(_Strict):
    run_id: str
    source_schema_ref: str
    source_artifact_ref: str
    verified_through_seq: int
    chain_head_hash: str
    source_artifact_digest: str
    check_result: dict[str, Any]
    verifier_agent_core_sha: str
    verifier_contract_version: str
    checked_at: str


class Batch(_Strict):
    contract_version: Literal["pulso-observations-2"]
    source_id: str
    tenant_id: str
    partition: str
    scan_mode: Literal["fast_poll", "rescan"]
    expected_cursor_revision: int | None
    from_seq: int | None
    to_seq: int | None
    cursor: str
    cut_ref: str | None = None
    events: list[Observation]
    verification_receipts: list[Receipt]
    batch_digest: str


def jcs_digest(body: dict[str, Any]) -> str:
    return hashlib.sha256(rfc8785.dumps(body)).hexdigest()


@dataclass
class IngestState:
    ledger: dict[tuple[str, str, str, str | None, str], str] = field(default_factory=dict)
    cursors: dict[tuple[str, str], dict[str, Any]] = field(default_factory=dict)
    receipts: dict[str, dict[str, Any]] = field(default_factory=dict)
    batches: list[dict[str, Any]] = field(default_factory=list)  # committed batches, in commit order
    verification_receipts: list[dict[str, Any]] = field(default_factory=list)
    artifacts: dict[str, dict[str, Any]] = field(default_factory=dict)
    # failure injection (tests): (status, headers) returned *before* any write, consumed one per request
    fail_next: list[tuple[int, dict[str, str]]] = field(default_factory=list)
    lock: threading.Lock = field(default_factory=threading.Lock)
    raw_bodies: list[bytes] = field(default_factory=list)

    def count(self, kind: str | None = None, level: str | None = None) -> int:
        return sum(1 for k in self.ledger if (kind is None or k[2] == kind) and (level is None or k[3] == level))


def _err(status: int, code: str, headers: dict[str, str] | None = None) -> JSONResponse:
    return JSONResponse({"error": code}, status_code=status, headers=headers)


def create_app(state: IngestState | None = None) -> FastAPI:
    st = state or IngestState()
    app = FastAPI()
    app.state.ingest = st

    @app.post("/internal/v1/platform/observations")
    async def observations(request: Request, idempotency_key: str | None = Header(default=None)) -> Any:
        raw = await request.body()
        with st.lock:
            if st.fail_next:
                code, headers = st.fail_next.pop(0)
                return _err(code, "injected", headers)
            if len(raw) > MAX_BATCH_BYTES:
                return _err(413, "batch_too_large")
            try:
                batch = Batch.model_validate_json(raw)
            except ValidationError:
                return _err(422, "schema_invalid")
            body = batch.model_dump(mode="json")
            body.pop("batch_digest")
            if not HEX64.match(batch.batch_digest) or jcs_digest(body) != batch.batch_digest:
                return _err(422, "batch_digest_mismatch")
            if idempotency_key != batch.batch_digest:
                return _err(422, "idempotency_key_mismatch")
            if len(batch.events) > MAX_BATCH_EVENTS or not CURSOR_RE.match(batch.cursor):
                return _err(422, "limits")
            for ev in batch.events:
                if (ev.kind == "core_event") != (ev.level is not None):
                    return _err(422, "level_required_for_core_event")
                if ev.source_event is None and ev.source_event_ref is None:
                    return _err(422, "source_event_ref_required")
                if not HEX64.match(ev.source_event_digest):
                    return _err(422, "digest_format")
                if ev.level == "engine_event" and (ev.source_run_ref is None or ev.source_sequence is None):
                    return _err(422, "audit_run_and_sequence_required")
            if batch.scan_mode == "fast_poll" and batch.expected_cursor_revision is None:
                return _err(422, "expected_cursor_revision_required")
            if batch.scan_mode == "rescan" and batch.expected_cursor_revision is not None:
                return _err(422, "rescan_requires_null_revision")
            prior = st.receipts.get(batch.batch_digest)
            if prior is not None:  # committed earlier: return the committed receipt, never re-apply a checkpoint
                return JSONResponse(prior, status_code=200)
            ckey = (batch.source_id, batch.partition)
            cur = st.cursors.get(ckey, {"cursor": None, "revision": 0, "last_batch_digest": None})
            if batch.scan_mode == "fast_poll" and batch.expected_cursor_revision != cur["revision"]:
                return _err(409, "stale_cursor_revision")  # before any write
            for ev in batch.events:  # conflict check first: all-or-nothing
                k = (batch.tenant_id, batch.source_id, ev.kind, ev.level, ev.native_event_id)
                if k in st.ledger and st.ledger[k] != ev.source_event_digest:
                    return _err(409, "source_digest_conflict")
            accepted = dup = 0
            for ev in batch.events:
                k = (batch.tenant_id, batch.source_id, ev.kind, ev.level, ev.native_event_id)
                if k in st.ledger:
                    dup += 1
                else:
                    st.ledger[k] = ev.source_event_digest
                    accepted += 1
            advanced = False
            if batch.scan_mode == "fast_poll":
                cur = {"cursor": batch.cursor, "revision": cur["revision"] + 1,
                       "last_batch_digest": batch.batch_digest}
                st.cursors[ckey] = cur
                advanced = True
            st.verification_receipts.extend(r.model_dump(mode="json") for r in batch.verification_receipts)
            st.batches.append(batch.model_dump(mode="json"))
            st.raw_bodies.append(raw)
            receipt = {"batch_digest": batch.batch_digest, "accepted_event_count": accepted,
                       "duplicate_event_count": dup, "checkpoint_advanced": advanced,
                       "current_cursor": cur["cursor"], "cursor_revision": cur["revision"]}
            # a duplicate POST returns the committed receipt without re-applying anything
            st.receipts[batch.batch_digest] = {**receipt, "checkpoint_advanced": False, "accepted_event_count": 0,
                                               "duplicate_event_count": accepted + dup}
            return JSONResponse(receipt, status_code=202)

    @app.get("/internal/v1/platform/exporters/{source_id}/partitions/{partition}/cursor")
    def get_cursor(source_id: str, partition: str) -> Any:
        if not re.match(r"^[A-Za-z0-9_.:-]{1,64}$", partition):
            return _err(404, "not_found")
        cur = st.cursors.get((source_id, partition), {"cursor": None, "revision": 0, "last_batch_digest": None})
        return {"contract_version": "pulso-observations-2", "source_id": source_id, "partition": partition,
                "cursor": cur["cursor"], "cursor_revision": cur["revision"],
                "last_batch_digest": cur["last_batch_digest"], "coverage": "partial", "cut_ref": None,
                "updated_at": None}

    @app.post("/internal/v1/broker/artifacts")
    async def artifacts(request: Request, idempotency_key: str | None = Header(default=None)) -> Any:
        raw = await request.body()
        if len(raw) > MAX_ARTIFACT_BYTES + 8192:
            return _err(413, "artifact_too_large")
        body = await request.json()
        digest = body["content_digest"]
        content = body["content"]
        if body["encoding"] == "utf8":
            data = content.encode("utf-8")
            if len(data) > MAX_ARTIFACT_BYTES:
                return _err(413, "artifact_too_large")
            actual = hashlib.sha256(data).hexdigest()
        else:
            actual = hashlib.sha256(rfc8785.dumps(content)).hexdigest()
        if actual != digest or idempotency_key != digest:
            return _err(422, "digest_mismatch")
        with st.lock:
            prior = st.artifacts.get(digest)
            if prior is None:
                st.artifacts[digest] = {"ref": f"artifact:{digest}", "body": body}
            return JSONResponse({"artifact_ref": f"artifact:{digest}", "receipt_ref": f"receipt:{digest}"},
                                status_code=200 if prior else 201)

    return app
