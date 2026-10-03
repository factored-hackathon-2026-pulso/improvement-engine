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
from .config import ARTIFACTS_PATH, CONTRACT, OBSERVATIONS_PATH, ExporterConfig
from .reader import AuditRow, CoreReader, Snapshot
from .state import ExporterState, Pending

AUDIT, REGISTRY, OUTBOX = "audit", "registry", "outbox"
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
        """Read-only anti-entropy: compare every exported (run, seq) with Core (tampered hash -> source_conflict,
        stop that partition) and re-export missing events with scan_mode=rescan (never advances the checkpoint)."""
        rep = PollReport()
        self._resume_pending(rep)
        for _ in range(1000):
            try:
                with self.reader.snapshot() as snap:
                    spec = self._build_audit_rescan(snap)
            except _UploadFailed:
                rep.deferred += 1
                break
            if spec is None:
                break
            if not self._deliver(spec, rep):
                break
            rep.rescan_batches += 1
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
    def _obs_audit(self, row: AuditRow, ref: str, marker: str) -> dict[str, Any]:
        return {"kind": "core_event", "level": "engine_event", "source_event": None, "native_event_id": row.event_id,
                "source_event_digest": row.hash, "source_event_ref": f"{ref}#seq={row.seq}",
                "source_schema_ref": self.cfg.source_schema_ref, "source_run_ref": row.run_id,
                "source_sequence": row.seq, "episode_ref": None, "goal_ref": None, "layer_mapping_ref": None,
                "observed_at": _utc(self.clock()), "trace_refs": [], "coverage_marker": marker}

    def _verify_incremental(self, rh: Any, run_id: str, rows: list[AuditRow]) -> tuple[list[AuditRow], str | None]:
        """Valid contiguous prefix of `rows` and the first problem (run_chain_gap / run_chain_broken)."""
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
                        and ev.hash == row.hash == event_hash(ev, prev))
            except Exception:
                good = False
            if not good:
                return ok, f"run_chain_broken:{run_id}:{row.seq}"
            ok.append(row)
            last, prev = row.seq, row.hash
        return ok, None

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
                status = "gap" if problem.startswith("run_chain_gap") else "broken"
                self.state.set_run_status(run_id, last, rh.last_hash if rh else "", bool(rh and rh.closed), status,
                                          problem)
            if good:
                cands[run_id] = good
        if not cands:
            return None
        part = lambda r: "audit:" + r.ts.astimezone(UTC).date().isoformat()  # noqa: E731
        chosen = min(part(rs[0]) for rs in cands.values())
        taken: list[AuditRow] = []
        for run_id in sorted(cands):
            for r in cands[run_id]:
                if part(r) != chosen:
                    break
                taken.append(r)
        limit = len(taken)
        while True:
            spec = self._assemble_audit(snap, chosen, taken[:limit])
            if spec is None:
                return None
            size = len(canonical_bytes(self._body(spec)))
            if size <= self.cfg.batch_max_bytes or limit <= 1:
                return spec
            limit = max(1, limit // 2)

    def _assemble_audit(self, snap: Snapshot, partition: str, taken: list[AuditRow]) -> Spec | None:
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
            except (art.MaterialError, _UploadFailed) as exc:
                if isinstance(exc, _UploadFailed):
                    raise
                self.state.set_run_status(run_id, rh.last_seq if rh else -1, rh.last_hash if rh else "",
                                          bool(rh and rh.closed), "gap", f"material_unavailable:{run_id}")
                return None
            marker = "late" if rh and rh.closed else ("complete_run" if closes else "open_run")
            for r in rows:
                events.append(self._obs_audit(r, ref, marker))
                ledger.append([run_id, r.seq, r.hash])
            runs_delta[run_id] = {"last_seq": last_row.seq, "last_hash": last_row.hash,
                                  "closed": bool(closes or (rh and rh.closed))}
            prev_last = rh.last_seq if rh else -1
            crossed = (last_row.seq + 1) // self.cfg.receipt_every > (prev_last + 1) // self.cfg.receipt_every
            if closes or crossed:
                receipts.append(self._receipt(run_id, prefix, ref, material_digest))
        cursor_in = self.state.cursor(partition)
        cursor = "a." + art.sha256_raw(
            ((cursor_in[0] if cursor_in and cursor_in[0] else "") + "|" + ",".join(r.hash for r in taken)).encode()
        )[:40]
        return Spec(AUDIT, partition, "fast_poll", None, None, cursor, events, receipts,
                    {"runs": runs_delta, "ledger": ledger})

    def _receipt(self, run_id: str, prefix: list[AuditRow], ref: str, material_digest: str) -> dict[str, Any]:
        events = [event_from_json(r.event_json) for r in prefix]
        chk = check_chain(run_id, events)  # pinned verifier, full authorised prefix from seq 0
        return {"run_id": run_id, "source_schema_ref": self.cfg.source_schema_ref, "source_artifact_ref": ref,
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
            for row in snap.audit_prefix(run_id, head):
                known = self.state.ledger_hash(run_id, row.seq)
                if known is None:
                    missing.append(row)
                elif known != row.hash:
                    self.state.stop("audit:" + row.ts.astimezone(UTC).date().isoformat(),
                                    f"source_conflict:{run_id}:{row.seq}")
                    self.state.stop(AUDIT, f"source_conflict:{run_id}:{row.seq}")
                    return None
        missing = [m for m in missing if not self.state.is_stopped(AUDIT)]
        if not missing:
            return None
        chosen = "audit:" + missing[0].ts.astimezone(UTC).date().isoformat()
        rows = [m for m in missing if "audit:" + m.ts.astimezone(UTC).date().isoformat() == chosen]
        rows = rows[: self.cfg.batch_max_events]
        events: list[dict[str, Any]] = []
        for r in rows:
            ref, _ = self._upload_chain(snap.audit_prefix(r.run_id, r.seq), chosen)
            events.append(self._obs_audit(r, ref, "late"))
        cur = "scan." + art.sha256_raw(",".join(r.hash for r in rows).encode())[:32]
        return Spec(AUDIT, chosen, "rescan", None, None, cur, events, [],
                    {"ledger": [[r.run_id, r.seq, r.hash] for r in rows]})

    # ------------------------------------------------------------------ artifacts
    def _upload_chain(self, prefix: list[AuditRow], partition: str) -> tuple[str, str]:
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

    def _upload(self, content: Any, encoding: str, media: str, digest: str, partition: str) -> str:
        body = {"schema_version": "1", "binding_ref": self.cfg.binding_ref,
                "source_schema_ref": self.cfg.source_schema_ref, "classification": "restricted_original",
                "information_partition": partition, "artifact_kind": "source_material", "media_type": media,
                "encoding": encoding, "content": content, "content_digest": digest}
        headers = {"Idempotency-Key": digest}
        if self.cfg.token_provider:
            headers["Authorization"] = "Bearer " + self.cfg.token_provider()
        try:
            r = self.client.post(ARTIFACTS_PATH, json=body, headers=headers)
        except httpx.HTTPError as exc:
            raise _UploadFailed(str(exc)) from exc
        if r.status_code not in (200, 201):
            raise _UploadFailed(f"upload status {r.status_code}")
        return str(r.json()["artifact_ref"])

    # ------------------------------------------------------------------ registry / outbox (sparse)
    @staticmethod
    def _cursor_seq(cur: tuple[str | None, int] | None) -> int:
        if cur and cur[0] and cur[0].startswith("s."):
            return int(cur[0][2:])
        return 0

    def _obs_sparse(self, source: str, row: Any, marker: str | None) -> dict[str, Any] | None:
        if source == REGISTRY:
            proj = project_registry_event(RegistryEvent.model_validate(loads(row.body)), row.seq)
            run_ref = None
        else:
            msg = OutboxMessage.model_validate(loads(row.body))
            proj, run_ref = project_outbox_message(msg), msg.run_id
        if proj is None:
            return None
        view = to_jsonable(proj)
        return {"kind": "core_event", "level": "outbound_event", "source_event": view,
                "native_event_id": view["event_id"], "source_event_digest": art.jcs_digest(view),
                "source_event_ref": None, "source_schema_ref": self.cfg.registry_schema_ref,
                "source_run_ref": run_ref, "source_sequence": None, "episode_ref": None, "goal_ref": None,
                "layer_mapping_ref": None, "observed_at": _utc(self.clock()), "trace_refs": [],
                "coverage_marker": marker}

    def _build_sparse(self, snap: Snapshot, source: str) -> Spec | None:
        table = SPARSE_TABLE[source]
        cur = self._reconciled_cursor(source)
        after = self._cursor_seq(cur)
        rows = snap.seq_rows(table, after, self.cfg.batch_max_events)
        try:
            for r in rows:  # unprojectable/invalid rows are schema_invalid: stop the partition, never fabricate
                self._obs_sparse(source, r, None)
        except Exception:
            self.state.stop(source, f"schema_invalid:{table}")
            return None
        now = self.clock().timestamp()
        taken, skipped, expected = [], [], after + 1
        for row in rows:
            if row.seq != expected:
                missing = list(range(expected, row.seq))
                first = min(self.state.note_hole(source, m, now) for m in missing)
                if now - first < self.cfg.gap_grace_seconds:
                    break  # hold-back: the hole may still commit
                skipped.extend(missing)
            taken.append(row)
            expected = row.seq + 1
        if not taken:
            return None
        events = [o for r in taken if (o := self._obs_sparse(source, r, None))]
        spec = Spec(source, source, "fast_poll", taken[0].seq, taken[-1].seq, f"s.{taken[-1].seq}", events, [],
                    {"holes_skipped": skipped, "holes_cleared": [r.seq for r in taken]})
        while len(canonical_bytes(self._body(spec))) > self.cfg.batch_max_bytes and len(taken) > 1:
            taken = taken[: len(taken) // 2]
            events = [o for r in taken if (o := self._obs_sparse(source, r, None))]
            spec = Spec(source, source, "fast_poll", taken[0].seq, taken[-1].seq, f"s.{taken[-1].seq}", events, [],
                        {"holes_skipped": [s for s in skipped if s < taken[-1].seq],
                         "holes_cleared": [r.seq for r in taken]})
        return spec

    def _build_late(self, snap: Snapshot, source: str) -> Spec | None:
        seqs = self.state.skipped_holes(source)
        rows = snap.seq_rows_in(SPARSE_TABLE[source], seqs)[: self.cfg.batch_max_events]
        if not rows:
            return None
        events = [o for r in rows if (o := self._obs_sparse(source, r, "late"))]
        return Spec(source, source, "rescan", rows[0].seq, rows[-1].seq, f"late.{rows[-1].seq}", events, [],
                    {"holes_cleared": [r.seq for r in rows]})

    # ------------------------------------------------------------------ cursor authority
    def _reconciled_cursor(self, partition_or_source: str) -> tuple[str | None, int]:
        cur = self.state.cursor(partition_or_source)
        if cur is not None:
            return cur
        remote = self._get_cursor(partition_or_source)
        if remote is None:
            return (None, 0)
        if remote["cursor_revision"] > 0:  # local state lost: Pulso cursor wins, rebuild mode is recorded
            self.state.set_meta("rebuild", "1")
        self.state.set_cursor(partition_or_source, remote["cursor"], int(remote["cursor_revision"]))
        return (remote["cursor"], int(remote["cursor_revision"]))

    def _source_of(self, partition: str) -> str:
        return self.cfg.source_id("audit" if partition.startswith("audit:") else partition)

    def _get_cursor(self, partition: str) -> dict[str, Any] | None:
        headers = {"Authorization": "Bearer " + self.cfg.token_provider()} if self.cfg.token_provider else {}
        try:
            r = self.client.get(f"/internal/v1/platform/exporters/{self._source_of(partition)}/partitions/"
                                f"{partition}/cursor", headers=headers)
        except httpx.HTTPError:
            return None
        return dict(r.json()) if r.status_code == 200 else None

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
        body = self._body(spec)
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
        headers = {"Idempotency-Key": p.idem_key, "Content-Type": "application/json"}
        if self.cfg.token_provider:
            headers["Authorization"] = "Bearer " + self.cfg.token_provider()
        for attempt in range(self.cfg.max_retries + 1):
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
