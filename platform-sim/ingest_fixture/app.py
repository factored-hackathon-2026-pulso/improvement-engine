"""In-memory ingest fixture implementing plan 16.13.3 / 16.13.5 / 16.15.3 (CL-0008 P5-P7) server semantics.

DOUBLE: not Codex's real control-api. It enforces the agreed transport rules so the exporter can be tested:
unknown fields rejected, JCS batch digest recomputed server-side, Idempotency-Key == batch_digest, 202 only after
commit, 200 duplicate returns the committed receipt without re-applying a checkpoint, fast_poll CAS on
expected_cursor_revision (stale -> 409 before any write), rescan never advances the checkpoint, event ledger dedup
identity (tenant, source_id, kind, level, native_event_id), source digest conflict -> 409, 1 MiB artifacts."""

from __future__ import annotations

import hashlib
import json
import re
import threading
from dataclasses import dataclass, field
from typing import Any, Literal

import rfc8785
from cryptography.exceptions import InvalidSignature
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PublicKey
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


class ArtifactRef(_Strict):
    """Annex D: ArtifactRef = {id, digest, media_type}; opaque id, never a path/URL."""

    id: str
    digest: str
    media_type: str


class Observation(_Strict):
    kind: Literal["core_event", "platform_event"]
    level: Literal["engine_event", "outbound_event"] | None
    source_event: dict[str, Any] | None
    native_event_id: str
    source_event_digest: str
    source_event_ref: ArtifactRef | None
    source_schema_ref: ArtifactRef
    source_run_ref: str | None
    source_sequence: int | None
    episode_ref: dict[str, str] | None = None
    goal_ref: dict[str, str] | None = None
    layer_mapping_ref: ArtifactRef | None = None
    observed_at: str
    trace_refs: list[ArtifactRef]
    coverage_marker: Literal["complete_run", "open_run", "gap", "late"] | None


class Receipt(_Strict):
    run_id: str
    source_schema_ref: ArtifactRef
    source_artifact_ref: ArtifactRef
    verified_through_seq: int
    chain_head_hash: str
    source_artifact_digest: str
    check_result: dict[str, Any]
    verifier_agent_core_sha: str
    verifier_contract_version: str
    checked_at: str


class Upload(_Strict):
    schema_version: Literal["1"]
    binding_ref: str
    source_schema_ref: ArtifactRef | None
    classification: Literal["treated", "restricted_original"]
    information_partition: str
    artifact_kind: Literal["schema", "source_material", "layer_mapping", "cut", "other"]
    media_type: Literal["application/json", "application/x-ndjson", "text/plain"]
    encoding: Literal["json", "utf8"]
    content: Any
    content_digest: str


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
    cut_ref: ArtifactRef | None = None
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
    auth_log: list[dict[str, Any]] = field(default_factory=list)  # accepted token claims (no token material)
    seen_jti: set[tuple[str, str]] = field(default_factory=set)

    def count(self, kind: str | None = None, level: str | None = None) -> int:
        return sum(1 for k in self.ledger if (kind is None or k[2] == kind) and (level is None or k[3] == level))


def _err(status: int, code: str, headers: dict[str, str] | None = None) -> JSONResponse:
    return JSONResponse({"error": code}, status_code=status, headers=headers)


@dataclass(frozen=True)
class FixtureKey:
    iss: str
    aud: str
    key: Ed25519PublicKey


@dataclass
class FixtureAuth:
    """Optional A03 service-JWT verification (typ=JWT, EdDSA fixed, kid bound to one (iss, aud), exp <= 5 min,
    singular scope, `sub` = registered exporter binding, receiver-owned jti replay). DOUBLE of the real receiver."""

    keys: dict[str, FixtureKey]
    binding_ref: str
    tenant_id: str
    now: Any = None  # callable returning epoch seconds (tests may freeze it)


def _b64d(text: str) -> bytes:
    import base64

    return base64.urlsafe_b64decode(text + "=" * (-len(text) % 4))


def _authorize(st: IngestState, auth: FixtureAuth | None, request: Request, *, aud: str, scope: str,
               purpose: str | None) -> JSONResponse | None:
    import json
    import time

    if auth is None:
        return None
    header_value = request.headers.get("authorization", "")
    if not header_value.startswith("Bearer "):
        return _err(401, "missing_bearer")
    parts = header_value[7:].split(".")
    if len(parts) != 3:
        return _err(401, "malformed")
    try:
        header, claims, sig = json.loads(_b64d(parts[0])), json.loads(_b64d(parts[1])), _b64d(parts[2])
    except ValueError:
        return _err(401, "malformed")
    if (not isinstance(header, dict) or not isinstance(claims, dict) or header.get("alg") != "EdDSA"
            or header.get("typ") != "JWT" or set(header) != {"alg", "kid", "typ"}):
        return _err(401, "bad_header")
    entry = auth.keys.get(header["kid"]) if isinstance(header["kid"], str) else None
    if entry is None:
        return _err(401, "unknown_kid")
    try:
        entry.key.verify(sig, f"{parts[0]}.{parts[1]}".encode())
    except InvalidSignature:
        return _err(401, "bad_signature")
    now = auth.now() if auth.now else time.time()
    exp, jti = claims.get("exp"), claims.get("jti")
    if claims.get("iss") != entry.iss or claims.get("aud") != entry.aud or claims.get("aud") != aud:
        return _err(401, "wrong_audience")
    if not isinstance(exp, int | float) or not isinstance(jti, str) or not jti:
        return _err(401, "missing_claims")
    if exp <= now or exp - now > 330:
        return _err(401, "expired_or_ttl")
    if claims.get("scope") != scope or (purpose is not None and claims.get("purpose") != purpose):
        return _err(403, "scope_denied")
    if claims.get("sub") != auth.binding_ref or claims.get("tenant_id") != auth.tenant_id:
        return _err(403, "binding_or_tenant_denied")
    with st.lock:
        if (claims["iss"], jti) in st.seen_jti:
            return _err(401, "jti_replayed")
        st.seen_jti.add((claims["iss"], jti))
        st.auth_log.append({k: claims.get(k) for k in ("iss", "aud", "scope", "purpose", "sub", "tenant_id", "jti")})
    return None


def _resolve_ref(st: IngestState, ref: ArtifactRef) -> dict[str, Any] | None:
    art = st.artifacts.get(ref.digest)
    if art is None or art["ref"] != {"id": ref.id, "digest": ref.digest, "media_type": ref.media_type}:
        return None
    return art


def _material(st: IngestState, ref: ArtifactRef) -> bytes | None:
    """Raw NDJSON material behind a ref (single artifact or immutable manifest of chunks)."""
    art = _resolve_ref(st, ref)
    if art is None:
        return None
    content = art["body"]["content"]
    if ref.media_type == "application/x-ndjson":
        return str(content).encode("utf-8")
    out = b""
    for ch in content.get("chunks", []):
        sub = _material(st, ArtifactRef(**ch["ref"]))
        if sub is None:
            return None
        out += sub
    return out


def create_app(state: IngestState | None = None, auth: FixtureAuth | None = None) -> FastAPI:
    st = state or IngestState()
    app = FastAPI()
    app.state.ingest = st

    @app.post("/internal/v1/platform/observations")
    async def observations(request: Request, idempotency_key: str | None = Header(default=None)) -> Any:
        raw = await request.body()
        denied = _authorize(st, auth, request, aud="control-api", scope="observations",
                            purpose="platform_observations")
        if denied is not None:
            return denied
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
            if auth is not None and batch.tenant_id != auth.tenant_id:
                return _err(403, "cross_tenant")
            for ev in batch.events:  # every ArtifactRef must resolve to a stored artifact with the same identity
                refs = [ev.source_schema_ref, *([ev.source_event_ref] if ev.source_event_ref else [])]
                if any(_resolve_ref(st, r) is None for r in refs):
                    return _err(422, "unresolvable_artifact_ref")
            for rc in batch.verification_receipts:
                mat = _material(st, rc.source_artifact_ref)
                if mat is None or _resolve_ref(st, rc.source_schema_ref) is None:
                    return _err(422, "unresolvable_artifact_ref")
                lines = mat.decode("utf-8").splitlines()
                try:
                    head = json.loads(lines[rc.verified_through_seq])["hash"]
                except (IndexError, ValueError, KeyError):
                    return _err(422, "receipt_material_mismatch")
                if (hashlib.sha256(mat).hexdigest() != rc.source_artifact_digest or head != rc.chain_head_hash
                        or len(lines) != rc.verified_through_seq + 1 or not re.fullmatch(
                            r"[0-9a-f]{40}", rc.verifier_agent_core_sha)):
                    return _err(422, "receipt_material_mismatch")
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
    def get_cursor(source_id: str, partition: str, request: Request) -> Any:
        denied = _authorize(st, auth, request, aud="control-api", scope="observations", purpose=None)
        if denied is not None:
            return denied
        if not re.match(r"^[A-Za-z0-9_.:-]{1,64}$", partition):
            return _err(404, "not_found")
        cur = st.cursors.get((source_id, partition), {"cursor": None, "revision": 0, "last_batch_digest": None})
        return {"contract_version": "pulso-observations-2", "source_id": source_id, "partition": partition,
                "cursor": cur["cursor"], "cursor_revision": cur["revision"],
                "last_batch_digest": cur["last_batch_digest"], "coverage": "partial", "cut_ref": None,
                "updated_at": None}

    @app.post("/internal/v1/broker/artifacts")
    async def artifacts(request: Request, idempotency_key: str | None = Header(default=None)) -> Any:
        denied = _authorize(st, auth, request, aud="lab-broker", scope="artifact_write", purpose="artifact_upload")
        if denied is not None:
            return denied
        raw = await request.body()
        if len(raw) > MAX_ARTIFACT_BYTES + 8192:
            return _err(413, "artifact_too_large")
        try:
            body = Upload.model_validate_json(raw).model_dump(mode="json")
        except ValidationError:
            return _err(422, "schema_invalid")
        if body["source_schema_ref"] is None and body["artifact_kind"] != "schema":
            return _err(422, "source_schema_ref_required")
        if body["source_schema_ref"] is not None and _resolve_ref(st, ArtifactRef(**body["source_schema_ref"])) is None:
            return _err(422, "unresolvable_artifact_ref")
        digest, content = body["content_digest"], body["content"]
        if body["encoding"] == "utf8":
            if not isinstance(content, str):
                return _err(422, "schema_invalid")
            data = content.encode("utf-8")
            if len(data) > MAX_ARTIFACT_BYTES:
                return _err(413, "artifact_too_large")
            actual = hashlib.sha256(data).hexdigest()
        else:
            actual = hashlib.sha256(rfc8785.dumps(content)).hexdigest()
        if actual != digest or idempotency_key != digest or not HEX64.match(digest):
            return _err(422, "digest_mismatch")
        with st.lock:
            prior = st.artifacts.get(digest)
            ref = {"id": f"artifact:{digest}", "digest": digest, "media_type": body["media_type"]}
            if prior is None:
                st.artifacts[digest] = {"ref": ref, "body": body}
            elif prior["ref"] != ref:
                return _err(409, "digest_conflict")
            return JSONResponse({"artifact_ref": ref, "receipt_ref": {**ref, "id": f"receipt:{digest}"}},
                                status_code=200 if prior else 201)

    return app
