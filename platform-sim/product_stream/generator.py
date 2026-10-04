"""Deterministic discrete-event generator of platform-shaped product rows.

One RNG, one event per heap step: output depends only on (seed, scenario, parameters), never on batch size.
Rows carry opaque synthetic ids, enums, counters and timestamps only: no names, contact data or free text."""
from __future__ import annotations

import hashlib
import heapq
import random
from datetime import datetime, timedelta, timezone

from .scenarios import CELLS, Scenario

SLA_SECONDS = {"high": 300, "medium": 900, "low": 3600}
CELL_W = (0.4, 0.3, 0.15, 0.15)
CAP = 4  # open cases per analyst before queueing
TENANT = "ten-product-sim"


def iso(dt: datetime) -> str:
    return dt.astimezone(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def make_scenario(name: str, horizon_events: int, **kw) -> Scenario:
    return Scenario(name=name)


class ProductStream:
    def __init__(self, seed: int = 0, scenario: str | Scenario = "null", horizon_events: int = 20000,
                 start: str = "2026-09-01T08:00:00Z", n_customers: int = 400, mean_gap_s: float = 20.0,
                 tenant: str = TENANT, **scenario_kw):
        self.seed = seed
        self.rng = random.Random(seed)
        self.horizon = horizon_events
        self.tenant = tenant
        self.mean_gap = mean_gap_s
        self.scn = scenario if isinstance(scenario, Scenario) else make_scenario(scenario, horizon_events,
                                                                                 **scenario_kw)
        self.now = datetime.fromisoformat(start.replace("Z", "+00:00"))
        self._seq = 0
        self._ctr: dict[str, int] = {}
        self._heap: list = []
        self._tie = 0
        self._buf: list[tuple[dict, dict]] = []
        self._cases: dict[str, dict] = {}
        self._open_by_cust: dict[str, str] = {}
        self._staff: list[dict] = []
        self._customers: list[str] = []
        self._pending_static: dict[str, list[dict]] = {}
        # realised stats: (sequence of the event that created it, cell)
        self.stats: dict[str, list] = {"cases": [], "reassigns": [], "reopens": []}
        self._seed_static(n_customers)
        self._schedule(0.0, "arrive", None)

    @property
    def last_sequence(self) -> int:
        return self._seq

    def _id(self, prefix: str) -> str:
        n = self._ctr[prefix] = self._ctr.get(prefix, 0) + 1
        return f"{prefix}-{hashlib.sha1(f'{self.seed}:{prefix}:{n}'.encode()).hexdigest()[:8]}"

    def _schedule(self, delay: float, kind: str, arg) -> None:
        self._tie += 1
        heapq.heappush(self._heap, (self.now + timedelta(seconds=delay), self._tie, kind, arg))

    def _next_seq(self) -> int:
        """Sequence the next appended event will receive."""
        return self._seq + len(self._buf) + 1

    def _seed_static(self, n_customers: int) -> None:
        rows: dict[str, list[dict]] = {"staff": [], "customers": [], "customer_case_slots": []}
        spec = [("supervisor", ["es", "pt"], "supervision"), ("analyst", ["es"], "general"),
                ("analyst", ["es"], "general"), ("analyst", ["es"], "general"),
                ("analyst", ["es", "pt"], "portuguese"), ("analyst", ["es", "pt"], "portuguese")]
        for role, langs, team in spec:
            sid = self._id("STF")
            s = {"id": sid, "roles": [role], "languages": langs, "team": team, "team_id": None, "active": True}
            self._staff.append({"id": sid, "role": role, "languages": langs, "load": 0})
            rows["staff"].append(s)
        for _ in range(n_customers):
            cid = self._id("CUS")
            self._customers.append(cid)
            rows["customers"].append({"id": cid, "simulator": True})
            rows["customer_case_slots"].append({"customer_id": cid, "open_case_id": None})
        self._pending_static = rows

    def _emit(self, etype: str, entity: str, entity_id, case_id, role: str, actor, payload: dict,
              side: dict | None = None) -> None:
        lag = self.rng.randint(0, 3)
        ev = {"event_id": self._id("EVT"), "event_type": etype, "entity": entity, "entity_id": entity_id,
              "case_id": case_id, "actor_role": role, "actor_id": actor, "event_time": iso(self.now),
              "ingested_at": iso(self.now + timedelta(seconds=lag)), "payload": payload, "tenant_id": self.tenant}
        self._buf.append((ev, side or {}))

    def _emit_turn(self, c: dict, role: str, actor) -> None:
        c["turns"] += 1
        row = {"id": self._id("TUR"), "case_id": c["id"], "sequence": c["turns"], "kind": "message",
               "audience": "everyone", "author_role": role, "created_at": iso(self.now)}
        self._emit("turn.created", "turn", row["id"], c["id"], role, actor,
                   {"sequence": c["turns"], "kind": "message", "audience": "everyone"}, {"turns": [row]})

    def _step(self) -> None:
        self.now, _, kind, arg = heapq.heappop(self._heap)
        getattr(self, f"_do_{kind}")(arg)

    def _do_arrive(self, _):
        mult = self.scn.volume_mult(self._seq)
        self._schedule(self.rng.expovariate(mult / self.mean_gap), "arrive", None)
        free = [c for c in self._customers if c not in self._open_by_cust]
        if free:
            self._open_case(self.rng.choice(free), None)

    def _open_case(self, cust: str, prev: str | None) -> None:
        cell = self.rng.choices(CELLS, CELL_W)[0] if prev is None else self._cases[prev]["cell"]
        prio = self.rng.choices(["high", "medium", "low"], [0.15, 0.55, 0.30])[0]
        cid = self._id("CAS")
        self._cases[cid] = {"id": cid, "cust": cust, "cell": cell, "turns": 0, "staff": None,
                            "status": "queued", "responded": False, "queued_at": None, "rounds": 0}
        self._open_by_cust[cust] = cid
        row = {"id": cid, "customer_id": cust, "channel": cell[1], "language": cell[0], "priority": prio,
               "opened_at": iso(self.now), "sla_due_at": iso(self.now + timedelta(seconds=SLA_SECONDS[prio])),
               "previous_case_id": prev, "tenant_id": self.tenant}
        self.stats["cases"].append((self._next_seq(), cell))
        if prev:
            self.stats["reopens"].append((self._next_seq(), cell))
        self._emit("case.opened", "case", cid, cid, "customer", cust, {"priority": prio},
                   {"cases": [row], "customer_case_slots": [{"customer_id": cust, "open_case_id": cid}]})
        self._schedule(self.rng.uniform(1, 5), "cmsg", cid)

    def _do_cmsg(self, cid):
        c = self._cases[cid]
        self._emit_turn(c, "customer", c["cust"])
        self._schedule(self.rng.uniform(1, 4), "route" if c["staff"] is None else "reply", cid)

    def _eligible(self, lang: str) -> list[dict]:
        return sorted((s for s in self._staff if s["role"] == "analyst" and lang in s["languages"]
                       and s["load"] < CAP), key=lambda s: (s["load"], s["id"]))

    def _do_route(self, cid):
        c = self._cases[cid]
        el = self._eligible(c["cell"][0])
        if not el:
            if c["queued_at"] is None:
                c["queued_at"] = self.now
                self._emit("case.queued", "case", cid, cid, "system", None, {"queue_label": c["cell"][0]})
            self._schedule(self.rng.uniform(20, 60), "route", cid)
            return
        waited = int((self.now - c["queued_at"]).total_seconds()) if c["queued_at"] else 0
        reason = "queue_drained" if c["queued_at"] else "language_least_loaded"
        self._assign(c, el[0], reason, waited, None, "system", None)
        self._schedule(self.rng.uniform(5, 40), "reply", cid)
        if self.rng.random() < self.scn.reassign_prob(c["cell"], self._seq):
            self._schedule(self.rng.uniform(60, 240), "reassign", cid)

    def _assign(self, c, s, reason, waited, prev, role, actor):
        for x in self._staff:
            if x["id"] == c["staff"]:
                x["load"] -= 1
        s["load"] += 1
        c["staff"] = s["id"]
        c["status"] = "assigned"
        row = {"id": self._id("ASG"), "case_id": c["id"], "staff_id": s["id"], "reason": reason,
               "policy_rule_id": "H1" if c["cell"][0] == "pt" else None, "strategy": "language_least_loaded",
               "open_cases_at_assignment": s["load"] - 1, "waited_seconds": waited, "previous_staff_id": prev,
               "paused_override": False, "assigned_at": iso(self.now)}
        pl = {"staff_id": s["id"], "reason": reason}
        if prev:
            pl["previous_staff_id"] = prev
        self._emit("case.assigned", "case", c["id"], c["id"], role, actor, pl, {"assignments": [row]})

    def _do_reassign(self, cid):
        c = self._cases[cid]
        if c["status"] == "closed":
            return
        others = [s for s in self._eligible(c["cell"][0]) if s["id"] != c["staff"]]
        if not others:
            return
        sup = next(s for s in self._staff if s["role"] == "supervisor")
        self.stats["reassigns"].append((self._next_seq(), c["cell"]))
        self._assign(c, others[0], "manual", 0, c["staff"], "supervisor", sup["id"])

    def _do_reply(self, cid):
        c = self._cases[cid]
        if c["status"] == "closed":
            return
        self._emit_turn(c, "analyst", c["staff"])
        if not c["responded"]:
            c["responded"] = True
            self._emit("case.first_responded", "case", cid, cid, "analyst", c["staff"], {})
            self._emit("case.status_changed", "case", cid, cid, "analyst", c["staff"],
                       {"from": "assigned", "to": "in_progress"})
        if self.rng.random() < 0.5:
            self._emit("case.read", "case", cid, cid, "analyst", c["staff"], {})
        c["status"] = "in_progress"
        c["rounds"] += 1
        if c["rounds"] < self.rng.choice([1, 2, 3]):
            self._schedule(self.rng.uniform(20, 120), "cmsg", cid)
        else:
            self._schedule(self.rng.uniform(30, 200), "close", cid)

    def _do_close(self, cid):
        c = self._cases[cid]
        reason = self.rng.choices(["resolved", "customer_unresponsive", "duplicate", "other"], [65, 15, 8, 12])[0]
        for x in self._staff:
            if x["id"] == c["staff"]:
                x["load"] -= 1
        c["status"] = "closed"
        self._open_by_cust.pop(c["cust"], None)
        self._emit("case.status_changed", "case", cid, cid, "analyst", c["staff"],
                   {"from": "in_progress", "to": "closed"})
        self._emit("case.closed", "case", cid, cid, "analyst", c["staff"], {"close_reason": reason},
                   {"customer_case_slots": [{"customer_id": c["cust"], "open_case_id": None}]})
        if self.rng.random() < self.scn.reopen_prob(c["cell"], self._seq):
            self._schedule(self.rng.uniform(120, 900), "reopen", cid)

    def _do_reopen(self, cid):
        c = self._cases[cid]
        if c["cust"] not in self._open_by_cust:
            self._open_case(c["cust"], cid)

    def next_batch(self, n: int) -> dict[str, list[dict]]:
        """Up to n event_log rows plus the side rows (cases, turns, assignments, slots) they carry.
        The first batch also carries the static staff/customers/slots rows."""
        out: dict[str, list[dict]] = {k: list(v) for k, v in self._pending_static.items()}
        self._pending_static = {}
        out["event_log"] = []
        while len(out["event_log"]) < n:
            while not self._buf:
                self._step()
            ev, side = self._buf.pop(0)
            self._seq += 1
            out["event_log"].append({"sequence": self._seq, **ev})
            for t, rows in side.items():
                out.setdefault(t, []).extend(rows)
        return out
