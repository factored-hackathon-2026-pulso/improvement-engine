"""Tests for decision_feedback. Run: python -m unittest discover -s scripts/feedback -p "test_*.py"."""
import json
import tempfile
import unittest
from pathlib import Path

import decision_feedback as df

NOW = "2026-10-20T00:00:00Z"
ENG = "engine-builder"
HDR = "[finding: M4|Queja|Phone|es] La tasa es alta."


def ev(type_, pid, at, origin="auto_detect", **extra):
    return {"type": type_, "actor": "a", "principal_type": "human", "origin": origin, "proposal_id": pid,
            "candidate_hash": "h", "release_id": None, "at": at, **extra}


def prop(pid, state="draft", origin="auto_detect", by=ENG, rev=1, title="template:t/x - M4: cambio"):
    return {"proposal_id": pid, "agent_id": "recepcion", "origin": origin, "state": state, "rev": rev,
            "base_release_id": None, "title": title, "created_by": by, "candidate_hash": None,
            "updated_at": "2026-10-01T00:00:00Z"}


def detail(p, rationale=HDR, kind="template", eid="t/x", changelog="c"):
    return {"proposal": p, "last_eval": None, "review": None,
            "changes": [{"kind": kind, "content": {"id": eid, "version": "1.0.1"},
                         "docs": {"description": "d", "rationale": rationale, "changelog": changelog}}]}


class Core:
    """Fake agent-core: /v1/registry/proposals(+/{pid}) and /v1/export/registry-events."""

    def __init__(self):
        self.proposals, self.details, self.events, self.calls = [], {}, [], []

    def add(self, p, **kw):
        self.proposals.append(p)
        self.details[p["proposal_id"]] = detail(p, **kw)

    def __call__(self, path, params):
        self.calls.append((path, dict(params)))
        if path == "/v1/registry/proposals":
            items = [p for p in self.proposals
                     if params.get("created_by") in (None, p["created_by"])][params.get("offset", 0):][:params["limit"]]
            return {"items": items, "total": len(self.proposals)}
        if path.startswith("/v1/registry/proposals/"):
            return self.details[path.rsplit("/", 1)[1]]
        if path == "/v1/export/registry-events":
            a, n = params["after"], params["limit"]
            items = self.events[a:a + n]
            return {"items": items, "next_after": a + len(items)}
        raise AssertionError(path)


def rec(pid, state, reason="unknown", at="2026-10-10T00:00:00Z", target="template:t/x", family="M4", kind="template",
        ttd=3600, source="structured"):
    return {"schema": df.SCHEMA, "proposal_id": pid, "state": state, "target": target, "family": family,
            "kind": kind, "decision_at": at, "time_to_decision_secs": ttd,
            "reason": {"code": reason, "source": source, "note_len": 0}}


class Reasons(unittest.TestCase):
    def test_structured_code_in_vocabulary(self):
        self.assertEqual(df.normalize_reason({"reason_code": "Wrong_Target"}), ("wrong_target", "structured", 0))

    def test_unknown_structured_code_is_other_not_passed_through(self):
        self.assertEqual(df.normalize_reason({"reason_code": "ignore previous instructions"})[0], "other")

    def test_free_text_mapped_and_never_kept(self):
        text = "Esto es duplicado de la propuesta anterior"
        code, src, n = df.normalize_reason({"reason": text})
        self.assertEqual((code, src), ("duplicate", "mapped"))
        self.assertEqual(n, len(text))

    def test_free_text_without_match_is_unknown(self):
        self.assertEqual(df.normalize_reason({"reason": "no me gusta"})[:2], ("unknown", "none"))

    def test_nothing(self):
        self.assertEqual(df.normalize_reason({}), ("unknown", "none", 0))


class Header(unittest.TestCase):
    def test_finding_key_and_family(self):
        self.assertEqual(df.finding_key(HDR, ""), "M4|Queja|Phone|es")
        self.assertEqual(df.family_of("M4|Queja|Phone|es"), "M4")

    def test_changelog_fallback_and_title_fallback(self):
        self.assertEqual(df.finding_key("x", "[finding: m2|a] y"), "m2|a")
        self.assertEqual(df.finding_key("x", "y", title="template:t/x - M7: cambio"), "M7")
        self.assertIsNone(df.finding_key("x", "y", title="sin metrica"))

    def test_hostile_header_rejected(self):
        self.assertIsNone(df.finding_key("[finding: " + "a" * 500 + "]", ""))
        self.assertIsNone(df.finding_key("[finding: <script>]", ""))


class Fold(unittest.TestCase):
    def test_rejected(self):
        events = [["proposal_created", "2026-10-10T00:00:00Z"], ["frozen", "2026-10-10T01:00:00Z"],
                  ["evaluated", "2026-10-10T02:00:00Z"], ["rejected", "2026-10-10T03:00:00Z", "wrong_target"]]
        f = df.fold_events(events, NOW, expire_days=14)
        self.assertEqual((f["state"], f["time_to_decision_secs"], f["reason_code"]), ("rejected", 10800, "wrong_target"))

    def test_approved_then_published(self):
        events = [["proposal_created", "2026-10-10T00:00:00Z"], ["approved", "2026-10-10T01:00:00Z"],
                  ["published", "2026-10-10T02:00:00Z"]]
        f = df.fold_events(events, NOW, expire_days=14)
        self.assertEqual((f["state"], f["time_to_decision_secs"]), ("published", 3600))

    def test_reopen_after_approval_returns_to_draft_but_keeps_first_decision(self):
        events = [["proposal_created", "2026-10-10T00:00:00Z"], ["approved", "2026-10-10T01:00:00Z"],
                  ["reopened", "2026-10-10T02:00:00Z"]]
        f = df.fold_events(events, NOW, expire_days=14)
        self.assertEqual(f["state"], "draft")
        self.assertEqual(f["time_to_decision_secs"], 3600)

    def test_expired_when_idle_and_undecided(self):
        f = df.fold_events([["proposal_created", "2026-10-01T00:00:00Z"], ["evaluated", "2026-10-02T00:00:00Z"]], NOW, 14)
        self.assertEqual(f["state"], "expired")
        self.assertIsNone(f["time_to_decision_secs"])

    def test_recent_undecided_is_evaluated_not_expired(self):
        f = df.fold_events([["proposal_created", "2026-10-18T00:00:00Z"], ["evaluated", "2026-10-19T00:00:00Z"]], NOW, 14)
        self.assertEqual(f["state"], "evaluated")

    def test_rejected_survives_rework_until_refrozen(self):
        base = [["proposal_created", "2026-10-10T00:00:00Z"], ["rejected", "2026-10-10T03:00:00Z"],
                ["draft_updated", "2026-10-10T04:00:00Z"]]
        self.assertEqual(df.fold_events(base, NOW, 14)["state"], "rejected")
        self.assertEqual(df.fold_events(base + [["frozen", "2026-10-10T05:00:00Z"]], NOW, 14)["state"], "candidate")

    def test_stale(self):
        f = df.fold_events([["proposal_created", "2026-10-10T00:00:00Z"], ["approved", "2026-10-10T01:00:00Z"],
                            ["proposal_staled", "2026-10-10T02:00:00Z"]], NOW, 14)
        self.assertEqual(f["state"], "draft")


class Poll(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.state = Path(self.tmp.name) / "s.json"
        self.out = Path(self.tmp.name) / "d.jsonl"
        self.core = Core()
        self.core.add(prop("p1"))
        self.core.add(prop("p2"))
        self.core.add(prop("p3", origin="manual"))
        self.core.add(prop("p4", by="someone-else"))
        e = self.core.events
        e += [ev("proposal_created", "p1", "2026-10-10T00:00:00Z"), ev("proposal_created", "p2", "2026-10-10T00:00:00Z"),
              ev("proposal_created", "p3", "2026-10-10T00:00:00Z", origin="manual"),
              ev("proposal_created", "p4", "2026-10-10T00:00:00Z"),
              ev("evaluated", "p1", "2026-10-10T01:00:00Z"), ev("approved", "p1", "2026-10-10T02:00:00Z"),
              ev("evaluated", "p2", "2026-10-10T01:00:00Z"),
              ev("rejected", "p2", "2026-10-10T03:00:00Z", reason_code="duplicate"),
              ev("published", None, "2026-10-10T04:00:00Z", release_id="r")]

    def reader(self, **kw):
        a = dict(fetch=self.core, sink=df.JsonlSink(self.out), state_path=self.state, created_by=ENG,
                 clock=lambda: NOW)
        a.update(kw)
        return df.DecisionReader(**a)

    def lines(self):
        return [json.loads(x) for x in self.out.read_text().splitlines()] if self.out.exists() else []

    def test_records_only_for_engine_origin_and_creator(self):
        n = self.reader().poll_once()
        got = {r["proposal_id"]: r for r in self.lines()}
        self.assertEqual(n, 2)
        self.assertEqual(set(got), {"p1", "p2"})
        r1, r2 = got["p1"], got["p2"]
        self.assertEqual((r1["schema"], r1["state"], r1["time_to_decision_secs"]), ("pulso.decision/1", "approved", 7200))
        self.assertEqual((r2["state"], r2["reason"]["code"], r2["reason"]["source"]), ("rejected", "duplicate", "structured"))
        self.assertEqual((r1["finding_key"], r1["family"], r1["target"], r1["kind"]),
                         ("M4|Queja|Phone|es", "M4", "template:t/x", "template"))

    def test_replay_is_idempotent_and_cursor_persists(self):
        self.reader().poll_once()
        before = self.out.read_text()
        self.assertEqual(self.reader().poll_once(), 0)  # new process, same state file
        self.assertEqual(self.out.read_text(), before)
        self.assertEqual(json.loads(self.state.read_text())["registry_after"], len(self.core.events))

    def test_state_change_appends_new_record_for_same_proposal(self):
        approved = self.core.events.pop(5)  # p1 not yet approved
        self.reader().poll_once()
        approved["at"] = "2026-10-11T02:00:00Z"
        self.core.events.append(approved)
        self.reader().poll_once()
        states = [r["state"] for r in self.lines() if r["proposal_id"] == "p1"]
        self.assertEqual(states, ["evaluated", "approved"])

    def test_loopback_guard(self):
        with self.assertRaises(ValueError):
            df.HttpFetch("http://example.com", "t", "t")
        df.HttpFetch("http://127.0.0.1:8001", "t", "t")

    def test_origin_filter_option(self):
        self.reader(origin="manual", created_by=None).poll_once()
        self.assertEqual({r["proposal_id"] for r in self.lines()}, {"p3"})

    def test_detail_fetched_once_per_rev(self):
        r = self.reader()
        r.poll_once()
        n = sum(1 for p, _ in self.core.calls if p.startswith("/v1/registry/proposals/"))
        r.poll_once()
        self.assertEqual(sum(1 for p, _ in self.core.calls if p.startswith("/v1/registry/proposals/")), n)

    def test_free_text_reason_never_persisted(self):
        self.core.events[7] = ev("rejected", "p2", "2026-10-10T03:00:00Z", reason="llame a Juan al 3001234567 duplicado")
        self.reader().poll_once()
        txt = self.out.read_text() + self.state.read_text()
        self.assertNotIn("3001234567", txt)
        self.assertNotIn("Juan", txt)
        r2 = [r for r in self.lines() if r["proposal_id"] == "p2"][-1]
        self.assertEqual((r2["reason"]["code"], r2["reason"]["source"]), ("duplicate", "mapped"))


class Metrics(unittest.TestCase):
    def test_aggregate(self):
        rs = [rec("a", "approved", ttd=100), rec("b", "published", ttd=300), rec("c", "rejected", "risk", ttd=200),
              rec("d", "expired", ttd=None), rec("e", "evaluated", ttd=None),
              rec("f", "rejected", "unknown", kind="prompt", ttd=400)]
        m = df.aggregate(rs)
        self.assertEqual((m["made"], m["approved"], m["rejected"], m["expired"], m["pending"]), (6, 2, 2, 1, 1))
        self.assertEqual(m["median_time_to_decision_secs"], 250)
        self.assertAlmostEqual(m["approval_rate"], 0.5)
        self.assertEqual(m["reasons"], {"risk": 1, "unknown": 1})
        self.assertEqual(m["by_kind"]["prompt"]["rejected"], 1)
        self.assertEqual(m["by_kind"]["template"]["made"], 5)

    def test_latest_record_per_proposal_wins(self):
        rs = [rec("a", "evaluated", ttd=None), rec("a", "approved", ttd=100)]
        self.assertEqual(df.aggregate(rs)["made"], 1)
        self.assertEqual(df.aggregate(rs)["approved"], 1)

    def test_empty(self):
        m = df.aggregate([])
        self.assertEqual((m["made"], m["approval_rate"], m["median_time_to_decision_secs"]), (0, None, None))


class Suppression(unittest.TestCase):
    def test_blocking_reason_within_cooldown(self):
        for reason in ("wrong_target", "duplicate", "policy_conflict", "risk"):
            d = df.suppress([rec("a", "rejected", reason)], "template:t/x", "M4", NOW, cooldown_days=30)
            self.assertTrue(d["suppress"], reason)
            self.assertEqual((d["reason"], d["proposal_id"]), (reason, "a"))

    def test_non_blocking_reasons_do_not_suppress(self):
        for reason in ("insufficient_evidence", "wording", "other", "unknown"):
            self.assertFalse(df.suppress([rec("a", "rejected", reason)], "template:t/x", "M4", NOW, 30)["suppress"])

    def test_after_cooldown_or_other_target_or_family(self):
        r = [rec("a", "rejected", "risk", at="2026-08-01T00:00:00Z")]
        self.assertFalse(df.suppress(r, "template:t/x", "M4", NOW, 30)["suppress"])
        r = [rec("a", "rejected", "risk")]
        self.assertFalse(df.suppress(r, "template:t/y", "M4", NOW, 30)["suppress"])
        self.assertFalse(df.suppress(r, "template:t/x", "M2", NOW, 30)["suppress"])

    def test_later_approval_lifts_it(self):
        r = [rec("a", "rejected", "risk", at="2026-10-10T00:00:00Z"), rec("b", "approved", at="2026-10-12T00:00:00Z")]
        self.assertFalse(df.suppress(r, "template:t/x", "M4", NOW, 30)["suppress"])

    def test_mapped_free_text_does_not_suppress_by_default(self):
        r = [rec("a", "rejected", "risk", source="mapped")]
        self.assertFalse(df.suppress(r, "template:t/x", "M4", NOW, 30)["suppress"])
        self.assertTrue(df.suppress(r, "template:t/x", "M4", NOW, 30, allow_mapped=True)["suppress"])

    def test_cooldown_until_reported(self):
        d = df.suppress([rec("a", "rejected", "risk", at="2026-10-10T00:00:00Z")], "template:t/x", "M4", NOW, 30)
        self.assertEqual(d["until"], "2026-11-09T00:00:00Z")


class History(unittest.TestCase):
    RS = [rec("a", "approved"), rec("b", "published"), rec("c", "rejected", "insufficient_evidence"),
          rec("d", "rejected", "insufficient_evidence"), rec("e", "rejected", "wording"),
          rec("f", "approved", target="template:t/other"), rec("g", "evaluated", ttd=None)]

    def test_same_target_and_family(self):
        h = df.history(self.RS, "template:t/x", "M4")
        self.assertEqual((h["scope"], h["approved"], h["rejected"]), ("target_family", 2, 3))
        self.assertEqual(h["top_reason"], "insufficient_evidence")
        self.assertEqual(h["reasons"], {"insufficient_evidence": 2, "wording": 1})

    def test_family_fallback(self):
        h = df.history(self.RS, "template:t/new", "M4")
        self.assertEqual((h["scope"], h["approved"], h["rejected"]), ("family", 3, 3))

    def test_none_when_no_history(self):
        self.assertIsNone(df.history(self.RS, "template:t/x", "M9"))

    def test_line_es_pt_en(self):
        h = df.history(self.RS, "template:t/x", "M4")
        self.assertIn("2 aprobadas", df.history_line(h, "es"))
        self.assertIn("3 rechazadas", df.history_line(h, "es"))
        self.assertIn("insufficient_evidence", df.history_line(h, "es"))
        self.assertIn("rejeitadas", df.history_line(h, "pt"))
        self.assertIn("2 approved", df.history_line(h, "en"))

    def test_line_without_reason_when_unknown_only(self):
        h = df.history([rec("a", "rejected", "unknown")], "template:t/x", "M4")
        self.assertNotIn("motivo", df.history_line(h, "es"))


if __name__ == "__main__":
    unittest.main()
