"""Exporter service: read Core (read-only), verify the audit chain with the pinned agent-core functions, build
`pulso-observations-2` batches, persist -> POST -> ACK -> commit cursors in one SQLite transaction, resume.

No use of AuditSink.read / PostgresOutbox.pending / mark_delivered. Audit payload is never forwarded: audit
observations carry `source_event=null`, the native hash as `source_event_digest` and a reference to the exact-bytes
NDJSON chain artifact uploaded under the exporter binding. Registry/outbox observations carry the public pinned
projection (`project_registry_event`, `project_outbox_message`)."""

from __future__ import annotations

import random
import time
from collections.abc import Callable
from dataclasses import dataclass, field
from datetime import UTC, datetime
from typing import Any

import httpx
from agent_core.audit.chain import check_chain, event_from_json, event_hash, genesis_hash
from agent_core.domain import OutboxMessage, canonical_bytes, loads, to_jsonable
from agent_core.outbound.project import project_outbox_message
from agent_core.registry.models import RegistryEvent
from agent_core.registry.outbound import project_registry_event

from . import artifact as art
from .config import ARTIFACTS_PATH, CONTRACT, OBSERVATIONS_PATH, PIN_CONTRACT_VERSION, PIN_SHA, ExporterConfig
from .reader import AuditRow, CoreReader, Snapshot
from .state import ExporterState, Pending

AUDIT, REGISTRY, OUTBOX = "audit", "registry", "outbox"
SCHEMA_EVENT, SCHEMA_OUTBOUND = "core-event", "core-outbound"
ArtifactRef = dict[str, str]
SPARSE_TABLE = {REGISTRY: "reg_events", OUTBOX: "outbox"}


class SimulatedCrash(RuntimeError):
    """Raised by test hooks to emulate a process death at a precise point."""


@dataclass
class PollReport:
    batches_sent: int = 0
    events_sent: int = 0
    duplicate_acks: int = 0
    partial_reasons: list[str] = field(default_factory=list)
    stopped: dict[str, str] = field(default_factory=dict)
    deferred: int = 0
    rescan_batches: int = 0


@dataclass
class Spec:
    source: str
    partition: str
    scan_mode: str
    from_seq: int | None
    to_seq: int | None
    cursor: str
    events: list[dict[str, Any]]
    receipts: list[dict[str, Any]]
    delta: dict[str, Any]


def _utc(ts: Any) -> str:
    if isinstance(ts, datetime):
        return ts.astimezone(UTC).strftime("%Y-%m-%dT%H:%M:%SZ")
    return str(ts)


def jcs_digest(body: dict[str, Any]) -> str:
    return art.jcs_digest(body)


class Exporter:
    def __init__(self, cfg: ExporterConfig, reader: CoreReader, state: ExporterState, client: httpx.Client,
                 clock: Callable[[], datetime] | None = None, sleep: Callable[[float], None] = time.sleep,
                 rng: random.Random | None = None) -> None:
        self.cfg, self.reader, self.state, self.client = cfg, reader, state, client
        self.clock = clock or (lambda: datetime.now(UTC))
        self.sleep = sleep
        self.rng = rng or random.Random()
        self.crash_after_ack: Callable[[], None] | None = None
        self.crash_before_post: Callable[[], None] | None = None
        self._stale = False
        self._schemas: dict[str, ArtifactRef] = {}

    def close(self) -> None:
        self.state.close()

    # ------------------------------------------------------------------ public API
    def poll_once(self) -> PollReport:
        rep = PollReport()
        self._resume_pending(rep)
        for source in (AUDIT, REGISTRY, OUTBOX):
            self._drain(source, rep)
        self._fill_report(rep)
        return rep

    def rescan(self) -> PollReport:
        """Read-only anti-entropy (never advances a checkpoint): (1) every exported (run, seq) is compared with Core
        (tampered hash -> source_conflict, stop that partition) and verified rows missing from the local ledger are
        re-exported as `late`; (2) the full prefix of every still-open run is re-exported (`open_run`) so Pulso
        can dedupe/complete what a finite overlap could have missed."""
        rep = PollReport()
        self._resume_pending(rep)
        for _ in range(1000):
            if self._blocked():
                rep.deferred += 1
                break
            try:
                with self.reader.snapshot() as snap:
                    spec = self._build_audit_rescan(snap)
            except _UploadFailed:
                rep.deferred += 1
                break
            if spec is None or not self._deliver(spec, rep):
                break
            rep.rescan_batches += 1
        for rh in [r for r in self.state.runs() if r.status == "ok" and not r.closed and r.last_seq >= 0]:
            self._rescan_open_prefix(rh.run_id, rh.last_seq, rep)
        self._fill_report(rep)
        return rep

    def sweep(self) -> PollReport:
        """24 h sweep: full pinned `check_chain` over every run's complete prefix plus the exported-hash ledger
        comparison. A broken run is excluded from completeness; a ledger/hash mismatch stops the audit partition."""
        rep = PollReport()
        with self.reader.snapshot() as snap:
            for run_id, head in sorted(snap.heads().items()):
                rh = self.state.run(run_id)
                if rh and rh.status != "ok":
                    continue
                prefix = snap.audit_prefix(run_id, head)
                if self._conflict(run_id, prefix):
                    break
                _, problem = self._verify_incremental(None, run_id, prefix)
                if problem is None:
                    chk = check_chain(run_id, [event_from_json(r.event_json) for r in prefix])
                    if not chk.ok:
                        problem = f"run_chain_broken:{run_id}:{chk.broken_at}"
                if problem:
                    self._flag(run_id, rh, problem)
        self._fill_report(rep)
        return rep

    def coverage(self) -> dict[str, Any]:
        reasons = self._reasons()
        return {"coverage": "partial" if reasons else "unknown", "missing_reason": reasons, "cut_ref": None}

    # ------------------------------------------------------------------ reporting
    def _reasons(self) -> list[str]:
        out = [r.reason for r in self.state.runs() if r.status != "ok" and r.reason]
        open_runs = sum(1 for r in self.state.runs() if r.status == "ok" and not r.closed)
        if open_runs:
            out.append(f"open_runs:{open_runs}")
        if self.state.any_skipped():
            out.append("registry_seq_gap_unverified")
        if self.state.meta("rebuild") == "1":
            out.append("exporter_rebuild")
        return sorted(out)

    def _fill_report(self, rep: PollReport) -> None:
        rep.partial_reasons = [r for r in self._reasons() if not r.startswith("open_runs:")]
        rep.stopped = self.state.stopped()

    # ------------------------------------------------------------------ loop
    def _drain(self, source: str, rep: PollReport) -> None:
        stale_retries = 0
        for _ in range(10_000):
            if self.state.is_stopped(source):
                return
            if self._blocked():  # an unACKed stored batch must be re-POSTed first, never replaced
                rep.deferred += 1
                return
            try:
                with self.reader.snapshot() as snap:
                    spec = (self._build_late(snap, source) if source != AUDIT else None) or self._build(snap, source)
            except _UploadFailed:
                rep.deferred += 1
                return
            if spec is None or self.state.is_stopped(spec.partition):
                return
            self._stale = False
            if not self._deliver(spec, rep):
                if self._stale and stale_retries < 3:
                    stale_retries += 1  # revision refreshed from the authoritative cursor: rebuild and retry
                    continue
                return

    def _build(self, snap: Snapshot, source: str) -> Spec | None:
        return self._build_audit(snap) if source == AUDIT else self._build_sparse(snap, source)

    # ------------------------------------------------------------------ audit
    def _blocked(self) -> bool:
        p = self.state.pending()
        return p is not None and not self.state.is_stopped(p.partition)

    def _schema(self, which: str) -> ArtifactRef:
        """Pinned schema artifact (annex D ArtifactRef): configured, or bootstrapped through the artifact route."""
        cfg_ref = self.cfg.source_schema_ref if which == SCHEMA_EVENT else self.cfg.registry_schema_ref
        if cfg_ref:
            return cfg_ref
        if which not in self._schemas:
            doc = {"schema": which, "agent_core_sha": PIN_SHA, "contract_version": PIN_CONTRACT_VERSION}
            self._schemas[which] = self._upload(doc, "json", "application/json", art.jcs_digest(doc), "schema",
                                                kind="schema", classification="treated", schema_ref=None)
        return self._schemas[which]

    # ------------------------------------------------------------------ audit
    def _obs_audit(self, row: AuditRow, ref: ArtifactRef, marker: str) -> dict[str, Any]:
        return {"kind": "core_event", "level": "engine_event", "source_event": None, "native_event_id": row.event_id,
                "source_event_digest": row.hash, "source_event_ref": ref,
                "source_schema_ref": self._schema(SCHEMA_EVENT), "source_run_ref": row.run_id,
                "source_sequence": row.seq, "episode_ref": None, "goal_ref": None, "layer_mapping_ref": None,
                "observed_at": _utc(self.clock()), "trace_refs": [], "coverage_marker": marker}

    def _verify_incremental(self, rh: Any, run_id: str, rows: list[AuditRow]) -> tuple[list[AuditRow], str | None]:
        """Valid contiguous prefix of `rows` and the first problem (run_chain_gap / run_chain_broken). Besides the
        pinned hash chain, the indexed row columns must equal the signed event (native id/type are exported)."""
        last = rh.last_seq if rh else -1
        prev = rh.last_hash if rh else genesis_hash(run_id)
        ok: list[AuditRow] = []
        for row in rows:
            expect = last + 1
            if row.seq != expect:
                return ok, f"run_chain_gap:{run_id}:{expect}-{row.seq - 1}"
            try:
                ev = event_from_json(row.event_json)
                good = (ev.run_id == run_id and ev.seq == row.seq and ev.prev_hash == prev == row.prev_hash
                        and ev.hash == row.hash == event_hash(ev, prev)
                        and ev.event_id == row.event_id and ev.type == row.type)
            except Exception:
                good = False
            if not good:
                return ok, f"run_chain_broken:{run_id}:{row.seq}"
            ok.append(row)
            last, prev = row.seq, row.hash
        return ok, None

    def _flag(self, run_id: str, rh: Any, problem: str) -> None:
        status = "gap" if problem.startswith("run_chain_gap") else "broken"
        self.state.set_run_status(run_id, rh.last_seq if rh else -1, rh.last_hash if rh else "",
                                  bool(rh and rh.closed), status, problem)

    def _conflict(self, run_id: str, rows: list[AuditRow]) -> bool:
        """Exported (run, seq) whose hash changed in Core -> source_conflict: stop the audit partitions."""
        for row in rows:
            known = self.state.ledger_hash(run_id, row.seq)
            if known is not None and known != row.hash:
                reason = f"source_conflict:{run_id}:{row.seq}"
                self.state.stop(self._part(row), reason)
                self.state.stop(AUDIT, reason)
                return True
        return False

    @staticmethod
    def _part(row: AuditRow) -> str:
        return "audit:" + row.ts.astimezone(UTC).date().isoformat()

    def _wire_size(self, spec: Spec) -> int:
        return len(canonical_bytes({**self._body(spec), "batch_digest": "0" * 64}))  # the digest is on the wire

    def _build_audit(self, snap: Snapshot) -> Spec | None:
        cands: dict[str, list[AuditRow]] = {}
        for run_id, head in sorted(snap.heads().items()):
            rh = self.state.run(run_id)
            if rh and rh.status != "ok":
                continue
            last = rh.last_seq if rh else -1
            if head <= last:
                continue
            rows = snap.audit_rows(run_id, last, self.cfg.batch_max_events)
            good, problem = self._verify_incremental(rh, run_id, rows)
            if problem:
                self._flag(run_id, rh, problem)
            if good:
                cands[run_id] = good
        if not cands:
            return None
        chosen = min(self._part(rs[0]) for rs in cands.values())
        taken: list[AuditRow] = []
        for run_id in sorted(cands):
            for r in cands[run_id]:
                if self._part(r) != chosen:
                    break
                taken.append(r)
        return self._fit_audit(snap, chosen, taken, "fast_poll", None)

    def _fit_audit(self, snap: Snapshot, partition: str, rows: list[AuditRow], mode: str,
                   marker: str | None) -> Spec | None:
        limit = len(rows)
        while True:
            spec = self._assemble_audit(snap, partition, rows[:limit], mode, marker)
            if spec is None or self._wire_size(spec) <= self.cfg.batch_max_bytes or limit <= 1:
                return spec
            limit = max(1, limit // 2)

    def _assemble_audit(self, snap: Snapshot, partition: str, taken: list[AuditRow], scan_mode: str,
                        marker_override: str | None) -> Spec | None:
        taken = taken[: self.cfg.batch_max_events]
        by_run: dict[str, list[AuditRow]] = {}
        for r in taken:
            by_run.setdefault(r.run_id, []).append(r)
        events: list[dict[str, Any]] = []
        receipts: list[dict[str, Any]] = []
        runs_delta: dict[str, Any] = {}
        ledger: list[list[Any]] = []
        for run_id, rows in sorted(by_run.items()):
            rh = self.state.run(run_id)
            closes = any(r.type == "run_closed" for r in rows)
            last_row = rows[-1]
            try:
                prefix = snap.audit_prefix(run_id, last_row.seq)
                ref, material_digest = self._upload_chain(prefix, partition)
            except art.MaterialError:
                self.state.set_run_status(run_id, rh.last_seq if rh else -1, rh.last_hash if rh else "",
                                          bool(rh and rh.closed), "gap", f"material_unavailable:{run_id}")
                return None
            marker = marker_override or ("late" if rh and rh.closed else ("complete_run" if closes else "open_run"))
            for r in rows:
                events.append(self._obs_audit(r, ref, marker))
                ledger.append([run_id, r.seq, r.hash])
            prev_last = rh.last_seq if rh else -1
            crossed = (last_row.seq + 1) // self.cfg.receipt_every > (prev_last + 1) // self.cfg.receipt_every
            if scan_mode == "fast_poll":
                runs_delta[run_id] = {"last_seq": last_row.seq, "last_hash": last_row.hash,
                                      "closed": bool(closes or (rh and rh.closed))}
            # receipt at close, every `receipt_every` events, for late events after close, and on any rescan
            if closes or crossed or (rh and rh.closed) or scan_mode == "rescan":
                receipts.append(self._receipt(run_id, prefix, ref, material_digest))
        cursor_in = self.state.cursor(partition)
        seed = (cursor_in[0] if cursor_in and cursor_in[0] else "") + "|" + ",".join(r.hash for r in taken)
        cursor = ("a." if scan_mode == "fast_poll" else "scan.") + art.sha256_raw(seed.encode())[:40]
        delta: dict[str, Any] = {"ledger": ledger}
        if scan_mode == "fast_poll":
            delta["runs"] = runs_delta
        return Spec(AUDIT, partition, scan_mode, None, None, cursor, events, receipts, delta)

    def _receipt(self, run_id: str, prefix: list[AuditRow], ref: ArtifactRef, material_digest: str) -> dict[str, Any]:
        events = [event_from_json(r.event_json) for r in prefix]
        chk = check_chain(run_id, events)  # pinned verifier, full authorised prefix from seq 0
        return {"run_id": run_id, "source_schema_ref": self._schema(SCHEMA_EVENT), "source_artifact_ref": ref,
                "verified_through_seq": prefix[-1].seq, "chain_head_hash": prefix[-1].hash,
                "source_artifact_digest": material_digest,
                "check_result": {"ok": chk.ok, "broken_at": chk.broken_at, "reason": chk.reason},
                "verifier_agent_core_sha": self.cfg.verifier_sha,
                "verifier_contract_version": self.cfg.verifier_contract_version,
                "checked_at": _utc(self.clock())}

    def _build_audit_rescan(self, snap: Snapshot) -> Spec | None:
        missing: list[AuditRow] = []
        for run_id, head in sorted(snap.heads().items()):
            rh = self.state.run(run_id)
            if rh and rh.status != "ok":
                continue
            prefix = snap.audit_prefix(run_id, head)
            if self._conflict(run_id, prefix):
                return None
            ok, problem = self._verify_incremental(None, run_id, prefix)  # never export unverified rows
            if problem:
                self._flag(run_id, rh, problem)
            missing.extend(r for r in ok if self.state.ledger_hash(run_id, r.seq) is None)
        if not missing:
            return None
        chosen = self._part(missing[0])
        rows = [m for m in missing if self._part(m) == chosen][: self.cfg.batch_max_events]
        return self._fit_audit(snap, chosen, rows, "rescan", "late")

    def _rescan_open_prefix(self, run_id: str, upto: int, rep: PollReport) -> None:
        after = -1
        for _ in range(100_000):
            if self._blocked():
                rep.deferred += 1
                return
            try:
                with self.reader.snapshot() as snap:
                    prefix = snap.audit_prefix(run_id, upto)
                    if self._conflict(run_id, prefix):
                        return
                    ok, problem = self._verify_incremental(None, run_id, prefix)
                    if problem:
                        self._flag(run_id, self.state.run(run_id), problem)
                    rows = [r for r in ok if r.seq > after]
                    if not rows:
                        return
                    part = self._part(rows[0])
                    rows = [r for r in rows if self._part(r) == part][: self.cfg.batch_max_events]
                    spec = self._fit_audit(snap, part, rows, "rescan", "open_run")
            except _UploadFailed:
                rep.deferred += 1
                return
            if spec is None or not self._deliver(spec, rep):
                return
            rep.rescan_batches += 1
            after = int(spec.events[-1]["source_sequence"])

    # ------------------------------------------------------------------ artifacts
    def _upload_chain(self, prefix: list[AuditRow], partition: str) -> tuple[ArtifactRef, str]:
        jsons = [r.event_json for r in prefix]
        chunks = art.plan_chunks(jsons, self.cfg.artifact_max_bytes, self.cfg.max_material_bytes)
        refs = [self._upload(c.data.decode("utf-8"), "utf8", "application/x-ndjson", art.sha256_raw(c.data),
                             partition) for c in chunks]
        material = b"".join(c.data for c in chunks)
        if len(chunks) == 1:
            return refs[0], art.sha256_raw(material)
        manifest = art.manifest_for(chunks, refs)
        mref = self._upload(manifest, "json", "application/json", art.jcs_digest(manifest), partition)
        return mref, manifest["material_digest"]

    def _upload(self, content: Any, encoding: str, media: str, digest: str, partition: str, *,
                kind: str = "source_material", classification: str = "restricted_original",
                schema_ref: ArtifactRef | None | str = "event") -> ArtifactRef:
        if schema_ref == "event":
            schema_ref = self._schema(SCHEMA_EVENT)
        body = {"schema_version": "1", "binding_ref": self.cfg.binding_ref, "source_schema_ref": schema_ref,
                "classification": classification, "information_partition": partition, "artifact_kind": kind,
                "media_type": media, "encoding": encoding, "content": content, "content_digest": digest}
        headers = {"Idempotency-Key": digest}
        tok = self.cfg.token("artifacts")
        if tok:
            headers["Authorization"] = "Bearer " + tok
        try:
            r = self.client.post(ARTIFACTS_PATH, json=body, headers=headers)
        except httpx.HTTPError as exc:
            raise _UploadFailed(str(exc)) from exc
        if r.status_code not in (200, 201):
            raise _UploadFailed(f"upload status {r.status_code}")
        ref = r.json().get("artifact_ref")
        if (not isinstance(ref, dict) or set(ref) != {"id", "digest", "media_type"} or ref["digest"] != digest
                or ref["media_type"] != media or not isinstance(ref["id"], str) or not ref["id"]):
            raise _UploadFailed("untrusted artifact ref")  # never adopt a ref that is not what we sent
        return {"id": ref["id"], "digest": ref["digest"], "media_type": ref["media_type"]}

    # ------------------------------------------------------------------ registry / outbox (sparse)
    @staticmethod
    def _cursor_seq(cur: tuple[str | None, int] | None) -> int:
        if cur and cur[0] and cur[0].startswith("s."):
            return int(cur[0][2:])
        return 0

    @staticmethod
    def _project(source: str, row: Any) -> tuple[dict[str, Any], str | None] | None:
        if source == REGISTRY:
            proj, run_ref = project_registry_event(RegistryEvent.model_validate(loads(row.body)), row.seq), None
        else:
            msg = OutboxMessage.model_validate(loads(row.body))
            proj, run_ref = project_outbox_message(msg), msg.run_id
        return None if proj is None else (to_jsonable(proj), run_ref)

    def _obs_sparse(self, source: str, row: Any, marker: str | None) -> dict[str, Any] | None:
        pr = self._project(source, row)
        if pr is None:
            return None
        view, run_ref = pr
        return {"kind": "core_event", "level": "outbound_event", "source_event": view,
                "native_event_id": view["event_id"], "source_event_digest": art.jcs_digest(view),
                "source_event_ref": None, "source_schema_ref": self._schema(SCHEMA_OUTBOUND),
                "source_run_ref": run_ref, "source_sequence": None, "episode_ref": None, "goal_ref": None,
                "layer_mapping_ref": None, "observed_at": _utc(self.clock()), "trace_refs": [],
                "coverage_marker": marker}

    def _sparse_spec(self, source: str, taken: list[Any], mode: str, marker: str | None,
                     delta: dict[str, Any]) -> Spec:
        events = [o for r in taken if (o := self._obs_sparse(source, r, marker))]
        prefix = "s." if mode == "fast_poll" else "late."
        return Spec(source, source, mode, taken[0].seq, taken[-1].seq, f"{prefix}{taken[-1].seq}", events, [], delta)

    def _build_sparse(self, snap: Snapshot, source: str) -> Spec | None:
        table = SPARSE_TABLE[source]
        self._schema(SCHEMA_OUTBOUND)  # bootstrap first so an upload failure is never mistaken for schema_invalid
        cur = self._reconciled_cursor(source)
        after = self._cursor_seq(cur)
        rows = snap.seq_rows(table, after, self.cfg.batch_max_events)
        try:
            for r in rows:  # unprojectable/invalid rows are schema_invalid: stop the partition, never fabricate
                self._project(source, r)
        except Exception:
            self.state.stop(source, f"schema_invalid:{table}")
            return None
        now = self.clock().timestamp()
        taken: list[Any] = []
        skipped: list[list[int]] = []
        expected = after + 1
        for row in rows:
            if row.seq != expected:
                lo, hi = expected, row.seq - 1  # one range, however wide the bigserial jump
                first = self.state.note_hole_range(source, lo, hi, now)
                if now - first < self.cfg.gap_grace_seconds:
                    break  # hold-back: the hole may still commit
                skipped.append([lo, hi])
            taken.append(row)
            expected = row.seq + 1
        if not taken:
            return None
        while True:
            sk = [r for r in skipped if r[1] < taken[-1].seq]
            spec = self._sparse_spec(source, taken, "fast_poll", None,
                                     {"holes_skipped": sk, "holes_unskipped_below": taken[-1].seq})
            if self._wire_size(spec) <= self.cfg.batch_max_bytes or len(taken) <= 1:
                return spec
            taken = taken[: len(taken) // 2]

    def _build_late(self, snap: Snapshot, source: str) -> Spec | None:
        ranges = self.state.skipped_ranges(source)
        rows = snap.seq_rows_in_ranges(SPARSE_TABLE[source], ranges, self.cfg.batch_max_events)
        if not rows:
            return None
        self._schema(SCHEMA_OUTBOUND)
        while True:
            spec = self._sparse_spec(source, rows, "rescan", "late", {"holes_cleared": [r.seq for r in rows]})
            if self._wire_size(spec) <= self.cfg.batch_max_bytes or len(rows) <= 1:
                return spec
            rows = rows[: len(rows) // 2]

    # ------------------------------------------------------------------ cursor authority
    def _reconciled_cursor(self, partition_or_source: str) -> tuple[str | None, int]:
        cur = self.state.cursor(partition_or_source)
        if cur is not None:
            return cur
        remote = self._get_cursor(partition_or_source, strict=True)
        if remote is None:
            return (None, 0)
        if remote["cursor_revision"] > 0:  # local state lost: Pulso cursor wins, rebuild mode is recorded
            self.state.set_meta("rebuild", "1")
        self.state.set_cursor(partition_or_source, remote["cursor"], int(remote["cursor_revision"]))
        return (remote["cursor"], int(remote["cursor_revision"]))

    def _source_of(self, partition: str) -> str:
        return self.cfg.source_id("audit" if partition.startswith("audit:") else partition)

    def _get_cursor(self, partition: str, strict: bool = False) -> dict[str, Any] | None:
        """strict: a failed read (network, 5xx, 401/403/404) defers instead of pretending 'no checkpoint'."""
        headers: dict[str, str] = {}
        tok = self.cfg.token("cursor")
        if tok:
            headers["Authorization"] = "Bearer " + tok
        try:
            r = self.client.get(f"/internal/v1/platform/exporters/{self._source_of(partition)}/partitions/"
                                f"{partition}/cursor", headers=headers)
        except httpx.HTTPError as exc:
            if strict:
                raise _UploadFailed(str(exc)) from exc
            return None
        if r.status_code == 200:
            return dict(r.json())
        if strict:
            raise _UploadFailed(f"cursor status {r.status_code}")
        return None

    # ------------------------------------------------------------------ batch body / transport
    def _body(self, spec: Spec) -> dict[str, Any]:
        rev: int | None = None
        if spec.scan_mode == "fast_poll":
            cur = self.state.cursor(spec.partition)
            if cur is None:
                cur = self._reconciled_cursor(spec.partition)
            rev = cur[1]
        return {"contract_version": CONTRACT, "source_id": self._source_of(spec.partition),
                "tenant_id": self.cfg.tenant_id, "partition": spec.partition, "scan_mode": spec.scan_mode,
                "expected_cursor_revision": rev, "from_seq": spec.from_seq, "to_seq": spec.to_seq,
                "cursor": spec.cursor, "cut_ref": None, "events": spec.events,
                "verification_receipts": spec.receipts}

    def _deliver(self, spec: Spec, rep: PollReport) -> bool:
        if self._blocked():
            rep.deferred += 1
            return False
        try:
            body = self._body(spec)
        except _UploadFailed:
            rep.deferred += 1
            return False
        digest = jcs_digest(body)
        raw = canonical_bytes({**body, "batch_digest": digest})
        if len(raw) > self.cfg.batch_max_bytes:
            self.state.stop(spec.partition, "batch_too_large")
            return False
        p = Pending(spec.partition, spec.scan_mode, digest, raw, spec.delta)
        self.state.save_pending(p)
        rep.events_sent += len(spec.events)
        return self._send_pending(p, rep)

    def _resume_pending(self, rep: PollReport) -> None:
        p = self.state.pending()
        if p is not None and not self.state.is_stopped(p.partition):
            self._send_pending(p, rep)

    def _send_pending(self, p: Pending, rep: PollReport) -> bool:
        if self.crash_before_post:
            self.crash_before_post()
        out = self._post(p, rep)
        if out is None:
            rep.deferred += 1
            return False
        status, ack = out
        if status in (200, 202):
            if status == 200:
                rep.duplicate_acks += 1
            if ack.get("batch_digest") != p.idem_key:
                self.state.stop(p.partition, "ack_digest_mismatch")
                return False
            if self.crash_after_ack:
                self.crash_after_ack()
            self.state.commit_ack(p, ack)
            rep.batches_sent += 1
            return True
        return False

    def _post(self, p: Pending, rep: PollReport) -> tuple[int, dict[str, Any]] | None:
        for attempt in range(self.cfg.max_retries + 1):
            headers = {"Idempotency-Key": p.idem_key, "Content-Type": "application/json"}
            tok = self.cfg.token("observations")  # A03: fresh token (new jti) for every HTTP attempt
            if tok:
                headers["Authorization"] = "Bearer " + tok
            try:
                r = self.client.post(OBSERVATIONS_PATH, content=p.body, headers=headers)
            except httpx.HTTPError:  # timeout after send -> unknown: the stored batch is re-POSTed
                self._backoff(attempt, None)
                continue
            code = r.status_code
            if code in (200, 202):
                return code, dict(r.json())
            if code in (429, 503):
                self._backoff(attempt, r.headers.get("Retry-After"))
                continue
            err = ""
            try:
                err = str(r.json().get("error", ""))
            except ValueError:
                pass
            if code == 409 and err == "stale_cursor_revision":
                self.state.drop_pending()  # lost race: refresh the authoritative cursor and rebuild
                remote = self._get_cursor(p.partition)
                if remote:
                    self.state.set_cursor(p.partition, remote["cursor"], int(remote["cursor_revision"]))
                self._stale = True
                return None
            if code == 409:
                self.state.stop(p.partition, f"digest_conflict:{err}")  # keep the pending batch for diagnosis
            elif code == 422:
                self.state.quarantine(p.idem_key, p.partition, err or "422", p.body)
                self.state.stop(p.partition, f"quarantined:{err or '422'}")
            else:  # 401/403/413/other: stop
                self.state.stop(p.partition, f"http_{code}")
            return None
        return None

    def _backoff(self, attempt: int, retry_after: str | None) -> None:
        base = float(retry_after) if retry_after and retry_after.replace(".", "", 1).isdigit() else 2.0 ** attempt
        self.sleep(min(self.cfg.backoff_cap_seconds, base) + self.rng.uniform(0, 0.25))


class _UploadFailed(RuntimeError):
    pass
