"""Deterministic simulator of the real support platform's data model (Phase 1).

Business rules reproduced from the Product artifact (commit a492bfa): one open case per
customer (customer_case_slots), closed cases never reopen (a new case points back through
previous_case_id), H1 language rule (Portuguese cases only to Portuguese speakers, least open
cases among the eligible), SLA 5/15/60 minutes by priority, mutable cases with optimistic
`version`, append-only turns/assignments/event_log, global contiguous event_log.sequence with
event_time and ingested_at.

NOTE: event payload shapes are simulator assumptions (the artifact does not publish them);
they are deliberately minimal and carry no message text.
"""

from __future__ import annotations

import json
import random
import sqlite3
from datetime import datetime, timedelta, timezone

from .ddl import append_only_guards_sqlite, render_ddl

SLA_SECONDS = {"high": 300, "medium": 900, "low": 3600}
LOCALES = ["es-CO", "es-MX", "es-AR", "pt-BR"]
COUNTRY = {"es-CO": "CO", "es-MX": "MX", "es-AR": "AR", "pt-BR": "BR"}
CLOSE_REASONS = ["resolved", "customer_unresponsive", "duplicate", "out_of_scope", "other"]
CLOSE_WEIGHTS = [60, 15, 8, 10, 7]
START = "2026-03-02T09:00:00Z"
_OPEN = ("assigned", "in_progress")


class SlotOccupied(Exception):
    """The customer already has an open case (customer_case_slots rule)."""


def _iso(dt: datetime) -> str:
    return dt.astimezone(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def _parse(s: str) -> datetime:
    return datetime.fromisoformat(s.replace("Z", "+00:00"))


class PlatformLiveSim:
    def __init__(self, seed: int = 0, path: str = ":memory:", start: str = START, n_customers: int = 12):
        self.rng = random.Random(seed)
        self.conn = sqlite3.connect(path)
        self.now = _parse(start)
        self.faults: list[dict] = []
        self._ids: dict[str, int] = {}
        self._next_seq = 1
        for stmt in render_ddl("sqlite"):
            self.conn.execute(stmt)
        self._seed_people(n_customers)
        for stmt in append_only_guards_sqlite():
            self.conn.execute(stmt)
        self.conn.commit()

    # ---- helpers -------------------------------------------------------------------------
    def _id(self, prefix: str) -> str:
        self._ids[prefix] = self._ids.get(prefix, 0) + 1
        return f"{prefix}-{self._ids[prefix]:04d}"

    def _iso_now(self) -> str:
        return _iso(self.now)

    def advance(self, seconds: float) -> None:
        self.now += timedelta(seconds=seconds)

    def _q(self, sql: str, args=()):
        return self.conn.execute(sql, args)

    def customer_ids(self, locale: str | None = None, simulator: bool | None = None) -> list[str]:
        sql, args = "SELECT id FROM customers WHERE 1=1", []
        if locale is not None:
            sql += " AND locale=?"
            args.append(locale)
        if simulator is not None:
            sql += " AND simulator=?"
            args.append(int(simulator))
        return [r[0] for r in self._q(sql + " ORDER BY id", args)]

    def staff_ids(self, language: str | None = None, role: str = "analyst") -> list[str]:
        out = []
        for sid, roles, langs in self._q("SELECT id, roles, languages FROM staff ORDER BY id"):
            if role in json.loads(roles) and (language is None or language in json.loads(langs)):
                out.append(sid)
        return out

    # ---- seeding -------------------------------------------------------------------------
    def _seed_people(self, n_customers: int) -> None:
        roster = [
            (["supervisor", "admin"], ["es", "pt"], "supervision"),
            (["analyst"], ["es"], "general"),
            (["analyst"], ["es"], "general"),
            (["analyst"], ["es"], "general"),
            (["analyst"], ["es", "pt"], "portuguese"),
            (["analyst"], ["es", "pt"], "portuguese"),
        ]
        since = self._iso_now()
        for roles, langs, team in roster:
            sid = self._id("STF")
            self._q("INSERT INTO staff (id, name, email, roles, languages, team, active, version) "
                    "VALUES (?,?,?,?,?,?,1,1)",
                    (sid, f"Staff {sid}", f"{sid.lower()}@example.invalid", json.dumps(roles),
                     json.dumps(langs), team))
            self._q("INSERT INTO login_accounts (staff_id, password_hash, failed_attempts) VALUES (?,?,0)",
                    (sid, f"FAKE-NOT-A-HASH-{sid}"))
            if "analyst" in roles:
                self._q("INSERT INTO analyst_availability (staff_id, status, since) VALUES (?,?,?)",
                        (sid, "available", since))
        for i in range(n_customers + 2):
            cid = self._id("CUS")
            sim = i >= n_customers
            locale = LOCALES[i % len(LOCALES)]
            self._q("INSERT INTO customers (id, display_name, country, city, locale, simulator, suggestions) "
                    "VALUES (?,?,?,?,?,?,?)",
                    (cid, f"Customer {cid}", COUNTRY[locale], None, locale, int(sim),
                     json.dumps([{"text": "demo suggestion"}]) if sim else None))
            self._q("INSERT INTO customer_case_slots (customer_id, open_case_id, version) VALUES (?,NULL,1)", (cid,))

    # ---- event log -----------------------------------------------------------------------
    def _emit(self, event_type, entity, entity_id, case_id, actor_role, actor_id, payload=None,
              event_time: datetime | None = None, ingested_at: datetime | None = None) -> int:
        et = event_time or self.now
        ing = ingested_at or (et + timedelta(seconds=self.rng.choice([0, 1, 1, 2])))
        seq = self._next_seq
        self._next_seq += 1
        self._q("INSERT INTO event_log (sequence, event_id, event_type, entity, entity_id, case_id, actor_role, "
                "actor_id, event_time, ingested_at, payload) VALUES (?,?,?,?,?,?,?,?,?,?,?)",
                (seq, self._id("EVT"), event_type, entity, entity_id, case_id, actor_role, actor_id,
                 _iso(et), _iso(ing), json.dumps(payload or {}, sort_keys=True)))
        return seq

    # ---- case mutation -------------------------------------------------------------------
    def _touch(self, case_id: str, **fields) -> None:
        cols = ", ".join(f"{k}=?" for k in fields)
        self._q(f"UPDATE cases SET {cols}, version=version+1 WHERE id=?", (*fields.values(), case_id))

    def _case(self, case_id: str) -> sqlite3.Row:
        self.conn.row_factory = sqlite3.Row
        try:
            r = self._q("SELECT * FROM cases WHERE id=?", (case_id,)).fetchone()
        finally:
            self.conn.row_factory = None
        if r is None:
            raise KeyError(case_id)
        return r

    def _add_turn(self, case_id, kind, audience, role, author_id, text) -> int:
        c = self._case(case_id)
        seq = c["last_sequence"] + 1
        tid = self._id("TRN")
        self._q("INSERT INTO turns (id, case_id, sequence, kind, audience, author_role, author_id, text, language, "
                "created_at, client_message_id) VALUES (?,?,?,?,?,?,?,?,?,?,?)",
                (tid, case_id, seq, kind, audience, role, author_id, text, c["language"], self._iso_now(),
                 f"cm-{tid}" if role == "customer" else None))
        upd: dict = {"last_sequence": seq, "last_turn_author_role": role, "last_turn_preview": text[:80]}
        if audience == "everyone":
            upd["last_public_sequence"] = seq
        if kind == "message":
            upd.update(last_message_at=self._iso_now(), last_message_author_role=role,
                       last_message_preview=text[:80], search_text=text.lower()[:200])
            unread = json.loads(c["unread_sequences"])
            if role == "customer":
                unread.append(seq)
            elif role == "analyst":
                unread, upd["assignee_read_sequence"] = [], seq
            upd["unread_sequences"] = json.dumps(unread)
        self._touch(case_id, **upd)
        actor_id = author_id
        self._emit("turn.created", "turn", tid, case_id, role, actor_id,
                   {"sequence": seq, "kind": kind, "audience": audience})
        return seq

    def _eligible(self, language: str) -> list[tuple[int, str]]:
        out = []
        for sid, status in self._q("SELECT s.id, a.status FROM staff s JOIN analyst_availability a "
                                   "ON a.staff_id=s.id WHERE s.active=1 ORDER BY s.id"):
            langs = json.loads(self._q("SELECT languages FROM staff WHERE id=?", (sid,)).fetchone()[0])
            if status == "available" and language in langs:
                load = self._q("SELECT COUNT(*) FROM cases WHERE assigned_analyst_id=? AND status IN "
                               "('assigned','in_progress')", (sid,)).fetchone()[0]
                out.append((load, sid))
        return sorted(out)

    def _try_assign(self, case_id: str, reason: str) -> bool:
        c = self._case(case_id)
        elig = self._eligible(c["language"])
        if not elig:
            return False
        load, sid = elig[0]
        waited = int((self.now - _parse(c["queued_at"])).total_seconds()) if reason == "queue_drained" else 0
        self._q("INSERT INTO assignments (id, case_id, staff_id, reason, policy_rule_id, open_cases_at_assignment, "
                "strategy, assigned_by_role, assigned_by_id, waited_seconds, previous_staff_id, paused_override, "
                "assigned_at) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?)",
                (self._id("ASG"), case_id, sid, reason, "H1" if c["language"] == "pt" else None, load,
                 "language_least_loaded", "system", None, waited, None, 0, self._iso_now()))
        self._touch(case_id, status="assigned", assigned_analyst_id=sid, assigned_at=self._iso_now(),
                    queue_label=None)
        self._emit("case.assigned", "case", case_id, case_id, "system", None,
                   {"staff_id": sid, "reason": reason})
        self._emit("case.status_changed", "case", case_id, case_id, "system", None,
                   {"from": "queued", "to": "assigned"})
        self._add_turn(case_id, "routing", "staff", "system", None, f"assigned to {sid}")
        return True

    def _drain_queue(self) -> None:
        for (cid,) in self._q("SELECT id FROM cases WHERE status='queued' ORDER BY queued_at, id").fetchall():
            self._try_assign(cid, "queue_drained")

    # ---- public operations ---------------------------------------------------------------
    def open_case(self, customer_id: str, priority: str = "medium", channel: str | None = None,
                  text: str | None = None) -> str:
        locale = self._q("SELECT locale FROM customers WHERE id=?", (customer_id,)).fetchone()[0]
        if self._q("SELECT open_case_id FROM customer_case_slots WHERE customer_id=?",
                   (customer_id,)).fetchone()[0] is not None:
            raise SlotOccupied(customer_id)
        lang = "pt" if locale.startswith("pt") else "es"
        prev = self._q("SELECT id FROM cases WHERE customer_id=? AND status='closed' "
                       "ORDER BY opened_at DESC, id DESC LIMIT 1", (customer_id,)).fetchone()
        cid = self._id("CASE")
        now = self._iso_now()
        self._q("INSERT INTO cases (id, customer_id, channel, language, priority, status, opened_at, sla_due_at, "
                "previous_case_id, queued_at, queue_label, unread_sequences) VALUES (?,?,?,?,?,?,?,?,?,?,?,?)",
                (cid, customer_id, channel or self.rng.choice(["app_chat", "web_chat"]), lang, priority,
                 "queued", now, _iso(self.now + timedelta(seconds=SLA_SECONDS[priority])),
                 prev[0] if prev else None, now, lang, "[]"))
        self._q("UPDATE customer_case_slots SET open_case_id=?, version=version+1 WHERE customer_id=?",
                (cid, customer_id))
        self._emit("case.opened", "case", cid, cid, "customer", customer_id, {"priority": priority})
        self._add_turn(cid, "message", "everyone", "customer", customer_id, text or "hola, necesito ayuda")
        if not self._try_assign(cid, "language_least_loaded"):
            self._emit("case.queued", "case", cid, cid, "system", None, {"queue_label": lang})
        self.conn.commit()
        return cid

    def customer_message(self, case_id: str, text: str) -> None:
        c = self._case(case_id)
        self._add_turn(case_id, "message", "everyone", "customer", c["customer_id"], text)
        self.conn.commit()

    def analyst_reply(self, case_id: str, text: str) -> None:
        c = self._case(case_id)
        if c["status"] not in _OPEN:
            raise ValueError(f"case {case_id} is {c['status']}, no analyst assigned")
        who = c["assigned_analyst_id"]
        self._add_turn(case_id, "message", "everyone", "analyst", who, text)
        if c["first_response_at"] is None:
            self._touch(case_id, first_response_at=self._iso_now())
            self._emit("case.first_responded", "case", case_id, case_id, "analyst", who, {})
        if c["status"] == "assigned":
            self._touch(case_id, status="in_progress")
            self._emit("case.status_changed", "case", case_id, case_id, "analyst", who,
                       {"from": "assigned", "to": "in_progress"})
        self.conn.commit()

    def mark_read(self, case_id: str) -> None:
        c = self._case(case_id)
        self._touch(case_id, assignee_read_sequence=c["last_sequence"], unread_sequences="[]")
        self._emit("case.read", "case", case_id, case_id, "analyst", c["assigned_analyst_id"], {})
        self.conn.commit()

    def view_case(self, case_id: str, supervisor_id: str | None = None) -> None:
        sup = supervisor_id or self.staff_ids(role="supervisor")[0]
        self._emit("case.viewed", "case", case_id, case_id, "supervisor", sup, {})
        self.conn.commit()

    def reassign(self, case_id: str, to_staff: str, supervisor_id: str | None = None) -> None:
        c = self._case(case_id)
        if c["status"] not in _OPEN or c["assigned_analyst_id"] == to_staff:
            raise ValueError("case not reassignable to that analyst")
        sup = supervisor_id or self.staff_ids(role="supervisor")[0]
        status = self._q("SELECT status FROM analyst_availability WHERE staff_id=?", (to_staff,)).fetchone()[0]
        load = self._q("SELECT COUNT(*) FROM cases WHERE assigned_analyst_id=? AND status IN "
                       "('assigned','in_progress')", (to_staff,)).fetchone()[0]
        self._q("INSERT INTO assignments (id, case_id, staff_id, reason, policy_rule_id, open_cases_at_assignment, "
                "strategy, assigned_by_role, assigned_by_id, waited_seconds, previous_staff_id, paused_override, "
                "assigned_at) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?)",
                (self._id("ASG"), case_id, to_staff, "manual", None, load, "manual", "supervisor", sup, None,
                 c["assigned_analyst_id"], int(status == "paused"), self._iso_now()))
        self._touch(case_id, status="assigned", assigned_analyst_id=to_staff, assigned_at=self._iso_now())
        self._emit("case.assigned", "case", case_id, case_id, "supervisor", sup,
                   {"staff_id": to_staff, "reason": "manual", "previous_staff_id": c["assigned_analyst_id"]})
        if c["status"] != "assigned":
            self._emit("case.status_changed", "case", case_id, case_id, "supervisor", sup,
                       {"from": c["status"], "to": "assigned"})
        self._add_turn(case_id, "notice", "staff", "system", None, f"reassigned to {to_staff}")
        self.conn.commit()

    def set_availability(self, staff_id: str, status: str) -> None:
        self._q("UPDATE analyst_availability SET status=?, since=? WHERE staff_id=?",
                (status, self._iso_now(), staff_id))
        self._emit("staff.availability_changed", "staff", staff_id, None, "analyst", staff_id, {"status": status})
        if status == "available":
            self._drain_queue()
        self.conn.commit()

    def close_case(self, case_id: str, reason: str = "resolved", note: str | None = None) -> None:
        c = self._case(case_id)
        if c["status"] == "closed":
            raise ValueError("already closed")
        by = c["assigned_analyst_id"]
        role = "analyst"
        if by is None:
            by, role = self.staff_ids(role="supervisor")[0], "supervisor"
        self._touch(case_id, status="closed", closed_at=self._iso_now(), closed_by_id=by, closed_by_role=role,
                    close_reason=reason, close_note=note)
        self._q("UPDATE customer_case_slots SET open_case_id=NULL, version=version+1 WHERE customer_id=?",
                (c["customer_id"],))
        self._emit("case.status_changed", "case", case_id, case_id, role, by, {"from": c["status"], "to": "closed"})
        self._emit("case.closed", "case", case_id, case_id, role, by, {"close_reason": reason})
        self._add_turn(case_id, "notice", "everyone", "system", None, "case closed")
        self.conn.commit()

    def auth_event(self, event_type: str, staff_id: str) -> None:
        self._emit(event_type, "staff", staff_id, None, "system", staff_id, {})
        self.conn.commit()

    # ---- fault injection -----------------------------------------------------------------
    def inject_late_event(self, case_id: str | None = None, lag_seconds: float = 3600,
                          event_type: str = "case.read") -> None:
        """An event that happened lag_seconds ago but only reached the log now."""
        seq = self._emit(event_type, "case", case_id, case_id, "analyst", None, {"late": True},
                         event_time=self.now - timedelta(seconds=lag_seconds), ingested_at=self.now)
        self.faults.append({"kind": "late_event", "sequence": seq, "lag_seconds": lag_seconds})
        self.conn.commit()

    def inject_sequence_gap(self, size: int = 1) -> None:
        """Simulates a rolled-back transaction: `size` sequence values are never used."""
        self.faults.append({"kind": "sequence_gap", "after": self._next_seq - 1, "size": size})
        self._next_seq += size

    def inject_turn_gap(self, case_id: str) -> None:
        """Skips one turns.sequence value in the case, then appends a notice turn."""
        c = self._case(case_id)
        self._touch(case_id, last_sequence=c["last_sequence"] + 1)
        self._add_turn(case_id, "notice", "staff", "system", None, "after a skipped sequence")
        self.faults.append({"kind": "turn_gap", "case_id": case_id})
        self.conn.commit()

    def inject_unknown_event_type(self, event_type: str = "case.escalated") -> None:
        seq = self._emit(event_type, "case", None, None, "system", None, {"simulated": True})
        self.faults.append({"kind": "unknown_event_type", "sequence": seq, "event_type": event_type})
        self.conn.commit()

    def evolve_teams(self) -> None:
        """Planned Product evolution: teams table, staff.team -> staff.team_id, admin_roster, staff/team events."""
        self._q("CREATE TABLE teams (id TEXT PRIMARY KEY, name TEXT NOT NULL UNIQUE, active INTEGER NOT NULL DEFAULT 1)")
        self._q("CREATE TABLE admin_roster (staff_id TEXT PRIMARY KEY REFERENCES staff(id), since TEXT NOT NULL)")
        self._q("ALTER TABLE staff ADD COLUMN team_id TEXT")
        team_ids: dict[str, str] = {}
        for (name,) in self._q("SELECT DISTINCT team FROM staff ORDER BY team").fetchall():
            tid = self._id("TEAM")
            team_ids[name] = tid
            self._q("INSERT INTO teams (id, name, active) VALUES (?,?,1)", (tid, name))
            self._emit("team.created", "team", tid, None, "admin", None, {"name": name})
        for sid, team, roles in self._q("SELECT id, team, roles FROM staff ORDER BY id").fetchall():
            self._q("UPDATE staff SET team_id=?, version=version+1 WHERE id=?", (team_ids[team], sid))
            self._emit("staff.team_changed", "staff", sid, None, "admin", None, {"team_id": team_ids[team]})
            if "admin" in json.loads(roles):
                self._q("INSERT INTO admin_roster (staff_id, since) VALUES (?,?)", (sid, self._iso_now()))
        self._q("ALTER TABLE staff DROP COLUMN team")
        self.faults.append({"kind": "teams_evolution"})
        self.conn.commit()

    # ---- scenario generator --------------------------------------------------------------
    def generate(self, n_cases: int = 30, faults: bool = False) -> list[str]:
        rng = self.rng
        pt_staff = self.staff_ids(language="pt")
        sup = self.staff_ids(role="supervisor")[0]
        analysts = self.staff_ids()
        customers = self.customer_ids()
        marks = ({n_cases // 5: "late", n_cases // 3: "gap", n_cases // 2: "unknown",
                  (2 * n_cases) // 3: "turn_gap", (3 * n_cases) // 4: "teams"} if faults else {})
        made: list[str] = []
        for i in range(n_cases):
            self.advance(rng.randint(5, 90))
            free = [c for c in customers
                    if self._q("SELECT open_case_id FROM customer_case_slots WHERE customer_id=?", (c,)).fetchone()[0] is None]
            if not free:
                for (cid,) in self._q("SELECT id FROM cases WHERE status!='closed' ORDER BY id").fetchall():
                    self.close_case(cid, "other")
                free = list(customers)
            cus = rng.choice(free)
            blackout = rng.random() < 0.2
            if blackout:
                for s in pt_staff:
                    self.set_availability(s, "paused")
            cid = self.open_case(cus, priority=rng.choice(["low", "medium", "high"]),
                                 text=rng.choice(["hola, necesito ayuda", "oi, preciso de ajuda", "consulta"]))
            made.append(cid)
            self.advance(rng.randint(10, 600))
            if blackout:
                if rng.random() < 0.3:
                    self.close_case(cid, "customer_unresponsive")
                for s in pt_staff:
                    self.set_availability(s, "available")
            if self._case(cid)["status"] in _OPEN:
                for _ in range(rng.randint(1, 3)):
                    self.advance(rng.randint(5, 200))
                    self.analyst_reply(cid, "respuesta del analista")
                    self.advance(rng.randint(5, 200))
                    if rng.random() < 0.7:
                        self.customer_message(cid, "mensaje del cliente")
                if rng.random() < 0.3:
                    self.mark_read(cid)
                if rng.random() < 0.15:
                    self.view_case(cid, sup)
                if rng.random() < 0.2:
                    cur = self._case(cid)["assigned_analyst_id"]
                    langs = "pt" if self._case(cid)["language"] == "pt" else "es"
                    options = [a for a in analysts if a != cur and a in self.staff_ids(language=langs)]
                    if options:
                        self.reassign(cid, rng.choice(options), sup)
                if rng.random() < 0.9:
                    self.advance(rng.randint(30, 900))
                    self.close_case(cid, rng.choices(CLOSE_REASONS, CLOSE_WEIGHTS)[0])
            if i % 7 == 0:
                self.auth_event(rng.choice(["auth.session_started", "auth.login_failed", "auth.session_ended"]),
                                rng.choice(analysts))
            mark = marks.get(i)
            if mark == "late":
                self.inject_late_event(cid, lag_seconds=7200)
            elif mark == "gap":
                self.inject_sequence_gap(2)
            elif mark == "unknown":
                self.inject_unknown_event_type("case.escalated")
            elif mark == "turn_gap":
                self.inject_turn_gap(cid)
            elif mark == "teams":
                self.evolve_teams()
            self.conn.commit()
        return made
