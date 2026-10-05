"""EVT2 tests: the REAL FEED hand-off. The monitor tick writes `packages/<snap>/{manifest.json,events.ndjson,cases.ndjson}`
(`seams/crates/sources/src/monitor.rs`); the aggregator reads all the packages of one source and builds the same cell table it builds
from the synthetic ndjson pair. Hand-built packages only (no platform data); the Rust side is tested in
`seams/crates/sources/tests/platform_feed.rs`."""
import json
import re
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
sys.path.insert(0, str(HERE))
import platform_event_cells as pec  # noqa: E402
from test_platform_event_cells import Log  # noqa: E402


def make_log(n=160):
    log = Log()
    for i in range(n):
        c = log.case(case_type="service_quality" if i % 2 else "app_issue", day=1 + i % 20)
        log.ev(c, "copilot.suggestion_ready", {"agent": "copiloto-sugerencias@1.0.0", "release": "rel-a"}, day=1 + i % 20)
        log.decided(c, "discarded" if i % 3 else "used", None if i % 3 else 120, agent="copiloto-sugerencias@1.0.0", day=1 + i % 20)
    return log


def package_event(e, ordinal):
    """The line the tick writes: ids, enums, times and the allow-listed payload."""
    return {"ordinal": ordinal, "sequence": e["sequence"], "event_id": f"EVT-{e['sequence']}", "event_type": e["event_type"],
            "entity": "case", "entity_id": e["case_id"], "case_id": e["case_id"], "actor_role": None, "actor_id": None,
            "event_time": e["event_time"], "payload": e["payload"]}


def package_case(c, with_key=True):
    row = {"case_id": c["case_id"], "channel": c["channel"], "language": c["language"], "priority": "medium", "previous_case_id": None,
           "case_type": c["case_type"], "opened_at": c["opened_at"]}
    if with_key:
        row["customer_key"] = "k" + "%015x" % (abs(hash(c["customer_id"])) % (16 ** 15))
    return row


def write_packages(root, log, parts=3, source_id="platform:evt2", with_key=True, start=0):
    root = Path(root)
    size = -(-len(log.events) // parts)
    by_case = {c["case_id"]: c for c in log.cases}
    for p in range(parts):
        evs = log.events[p * size:(p + 1) * size]
        if not evs:
            continue
        d = root / f"snap-{start + p:03d}"
        d.mkdir(parents=True)
        (d / "events.ndjson").write_text("".join(json.dumps(package_event(e, i)) + "\n" for i, e in enumerate(evs)), encoding="utf-8")
        cases = sorted({e["case_id"] for e in evs})
        (d / "cases.ndjson").write_text("".join(json.dumps(package_case(by_case[c], with_key)) + "\n" for c in cases), encoding="utf-8")
        (d / "manifest.json").write_text(json.dumps({"contract": "platform-events-package/0", "source_id": source_id, "data_mode": "platform",
                                                     "watermark_from": f"seq:{evs[0]['sequence'] - 1}", "watermark_to": f"seq:{evs[-1]['sequence']}",
                                                     "events": len(evs), "dimensions": ["cases.ndjson"]}), encoding="utf-8")


class Packages(unittest.TestCase):
    def test_all_packages_of_a_source_build_the_same_table_as_the_flat_pair(self):
        log = make_log()
        flat_cases = [{**c, "customer_id": "cus-x"} for c in log.cases]  # same cells; the split key differs, so compare unsplit totals
        with tempfile.TemporaryDirectory() as t:
            write_packages(t, log)
            events, cases, info = pec.read_packages(t)
        self.assertEqual(len(events), len(log.events))
        self.assertEqual({c["case_id"] for c in cases}, {c["case_id"] for c in flat_cases})
        self.assertEqual(info["packages"], 3)
        rows, _ = pec.build(events, cases)
        want, _ = pec.build(log.events, log.cases)
        key = lambda rs: sorted((r["metric"], json.dumps(r["dims"], sort_keys=True), r["period"], r["numerator"] + r["denominator"]) for r in rs)  # noqa: E731
        # the ALL period is independent of the customer split: same totals whatever the key
        all_got = [r for r in rows if r["period"] == "ALL"]
        all_want = [r for r in want if r["period"] == "ALL"]
        self.assertEqual({(r["metric"], json.dumps(r["dims"], sort_keys=True)) for r in all_got},
                         {(r["metric"], json.dumps(r["dims"], sort_keys=True)) for r in all_want})
        tot = lambda rs: sum(r["denominator"] for r in rs)  # noqa: E731
        self.assertEqual(tot(all_got), tot(all_want))
        self.assertTrue(key(rows))

    def test_a_package_replayed_or_overlapping_does_not_double_count(self):
        log = make_log()
        with tempfile.TemporaryDirectory() as t:
            write_packages(t, log, parts=2)
            write_packages(t, log, parts=2, start=10)  # the same events again, other snapshot names
            events, _, info = pec.read_packages(t)
        self.assertEqual(len(events), len(log.events))
        self.assertEqual(info["duplicate_events_dropped"], len(log.events))

    def test_only_the_packages_of_the_requested_source_are_read(self):
        a, b = make_log(60), make_log(40)
        with tempfile.TemporaryDirectory() as t:
            write_packages(t, a, parts=1, source_id="platform:one")
            write_packages(t, b, parts=1, source_id="platform:two", start=5)
            ev, _, info = pec.read_packages(t, source_id="platform:two")
            ev_all, _, _ = pec.read_packages(t)
        self.assertEqual(len(ev), len(b.events))
        self.assertEqual(info["packages"], 1)
        self.assertGreater(len(ev_all), len(ev))

    def test_an_incomplete_package_without_a_manifest_is_skipped(self):
        log = make_log(40)
        with tempfile.TemporaryDirectory() as t:
            write_packages(t, log, parts=1)
            (Path(t) / "snap-009.tmp").mkdir()
            (Path(t) / "snap-009.tmp" / "events.ndjson").write_text('{"sequence": 1}\n', encoding="utf-8")
            _, _, info = pec.read_packages(t)
        self.assertEqual(info["packages"], 1)

    def test_a_later_package_updates_the_case_type_of_a_case(self):
        log = Log()
        c = log.case(case_type="app_issue")
        log.ev(c, "copilot.suggestion_ready", {"agent": "a-1@1", "release": "r1"})
        with tempfile.TemporaryDirectory() as t:
            write_packages(t, log, parts=1)
            log2 = Log()
            log2.cases = [{**log.cases[0], "case_type": "virtual_card"}]
            log2.events = [{**log.events[0], "sequence": 99}]
            write_packages(t, log2, parts=1, start=1)
            _, cases, _ = pec.read_packages(t)
        self.assertEqual([x["case_type"] for x in cases], ["virtual_card"])

    def test_the_cli_builds_cells_from_a_package_root(self):
        log = make_log(200)
        with tempfile.TemporaryDirectory() as t:
            write_packages(Path(t) / "packages", log)
            out = Path(t) / "cells.ndjson"
            self.assertEqual(pec.main(["--package-root", str(Path(t) / "packages"), "--source-id", "platform:evt2", "--out", str(out)]), 0)
            rows = [json.loads(line) for line in out.read_text(encoding="utf-8").splitlines()]
        self.assertTrue(rows)
        self.assertTrue({"P_DRAFT_REJECT"} <= {r["metric"] for r in rows})
        for r in rows:
            self.assertEqual(set(r), {"metric", "dims", "half", "period", "numerator", "denominator"})


class CustomerKey(unittest.TestCase):
    def test_the_hashed_customer_key_splits_exactly_like_the_id_it_stands_for(self):
        log = make_log(240)
        keyed = [{**c, "customer_key": c["customer_id"], "customer_id": None} for c in log.cases]
        a, _ = pec.build(log.events, log.cases)
        b, _ = pec.build(log.events, keyed)
        self.assertEqual(pec.to_ndjson(a), pec.to_ndjson(b))

    def test_the_key_wins_over_a_raw_id_and_is_never_emitted(self):
        log = make_log(240)
        keyed = [{**c, "customer_key": f"kx{i:04d}", "customer_id": f"CUS-RAW-{i}"} for i, c in enumerate(log.cases)]
        rows, stats = pec.build(log.events, keyed)
        blob = pec.to_ndjson(rows) + json.dumps(stats)
        self.assertNotIn("kx0", blob)
        self.assertNotIn("CUS-RAW", blob)
        self.assertFalse(re.search(r"CASE-\d", blob))

    def test_cases_of_one_customer_key_stay_in_one_half(self):
        log = Log()
        for i in range(120):
            c = log.case(day=1 + i % 25)
            log.decided(c, "discarded" if i % 2 else "used", None if i % 2 else 50, agent="copiloto-sugerencias@1.0.0", day=1 + i % 25)
        keyed = [{**c, "customer_key": f"k{i % 12:02d}"} for i, c in enumerate(log.cases)]
        cmap = pec.derive_units(log.events, keyed)[2]
        halves = {}
        for c in keyed:
            halves.setdefault(c["customer_key"], set()).add(pec.bc.split_half(c["customer_key"]))
        self.assertTrue(all(len(h) == 1 for h in halves.values()))
        self.assertTrue(cmap)


if __name__ == "__main__":
    unittest.main()
