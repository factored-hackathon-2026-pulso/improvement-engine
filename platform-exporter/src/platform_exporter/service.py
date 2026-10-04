"""Platform exporter: read-only cursor over `event_log` -> `PlatformBatch` (pulso-observations-2) -> POST.

Pattern of the core exporter: persist the batch before the POST, re-POST the stored bytes until a durable ACK, commit
delta + cursor in one local transaction, `Idempotency-Key = batch_digest = sha256(JCS(body))`, cursor authority is the
Pulso cursor endpoint when local state is lost. Platform specifics: kind=platform_event, source_sequence =
event_log.sequence, available_at = ingested_at, closed catalog with quarantine, gap_suspected + backfill request,
late-event window revision, simulator rows tagged team_generated, capability profile manifest."""

from __future__ import annotations

import random
import time
from collections.abc import Callable
from dataclasses import dataclass, field, replace
from datetime import UTC, datetime, timedelta
from typing import Any

import httpx
import rfc8785

from .asof import CaseState, reconstruct_cases
from .catalog import (CATALOG_VERSION, FINDING_SEVERITY, FREE_TEXT_PAYLOAD_KEYS, DENIED_EVENT_TYPES, KNOWN_EVENT_TYPES, PLANNED_PREFIXES, SOURCE_NAMESPACE,
                      fmt_ts, jcs_digest, parse_ts, treat_payload)
from .config import ARTIFACTS_PATH, CONTRACT, OBSERVATIONS_PATH, ExporterConfig
from .profile import build_profile
from .source import RawEvent, SchemaDrift, SqlSource
from .state import ExporterState, Pending

ArtifactRef = dict[str, str]


class SimulatedCrash(RuntimeError):
    """Raised by test hooks to emulate a process death at a precise point."""


class _UploadFailed(RuntimeError):
    pass


@dataclass
class PollReport:
    batches_sent: int = 0
    events_sent: int = 0
    duplicate_acks: int = 0
    deferred: int = 0
    stopped: dict[str, str] = field(default_factory=dict)
    errors: list[str] = field(default_factory=list)
    unknown_event_types: dict[str, int] = field(default_factory=dict)
    denied_event_types: dict[str, int] = field(default_factory=dict)
    gaps: list[tuple[int, int]] = field(default_factory=list)
    backfill_requests: list[tuple[int, int]] = field(default_factory=list)
    late_events: list[str] = field(default_factory=list)
    quality_findings: int = 0


@dataclass
class Spec:
    mode: str  # fast_poll | rescan
    from_seq: int | None
    to_seq: int | None
    cursor: str
    events: list[dict[str, Any]]
    delta: dict[str, Any]
    info: dict[str, Any]


class Exporter:
    def __init__(self, cfg: ExporterConfig, source: SqlSource, state: ExporterState, client: httpx.Client,
                 clock: Callable[[], datetime] | None = None, sleep: Callable[[float], None] = time.sleep,
                 rng: random.Random | None = None) -> None:
        self.cfg, self.source, self.state, self.client = cfg, source, state, client
        self.clock = clock or (lambda: datetime.now(UTC))
        self.sleep = sleep
        self.rng = rng or random.Random()
        self.known_types = KNOWN_EVENT_TYPES | cfg.extra_event_types
        self.crash_after_ack: Callable[[], None] | None = None
        self.crash_before_post: Callable[[], None] | None = None
        self._stale = False
        self._schema_ref: ArtifactRef | None = cfg.source_schema_ref
        self._cases: dict[str, str] = {}
        self._simulators: set[str] = set()
        self._dims_loaded = False

    def close(self) -> None:
        self.state.close()
        self.source.close()

    # ------------------------------------------------------------------ public API
    def poll_once(self) -> PollReport:
        rep = PollReport()
        try:
            self._resume_pending(rep)
            self._drain(rep)
        except SchemaDrift as exc:
            rep.errors.append(exc.code)
        rep.stopped = self.state.stopped()
        rep.backfill_requests = self.state.open_backfills(self.cfg.partition)
        return rep

    def rescan(self) -> PollReport:
        """Anti-entropy that never advances the checkpoint: re-emit the capability profile, the dimension snapshot and
        per-case `turns.sequence` completeness findings."""
        rep = PollReport()
        try:
            self._resume_pending(rep)
            if not self._blocked():
                self._load_dims(force=True)
                spec = self._build_rescan()
                if spec is not None:
                    self._deliver(spec, rep)
        except (SchemaDrift, _UploadFailed) as exc:
            rep.errors.append(getattr(exc, "code", "upload_failed"))
        rep.stopped = self.state.stopped()
        return rep

    def backfill(self, lo: int, hi: int) -> PollReport:
        """Re-read sequences lo..hi (inclusive) and emit them as `late` events; the server dedupes by event_id."""
        rep = PollReport()
        self._resume_pending(rep)
        pos = lo
        while pos <= hi and not self._blocked() and not self.state.is_stopped(self.cfg.partition):
            rows = self.source.events_between(pos, hi, self.cfg.batch_max_events)
            if not rows:
                break
            spec = self._fit(rows, "rescan", [], force_late=True, clear=True)
            if spec is None or not self._deliver(spec, rep):
                break
            pos = rows[-1].sequence + 1
        rep.stopped = self.state.stopped()
        rep.backfill_requests = self.state.open_backfills(self.cfg.partition)
        return rep

    def case_extract(self, cutoff: datetime) -> dict[str, CaseState]:
        """Cases as of `cutoff`, rebuilt from events only (never from the mutable `cases` row)."""
        return reconstruct_cases(self.source.iter_events(), cutoff)

    def capability_profile(self) -> dict[str, Any]:
        return build_profile(self.source, self.cfg.extra_event_types)

    # ------------------------------------------------------------------ dimensions
    def _load_dims(self, force: bool = False) -> None:
        if self._dims_loaded and not force:
            return
        self._cases, self._simulators = self.source.case_customers(), self.source.simulator_customers()
        self._dims_loaded = True

    def _is_simulator(self, ev: RawEvent) -> bool:
        if ev.case_id and ev.case_id not in self._cases and self._dims_loaded:
            self._load_dims(force=True)  # a case created after the last snapshot: refresh once per unknown id
        cust = self._cases.get(ev.case_id or "")
        if cust is None and isinstance(ev.payload, dict):
            cust = ev.payload.get("customer_id")
        return cust in self._simulators

    # ------------------------------------------------------------------ observations
    def _schema(self) -> ArtifactRef:
        if self._schema_ref is None:
            doc = {"schema": "platform_live-event", "catalog_version": CATALOG_VERSION,
                   "event_types": sorted(self.known_types), "available_at": "ingested_at"}
            self._schema_ref = self._upload(doc, jcs_digest(doc))
        return self._schema_ref

    def _obs(self, native_id: str, se: dict[str, Any], seq: int | None, marker: str | None,
             observed_at: str | None = None) -> dict[str, Any]:
        return {"kind": "platform_event", "level": None, "source_event": se, "native_event_id": native_id,
                "source_event_digest": jcs_digest(se), "source_event_ref": None, "source_schema_ref": self._schema(),
                "source_run_ref": None, "source_sequence": seq, "episode_ref": None, "goal_ref": None,
                "layer_mapping_ref": None, "observed_at": observed_at or fmt_ts(self.clock()), "trace_refs": [],
                "coverage_marker": marker}

    def _base(self, event_type: str) -> dict[str, Any]:
        return {"event_type": event_type, "source_namespace": SOURCE_NAMESPACE, "tenant_id": self.cfg.tenant_id,
                "catalog_version": CATALOG_VERSION}

    def _finding(self, fid: str, name: str, seq: int | None, data: dict[str, Any]) -> dict[str, Any]:
        if self.cfg.legacy_prefix:  # contract 1.0.0 interim shape
            se = {**self._base("exporter.finding"), "finding": {"type": name, **data}}
            return self._obs(f"finding:{fid}", se, seq, None)
        return self._meta_obs(f"finding:{fid}", name, data, described=data.get("event_id"), seq=seq)

    def _meta_obs(self, native_id: str, code: str, details: dict[str, Any], *, described: str | None = None,
                  seq: int | None = None) -> dict[str, Any]:
        """Contract 1.1.0 exporter metadata: kind=exporter_finding, no event_type, identity on the envelope. The body
        carries no wall-clock value, so re-emission (rescan, re-POST) keeps the same digest."""
        se = {"kind": "exporter_finding", "source_namespace": SOURCE_NAMESPACE, "catalog_version": CATALOG_VERSION,
              "tenant_id": self.cfg.tenant_id, "finding_code": code, "severity": FINDING_SEVERITY.get(code, "warning"),
              "described_native_event_id": described, "described_source_sequence": seq if described else None,
              "details": details}
        return self._obs(native_id, se, seq, None)

    @staticmethod
    def _is_quality_finding(obs: dict[str, Any]) -> bool:
        se = obs["source_event"]
        if se.get("kind") == "exporter_finding":
            return se["finding_code"] not in ("capability_profile", "dimension_snapshot")
        return se.get("event_type") == "exporter.finding"

    def _window(self, event_time: datetime) -> tuple[datetime, datetime]:
        w = self.cfg.window_seconds
        start = datetime.fromtimestamp((int(event_time.timestamp()) // w) * w, UTC)
        return start, start + timedelta(seconds=w)

    def _is_late(self, ev: RawEvent) -> tuple[bool, datetime, datetime]:
        assert ev.event_time is not None and ev.ingested_at is not None
        start, end = self._window(ev.event_time)
        return ev.ingested_at > end + timedelta(seconds=self.cfg.allowed_lateness_seconds), start, end

    def _stub(self, ev: RawEvent) -> dict[str, Any]:
        """Payload-free continuity marker of a row the exporter withholds (type not admitted). The real ingest keeps
        findings out of source-sequence continuity and quarantines the type itself, so the sequence slot stays covered
        without a hole; nothing but the type, id, sequence and times leaves."""
        assert ev.event_time is not None and ev.ingested_at is not None
        se = {**({} if self.cfg.legacy_prefix else {"kind": "domain_event"}), **self._base(ev.event_type),
              "event_id": ev.event_id,
              "event_time": fmt_ts(ev.event_time), "ingested_at": fmt_ts(ev.ingested_at),
              "available_at": fmt_ts(ev.ingested_at), "case_id": None, "entity": None, "entity_id": None,
              "actor_role": None, "actor_ref": None, "payload": {}, "redacted_fields": [],
              "evidence_kind": "observed", "population_excluded": False, "late": False}
        return self._obs(ev.event_id, se, ev.sequence, None, observed_at=fmt_ts(ev.ingested_at))

    def _withheld_hole(self, seq: int) -> dict[str, Any]:
        return self._finding(f"gap_suspected:{seq}-{seq}", "gap_suspected", None, {
            "from_sequence": seq, "to_sequence": seq, "backfill_requested": False, "verify_with_owner": False,
            "reason": "row_withheld"})

    def _is_deferred_late(self, ev: RawEvent) -> bool:
        return ev.problem is None and ev.event_type in self.known_types and self._is_late(ev)[0]

    def _event_obs(self, ev: RawEvent, force_late: bool, info: dict[str, Any], delta: dict[str, Any]
                   ) -> list[dict[str, Any]]:
        assert ev.event_time is not None and ev.ingested_at is not None
        bump = delta["counters"]
        if ev.problem or ev.event_type not in self.known_types:
            reason = ev.problem or "unknown_event_type"
            delta["quarantined"].append([ev.event_id, ev.sequence, ev.event_type, reason])
            if ev.problem:
                bump["bad_row"] = bump.get("bad_row", 0) + 1
                return [self._finding(f"bad_row:{ev.event_id}", "bad_row", ev.sequence,
                                      {"event_id": ev.event_id, "reason": reason, "sequence": ev.sequence}),
                        self._withheld_hole(ev.sequence)]
            if ev.event_type in DENIED_EVENT_TYPES:
                bump["denied_event_type"] = bump.get("denied_event_type", 0) + 1
                info["denied"][ev.event_type] = info["denied"].get(ev.event_type, 0) + 1
                return [self._finding(f"denied_event_type:{ev.event_id}", "denied_event_type", ev.sequence, {
                    "event_type": ev.event_type, "event_id": ev.event_id, "sequence": ev.sequence,
                    "quarantined": True, "payload_forwarded": False}), self._stub(ev)]
            status = "planned" if ev.event_type.startswith(PLANNED_PREFIXES) else "unknown"
            bump["unknown_event_type"] = bump.get("unknown_event_type", 0) + 1
            info["unknown"][ev.event_type] = info["unknown"].get(ev.event_type, 0) + 1
            return [self._finding(f"unknown_event_type:{ev.event_id}", "unknown_event_type", ev.sequence, {
                "event_type": ev.event_type, "catalog_status": status, "event_id": ev.event_id,
                "sequence": ev.sequence,
                "event_time": fmt_ts(ev.event_time), "ingested_at": fmt_ts(ev.ingested_at),
                "quarantined": True, "payload_forwarded": False}), self._stub(ev)]
        late, wstart, wend = self._is_late(ev)
        redacted: list[str] = []
        sim = self._is_simulator(ev)
        se = {**({} if self.cfg.legacy_prefix else {"kind": "domain_event"}), **self._base(ev.event_type),
              "event_id": ev.event_id, "event_time": fmt_ts(ev.event_time),
              "ingested_at": fmt_ts(ev.ingested_at), "available_at": fmt_ts(ev.ingested_at), "case_id": ev.case_id,
              "entity": ev.entity, "entity_id": ev.entity_id, "actor_role": ev.actor_role, "actor_ref": ev.actor_id,
              "payload": treat_payload(ev.payload, redacted, drop_keys=FREE_TEXT_PAYLOAD_KEYS.get(ev.event_type, frozenset())),
              "redacted_fields": sorted(redacted),
              "evidence_kind": "team_generated" if sim else "observed", "population_excluded": sim, "late": late}
        out = [self._obs(ev.event_id, se, ev.sequence, "late" if (late or force_late) else None,
                         observed_at=fmt_ts(ev.ingested_at))]
        if late:
            info["late"].append(ev.event_id)
            bump["late_event"] = bump.get("late_event", 0) + 1
            out.append(self._finding(f"late_event:{ev.event_id}", "late_event", ev.sequence, {
                "event_id": ev.event_id, "sequence": ev.sequence, "window_start": fmt_ts(wstart),
                "window_end": fmt_ts(wend), "ingested_at": fmt_ts(ev.ingested_at),
                "lag_seconds": int((ev.ingested_at - wend).total_seconds()), "window_revision_required": True}))
        return out

    def _profile_obs(self, always: bool = False) -> tuple[dict[str, Any] | None, dict[str, str]]:
        prof = build_profile(self.source, self.cfg.extra_event_types)
        if not always and self.state.meta("profile_digest") == prof["digest"]:
            return None, {}
        if self.cfg.legacy_prefix:
            se = {**self._base("exporter.capability_profile"), "profile": prof}
            return self._obs(f"profile:{prof['digest']}", se, None, None), {"profile_digest": prof["digest"]}
        return (self._meta_obs(f"profile:{prof['digest']}", "capability_profile", {"profile": prof}),
                {"profile_digest": prof["digest"]})

    # ------------------------------------------------------------------ batch construction
    @staticmethod
    def _cursor_seq(cur: tuple[str | None, int] | None) -> int:
        if cur and cur[0] and cur[0].startswith("s."):
            return int(cur[0][2:])
        return 0

    def _new_delta(self) -> dict[str, Any]:
        return {"quarantined": [], "counters": {}, "meta": {}}

    def _wire_size(self, spec: Spec) -> int:
        return len(rfc8785.dumps({**self._body(spec), "batch_digest": "0" * 64}))

    def _fit(self, rows: list[RawEvent], mode: str, skipped: list[list[int]], *, force_late: bool = False,
             clear: bool = False, extra: list[dict[str, Any]] | None = None, meta: dict[str, str] | None = None,
             ) -> Spec | None:
        taken = rows
        while True:
            spec = self._assemble(taken, mode, skipped, force_late, clear, extra or [], meta or {})
            if self._wire_size(spec) <= self.cfg.batch_max_bytes:
                return spec
            if len(taken) <= 1:
                if taken[0].problem is None:  # a poison row must not stop the partition: quarantine, never forward
                    taken = [replace(taken[0], payload=None, problem="oversized_event")]
                    continue
                return spec
            taken = taken[: len(taken) // 2]

    def _assemble(self, rows: list[RawEvent], mode: str, skipped: list[list[int]], force_late: bool, clear: bool,
                  extra: list[dict[str, Any]], meta: dict[str, str]) -> Spec:
        info: dict[str, Any] = {"unknown": {}, "denied": {}, "late": [], "gaps": []}
        delta = self._new_delta()
        last = rows[-1].sequence
        events: list[dict[str, Any]] = []
        sk = [r for r in skipped if r[1] < last]
        for lo, hi in sk:
            info["gaps"].append((lo, hi))
            delta["counters"]["gap_suspected"] = delta["counters"].get("gap_suspected", 0) + 1
            events.append(self._finding(f"gap_suspected:{lo}-{hi}", "gap_suspected", None, {
                "from_sequence": lo, "to_sequence": hi, "backfill_requested": True, "verify_with_owner": True}))
        deferred: list[list[int]] = []
        for ev in rows:
            if mode == "fast_poll" and self._is_deferred_late(ev):
                # The real ingest keeps late rows out of source-sequence continuity: declare the slot as a hole now and
                # deliver the row through the late path, which closes it (one backfill request, no suspected loss).
                deferred.append([ev.sequence, ev.sequence])
                events.append(self._finding(f"gap_suspected:{ev.sequence}-{ev.sequence}", "gap_suspected", None, {
                    "from_sequence": ev.sequence, "to_sequence": ev.sequence, "backfill_requested": True,
                    "verify_with_owner": False, "reason": "late_row_deferred"}))
                continue
            events.extend(self._event_obs(ev, force_late, info, delta))
        events.extend(extra)
        delta["meta"].update(meta)
        if mode == "fast_poll":
            delta["holes_skipped"], delta["holes_unskipped_below"] = sk + deferred, last
            prefix = "s"
        else:
            prefix = "late"
            if clear:
                delta["holes_cleared"] = [r.sequence for r in rows]
        return Spec(mode, rows[0].sequence, last, f"{prefix}.{last}", events, delta, info)

    def _build_fast(self) -> Spec | None:
        cur = self._reconciled_cursor()
        after = max(self._cursor_seq(cur), self.cfg.start_sequence - 1)
        self._load_dims()
        rows = self.source.events_after(after, self.cfg.batch_max_events)
        prof, prof_meta = self._profile_obs()
        now = self.clock().timestamp()
        taken: list[RawEvent] = []
        skipped: list[list[int]] = []
        expected = after + 1
        for row in rows:
            if row.sequence != expected:
                lo, hi = expected, row.sequence - 1  # one range, however wide the jump
                first = self.state.note_hole_range(self.cfg.partition, lo, hi, now)
                if now - first < self.cfg.gap_grace_seconds:
                    break  # hold-back: the hole may still commit
                skipped.append([lo, hi])
            taken.append(row)
            expected = row.sequence + 1
        if not taken:
            if prof is None:
                return None
            cursor = cur[0] or "s.0"  # profile-only batch: no rows, the cursor does not move
            return Spec("fast_poll", None, None, cursor, [prof], {"meta": prof_meta, "counters": {}}, {
                "unknown": {}, "denied": {}, "late": [], "gaps": []})
        return self._fit(taken, "fast_poll", skipped, extra=[prof] if prof else None, meta=prof_meta)

    def _build_late(self) -> Spec | None:
        ranges = self.state.open_backfills(self.cfg.partition)
        for lo, hi in ranges:
            rows = self.source.events_between(lo, hi, self.cfg.batch_max_events)
            if rows:
                self._load_dims()
                return self._fit(rows, "rescan", [], force_late=True, clear=True)
        return None

    def _build_rescan(self) -> Spec | None:
        extra: list[dict[str, Any]] = []
        prof, meta = self._profile_obs(always=True)
        assert prof is not None
        extra.append(prof)
        dim = {"cases": len(self._cases), "simulator_customers": len(self._simulators),
               "staff": jcs_digest(self.source.staff_dimension()), "cases_digest": jcs_digest(sorted(self._cases.items())),
               "simulator_digest": jcs_digest(sorted(self._simulators))}
        if self.cfg.legacy_prefix:
            se = {**self._base("exporter.dimension_snapshot"), "snapshot": dim}
            extra.append(self._obs(f"dimensions:{jcs_digest(dim)}", se, None, None))
        else:
            extra.append(self._meta_obs(f"dimensions:{jcs_digest(dim)}", "dimension_snapshot", dim))
        for case_id, missing in sorted(self.source.turn_sequence_gaps().items()):
            extra.append(self._finding(f"turn_sequence_gap:{case_id}:{missing[0]}-{missing[-1]}",
                                       "turn_sequence_gap", None, {"case_id": case_id, "missing": missing}))
        return Spec("rescan", None, None, "r.0", extra, {"meta": meta, "counters": {}}, {
            "unknown": {}, "denied": {}, "late": [], "gaps": []})

    # ------------------------------------------------------------------ loop
    def _blocked(self) -> bool:
        p = self.state.pending()
        return p is not None and not self.state.is_stopped(p.partition)

    def _drain(self, rep: PollReport) -> None:
        stale_retries = 0
        for _ in range(10_000):
            part = self.cfg.partition
            if self.state.is_stopped(part):
                return
            if self._blocked():
                rep.deferred += 1
                return
            try:
                spec = self._build_late() or self._build_fast()
            except _UploadFailed:
                rep.deferred += 1
                return
            if spec is None:
                return
            self._stale = False
            if not self._deliver(spec, rep):
                if self._stale and stale_retries < 3:
                    stale_retries += 1
                    continue
                return

    # ------------------------------------------------------------------ cursor authority
    def _reconciled_cursor(self) -> tuple[str | None, int]:
        part = self.cfg.partition
        cur = self.state.cursor(part)
        if cur is not None:
            return cur
        remote = self._get_cursor(strict=True)
        if remote is None:
            return (None, 0)
        if remote["cursor_revision"] > 0:  # local state lost: the Pulso cursor wins
            self.state.set_meta("rebuild", "1")
        self.state.set_cursor(part, remote["cursor"], int(remote["cursor_revision"]))
        return (remote["cursor"], int(remote["cursor_revision"]))

    def _get_cursor(self, strict: bool = False) -> dict[str, Any] | None:
        headers: dict[str, str] = {}
        tok = self.cfg.token("cursor")
        if tok:
            headers["Authorization"] = "Bearer " + tok
        try:
            r = self.client.get(f"/internal/v1/platform/exporters/{self.cfg.source_id}/partitions/"
                                f"{self.cfg.partition}/cursor", headers=headers)
        except httpx.HTTPError as exc:
            if strict:
                raise _UploadFailed(str(exc)) from exc
            return None
        if r.status_code == 200:
            return dict(r.json())
        if strict:
            raise _UploadFailed(f"cursor status {r.status_code}")
        return None

    # ------------------------------------------------------------------ transport
    def _body(self, spec: Spec) -> dict[str, Any]:
        rev: int | None = None
        if spec.mode == "fast_poll":
            cur = self.state.cursor(self.cfg.partition) or self._reconciled_cursor()
            rev = cur[1]
        return {"contract_version": CONTRACT, "source_id": self.cfg.source_id, "tenant_id": self.cfg.tenant_id,
                "partition": self.cfg.partition, "scan_mode": spec.mode, "expected_cursor_revision": rev,
                "from_seq": spec.from_seq, "to_seq": spec.to_seq, "cursor": spec.cursor, "cut_ref": None,
                "events": spec.events, "verification_receipts": []}

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
        raw = rfc8785.dumps({**body, "batch_digest": digest})
        if len(raw) > self.cfg.batch_max_bytes:
            self.state.stop(self.cfg.partition, "batch_too_large")
            return False
        p = Pending(self.cfg.partition, spec.mode, digest, raw, spec.delta)
        self.state.save_pending(p)
        rep.events_sent += len(spec.events)
        if not self._send_pending(p, rep):
            return False
        for name, n in spec.info["unknown"].items():
            rep.unknown_event_types[name] = rep.unknown_event_types.get(name, 0) + n
        for name, n in spec.info["denied"].items():
            rep.denied_event_types[name] = rep.denied_event_types.get(name, 0) + n
        rep.gaps.extend(spec.info["gaps"])
        rep.late_events.extend(spec.info["late"])
        rep.quality_findings += sum(1 for e in spec.events if self._is_quality_finding(e))
        return True

    def _resume_pending(self, rep: PollReport) -> None:
        p = self.state.pending()
        if p is not None and not self.state.is_stopped(p.partition):
            self._send_pending(p, rep)

    def _send_pending(self, p: Pending, rep: PollReport) -> bool:
        if self.crash_before_post:
            self.crash_before_post()
        out = self._post(p)
        if out is None:
            rep.deferred += 1
            return False
        status, ack = out
        if ack.get("batch_digest") != p.idem_key:
            self.state.stop(p.partition, "ack_digest_mismatch")
            return False
        if ack.get("disposition") == "quarantined":  # the server kept the batch aside and did not move the checkpoint
            reason = str(ack.get("quarantine_reason") or "quarantined")
            self.state.quarantine_batch(p.idem_key, p.partition, reason, p.body)
            self.state.stop(p.partition, f"quarantined:{reason}")
            return False
        if status == 200:
            rep.duplicate_acks += 1
        if self.crash_after_ack:
            self.crash_after_ack()
        self.state.commit_ack(p, ack)
        rep.batches_sent += 1
        return True

    def _post(self, p: Pending) -> tuple[int, dict[str, Any]] | None:
        for attempt in range(self.cfg.max_retries + 1):
            headers = {"Idempotency-Key": p.idem_key, "Content-Type": "application/json"}
            tok = self.cfg.token("observations")  # a fresh token (new jti) for every HTTP attempt
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
                self.state.drop_pending()
                remote = self._get_cursor()
                if remote:
                    self.state.set_cursor(p.partition, remote["cursor"], int(remote["cursor_revision"]))
                self._stale = True
                return None
            if code == 409:
                self.state.stop(p.partition, f"digest_conflict:{err}")
            elif code == 422:
                self.state.quarantine_batch(p.idem_key, p.partition, err or "422", p.body)
                self.state.stop(p.partition, f"quarantined:{err or '422'}")
            else:
                self.state.stop(p.partition, f"http_{code}")
            return None
        return None

    def _backoff(self, attempt: int, retry_after: str | None) -> None:
        base = float(retry_after) if retry_after and retry_after.replace(".", "", 1).isdigit() else 2.0 ** attempt
        self.sleep(min(self.cfg.backoff_cap_seconds, base) + self.rng.uniform(0, 0.25))

    def _upload(self, content: Any, digest: str) -> ArtifactRef:
        body = {"schema_version": "1", "binding_ref": self.cfg.binding_ref, "source_schema_ref": None,
                "classification": "treated", "information_partition": self.cfg.partition, "artifact_kind": "schema",
                "media_type": "application/json", "encoding": "json", "content": content, "content_digest": digest}
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
                or ref["media_type"] != "application/json" or not isinstance(ref["id"], str) or not ref["id"]):
            raise _UploadFailed("untrusted artifact ref")
        return {"id": ref["id"], "digest": ref["digest"], "media_type": ref["media_type"]}


__all__ = ["Exporter", "PollReport", "SimulatedCrash", "parse_ts"]
