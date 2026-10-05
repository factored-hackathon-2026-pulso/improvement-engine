"""Tests for agentcore_poller. Run: python -m unittest discover -s scripts/triggers -p "test_*.py"."""
import json
import tempfile
import unittest
from pathlib import Path

import agentcore_poller as ap

CFG = dict(tenant="tenant-local", mission="quejas", source="agent-core", config_digest="sha256:cfg1")


def run(rid, cursor, closed=False, agent="pulso-builder"):
    return {"run_seq": cursor, "cursor": cursor, "run_id": rid, "session_id": None, "release": "r1",
            "agent": {"id": agent, "version": "1.0.0"}, "principal_type": "builder", "mode": "task", "locale": "es",
            "status": "closed" if closed else "open", "outcome": "completed" if closed else None,
            "created_at": "2026-10-04T10:00:00Z", "closed_at": "2026-10-04T10:05:00Z" if closed else None}


def closed_ev(rid, seq, eid):
    return {"type": "run_closed", "event_id": eid, "run_id": rid, "seq": seq, "ts": "2026-10-04T10:05:00Z",
            "payload": {"outcome": "completed", "closed_by": "flow"}}


def reg(type_, pid="p1", origin="auto_detect", rel=None):
    return {"type": type_, "actor": "a", "principal_type": "human", "origin": origin, "proposal_id": pid,
            "candidate_hash": "h1", "release_id": rel, "at": "2026-10-04T11:00:00Z"}


class FakeCore:
    """Serves the three pull endpoints with the real {items, next_after} paging contract."""

    def __init__(self):
        self.runs, self.events, self.registry, self.calls = [], {}, [], []

    def __call__(self, path, params):
        self.calls.append((path, dict(params)))
        after, limit = params.get("after", 0), params.get("limit", 100)
        if path == "/v1/export/runs":
            items = [r for r in self.runs if r["cursor"] > after][:limit]
            return {"items": items, "next_after": items[-1]["cursor"] if items else after}
        if path.startswith("/v1/export/runs/") and path.endswith("/events"):
            rid = path.split("/")[4]
            items = [e for e in self.events.get(rid, []) if e["seq"] > after][:limit]
            return {"items": items, "next_after": items[-1]["seq"] if items else after}
        if path == "/v1/export/registry-events":
            items = self.registry[after:after + limit]
            return {"items": items, "next_after": after + len(items)}
        raise AssertionError(path)


class Sink:
    def __init__(self, fail=0):
        self.got, self.fail = [], fail

    def send(self, req):
        if self.fail:
            self.fail -= 1
            raise ap.SinkError("boom")
        self.got.append(req)


class PollerTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.state = Path(self.tmp.name) / "state.json"
        self.core, self.sink = FakeCore(), Sink()

    def tearDown(self):
        self.tmp.cleanup()

    def poller(self, sink=None, **kw):
        return ap.Poller(fetch=self.core, sink=sink or self.sink, state_path=self.state, page_limit=2, **CFG, **kw)

    def test_run_closed_becomes_request_with_stable_key_and_no_customer_data(self):
        self.core.runs = [run("r-open", 1), run("r-1", 2, closed=True)]
        self.core.events = {"r-1": [closed_ev("r-1", 3, "ev-9")]}
        n = self.poller().poll_once()
        self.assertEqual(n, 1)
        req = self.sink.got[0]
        self.assertEqual((req["schema"], req["kind"], req["event"]["type"]), ("pulso.trigger.v1", "outcome", "run.closed"))
        self.assertEqual(req["event"]["ref"], "ev-9")
        self.assertEqual(req["tenant"], "tenant-local")
        self.assertEqual(req["config_digest"], "sha256:cfg1")
        self.assertTrue(req["trigger_key"].startswith("sha256:") and len(req["trigger_key"]) == 71)
        self.assertEqual(req["event"]["subject"]["outcome"], "completed")
        self.assertNotIn("principal_type", json.dumps(req))

    def test_cursor_persists_and_replay_is_silent(self):
        self.core.runs = [run("r-1", 1, closed=True)]
        self.core.events = {"r-1": [closed_ev("r-1", 1, "ev-1")]}
        self.poller().poll_once()
        self.assertEqual(self.poller().poll_once(), 0)  # new instance, same state file
        self.assertEqual(len(self.sink.got), 1)
        self.assertEqual(json.loads(self.state.read_text())["runs_cursor"], 1)

    def test_open_run_that_later_closes_fires_once(self):
        self.core.runs = [run("r-1", 1)]
        p = self.poller()
        self.assertEqual(p.poll_once(), 0)
        self.core.runs = [run("r-1", 2, closed=True)]
        self.core.events = {"r-1": [closed_ev("r-1", 4, "ev-4")]}
        self.assertEqual(p.poll_once(), 1)
        self.assertEqual(p.poll_once(), 0)

    def test_release_events_map_and_other_registry_events_are_ignored(self):
        self.core.registry = [reg("proposal_created"), reg("approved"), reg("published", rel="rel-1"),
                              reg("promoted", rel="rel-1"), reg("revoked", rel="rel-1"), reg("draft_updated")]
        self.assertEqual(self.poller().poll_once(), 3)  # spans 3 pages of 2
        self.assertEqual([r["event"]["type"] for r in self.sink.got], ["release.published", "release.promoted", "release.revoked"])
        self.assertEqual(self.sink.got[0]["event"]["subject"]["release_id"], "rel-1")
        self.assertEqual(json.loads(self.state.read_text())["registry_after"], 6)

    def test_origin_filter_keeps_only_our_proposals(self):
        self.core.registry = [reg("published", pid="a", origin="manual", rel="r1"), reg("published", pid="b", rel="r2")]
        self.assertEqual(self.poller(only_origin="auto_detect").poll_once(), 1)
        self.assertEqual(self.sink.got[0]["event"]["subject"]["proposal_id"], "b")

    def test_overlap_rereads_without_duplicates(self):
        self.core.registry = [reg("published", rel="r1"), reg("promoted", rel="r1")]
        p = self.poller(overlap=5)
        p.poll_once()
        n = len(self.core.calls)
        self.assertEqual(p.poll_once(), 0)
        self.assertEqual(len(self.sink.got), 2)
        first = [c for c in self.core.calls[n:] if c[0].endswith("registry-events")][0]
        self.assertEqual(first[1]["after"], 0)  # re-read from cursor minus overlap, floored at 0

    def test_sink_failure_keeps_cursor_then_retry_delivers_once(self):
        self.core.registry = [reg("published", rel="r1")]
        bad = Sink(fail=1)
        with self.assertRaises(ap.SinkError):
            self.poller(sink=bad).poll_once()
        self.assertFalse(self.state.exists() and json.loads(self.state.read_text()).get("registry_after"))
        self.assertEqual(self.poller(sink=bad).poll_once(), 1)
        self.assertEqual(len(bad.got), 1)

    def test_explicit_and_scheduled_share_the_request_path(self):
        p = self.poller()
        self.assertTrue(p.explicit("run-now-1", reason="operator"))
        self.assertFalse(p.explicit("run-now-1", reason="operator"))  # same idempotency key
        self.assertTrue(p.scheduled(now=1000, interval_secs=300))
        self.assertFalse(p.scheduled(now=1100, interval_secs=300))  # same slot 3
        self.assertTrue(p.scheduled(now=1300, interval_secs=300))
        self.assertEqual([r["kind"] for r in self.sink.got], ["explicit", "scheduled", "scheduled"])
        self.assertEqual({r["schema"] for r in self.sink.got}, {"pulso.trigger.v1"})

    def test_key_depends_on_scope(self):
        base = dict(tenant="t", mission="m", source="s", config_digest="c", kind="outcome", ref="x")
        a = ap.trigger_key(**base)
        self.assertEqual(a, ap.trigger_key(**base))
        self.assertNotEqual(a, ap.trigger_key(**{**base, "tenant": "t2"}))

    def test_only_loopback_urls_allowed(self):
        ap.require_loopback("http://127.0.0.1:8001")
        ap.require_loopback("http://localhost:8001/x")
        for bad in ("http://example.com", "https://10.0.0.5:1", "http://127.0.0.1.evil.com"):
            with self.assertRaises(ValueError):
                ap.require_loopback(bad)

    def test_token_never_in_state_or_error_text(self):
        self.core.registry = [reg("published", rel="r1")]
        self.poller().poll_once()
        self.assertNotIn("Bearer", self.state.read_text())
        err = ap.redact("HTTP 401 Authorization: Bearer abc.def.ghi", ["abc.def.ghi"])
        self.assertNotIn("abc.def", err)


class RecordedFixtureTest(unittest.TestCase):
    """RECORDED (not live): real response shapes captured from the local agent-core stack."""

    def test_recorded_pages_yield_run_closed_and_no_release_events(self):
        fx = json.loads((Path(__file__).parent / "fixtures" / "export_recorded.json").read_text(encoding="utf-8"))

        def fetch(path, params):
            if path == "/v1/export/runs":
                return fx["runs"] if params["after"] == 0 else {"items": [], "next_after": params["after"]}
            if path == "/v1/export/registry-events":
                return fx["registry"] if params["after"] == 0 else {"items": [], "next_after": params["after"]}
            return fx["events"][path.split("/")[4]]

        sink = Sink()
        with tempfile.TemporaryDirectory() as d:
            p = ap.Poller(fetch=fetch, sink=sink, state_path=Path(d) / "s.json", **CFG)
            self.assertEqual(p.poll_once(), len(fx["runs"]["items"]))
        self.assertEqual({r["event"]["type"] for r in sink.got}, {"run.closed"})  # no release.* in the recording


if __name__ == "__main__":
    unittest.main()
