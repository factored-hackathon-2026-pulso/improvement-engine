"""TRN0: train integrator (dependency order, restack check, W0 receipt per train PR, PR size cap, exchange bundle)."""
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

GOV = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(GOV))
import trn0_train as trn  # noqa: E402


def git(repo, *a):
    return subprocess.run(["git", "-C", str(repo), "-c", "user.name=t", "-c", "user.email=t@example.invalid", *a],
                          capture_output=True, text=True, check=True).stdout.strip()


def commit(repo, name, text="x"):
    Path(repo, name).write_text(text, encoding="utf-8")
    git(repo, "add", name)
    git(repo, "commit", "-qm", f"add {name}")


def lanes(*specs):
    return [{"id": i, "branch": b, "deps": d, "lane_hours": h} for i, b, d, h in specs]


class Order(unittest.TestCase):
    def test_dependency_order_ok(self):
        ls = lanes(("a", "ba", [], 5), ("b", "bb", ["a"], 5))
        self.assertEqual(trn.check_order(ls), [])

    def test_first_red_lane_before_its_dependency_fails(self):
        ls = lanes(("b", "bb", ["a"], 5), ("a", "ba", [], 5))
        self.assertTrue(any("out of order" in p for p in trn.check_order(ls)))

    def test_unknown_dependency_and_duplicates_fail(self):
        self.assertTrue(trn.check_order(lanes(("a", "ba", ["zz"], 5))))
        self.assertTrue(trn.check_order(lanes(("a", "ba", [], 5), ("a", "bb", [], 5))))

    def test_dependency_cycle_fails(self):
        self.assertTrue(trn.check_order(lanes(("a", "ba", ["b"], 5), ("b", "bb", ["a"], 5))))


class Safety(unittest.TestCase):
    def test_receipt_leg_pass_needs_command_and_exit_zero(self):
        with tempfile.TemporaryDirectory() as t:
            leg = {"status": "pass", "exit_code": 0, "command": "c"}
            doc = {"schema": "pre-pr-gate/v1", "verdict": "pass", "legs": {k: dict(leg) for k in trn.LEGS}}
            f = Path(t, "r.json")
            f.write_text(json.dumps(doc))
            self.assertEqual(trn.check_receipt(f), [])
            del doc["legs"]["ci"]["command"]
            f.write_text(json.dumps(doc))
            self.assertTrue(trn.check_receipt(f))

    def test_negative_missing_or_nonnumeric_lane_hours_are_rejected(self):
        for h in (-5, 0, "10", None, True):
            self.assertTrue(trn.check_plan([{"id": "a", "branch": "ba", "deps": [], "lane_hours": h}], 35), h)
        self.assertTrue(trn.check_plan([{"id": "a", "branch": "ba", "deps": []}], 35))

    def test_option_like_branch_names_are_rejected(self):
        self.assertTrue(trn.check_order([{"id": "a", "branch": "--upload-pack=x", "deps": [], "lane_hours": 1}]))

    def test_merge_never_resets_a_lane_or_base_branch(self):
        with tempfile.TemporaryDirectory() as t:
            git(t, "init", "-q", "-b", "main")
            commit(t, "base.txt")
            git(t, "checkout", "-q", "-b", "lane-a")
            commit(t, "a.txt")
            git(t, "checkout", "-q", "main")
            before = git(t, "rev-parse", "lane-a")
            ls = lanes(("a", "lane-a", [], 5))
            for bad in ("lane-a", "main"):
                with self.assertRaises(ValueError):
                    trn.merge(t, "main", bad, ls)
            self.assertEqual(git(t, "rev-parse", "lane-a"), before)

    def test_merge_refuses_a_remote_tracking_or_ref_style_train_branch(self):
        with tempfile.TemporaryDirectory() as t:
            git(t, "init", "-q", "-b", "main")
            commit(t, "base.txt")
            for bad in ("origin/x", "refs/heads/x", "-x"):
                with self.assertRaises(ValueError):
                    trn.merge(t, "main", bad, [])


class Plan(unittest.TestCase):
    def test_groups_respect_the_cap_in_order(self):
        ls = lanes(("a", "ba", [], 20), ("b", "bb", [], 10), ("c", "bc", [], 20), ("d", "bd", [], 5))
        self.assertEqual([[l["id"] for l in g] for g in trn.plan(ls, 35)], [["a", "b"], ["c", "d"]])

    def test_oversize_lane_is_flagged(self):
        ls = lanes(("a", "ba", [], 40))
        self.assertTrue(any("exceeds" in p for p in trn.check_plan(ls, 35)))
        self.assertEqual(trn.check_plan(lanes(("a", "ba", [], 30)), 35), [])


class Receipts(unittest.TestCase):
    def write(self, d, name, doc):
        Path(d, name).write_text(json.dumps(doc), encoding="utf-8")

    def good(self):
        return {"schema": "pre-pr-gate/v1", "verdict": "pass",
                "legs": {k: {"status": "pass", "exit_code": 0, "command": "c"} for k in ("ci", "pytest", "ratchet")}}

    def test_missing_receipt_fails(self):
        with tempfile.TemporaryDirectory() as d:
            self.assertTrue(any("missing" in p for p in trn.check_receipt(Path(d, "pr-1.json"))))

    def test_good_receipt_ok_and_failed_or_partial_receipts_fail(self):
        with tempfile.TemporaryDirectory() as d:
            self.write(d, "ok.json", self.good())
            self.assertEqual(trn.check_receipt(Path(d, "ok.json")), [])
            r = self.good(); r["verdict"] = "fail"; self.write(d, "f.json", r)
            self.assertTrue(trn.check_receipt(Path(d, "f.json")))
            r = self.good(); r["legs"]["ratchet"]["status"] = "missing"; self.write(d, "m.json", r)
            self.assertTrue(trn.check_receipt(Path(d, "m.json")))
            r = self.good(); del r["legs"]["ci"]; self.write(d, "n.json", r)
            self.assertTrue(trn.check_receipt(Path(d, "n.json")))
            r = self.good(); r["schema"] = "x"; self.write(d, "s.json", r)
            self.assertTrue(trn.check_receipt(Path(d, "s.json")))


class GitSteps(unittest.TestCase):
    def setUp(self):
        self._t = tempfile.TemporaryDirectory()
        self.repo = Path(self._t.name, "repo")
        self.repo.mkdir()
        git(self.repo, "init", "-q", "-b", "main")
        commit(self.repo, "base.txt")
        git(self.repo, "checkout", "-qb", "ba"); commit(self.repo, "a.txt")
        git(self.repo, "checkout", "-qb", "bb"); commit(self.repo, "b.txt")       # stacked on ba
        git(self.repo, "checkout", "-q", "main")
        self.ls = lanes(("a", "ba", [], 5), ("b", "bb", ["a"], 5))

    def tearDown(self):
        self._t.cleanup()

    def test_stacked_lanes_need_no_restack(self):
        self.assertEqual(trn.check_restack(self.repo, self.ls), [])

    def test_moved_dependency_needs_restack(self):
        git(self.repo, "checkout", "-q", "ba"); commit(self.repo, "a2.txt"); git(self.repo, "checkout", "-q", "main")
        self.assertTrue(any("restack" in p for p in trn.check_restack(self.repo, self.ls)))

    def test_missing_branch_is_reported(self):
        self.assertTrue(trn.check_restack(self.repo, lanes(("z", "nope", [], 1))))

    def test_merge_merges_in_order_into_the_train_branch(self):
        trn.merge(self.repo, "main", "train", self.ls)
        self.assertEqual(git(self.repo, "rev-parse", "--abbrev-ref", "HEAD"), "train")
        self.assertTrue(Path(self.repo, "a.txt").exists() and Path(self.repo, "b.txt").exists())
        log = git(self.repo, "log", "--format=%s", "--merges")
        self.assertLess(log.index("bb"), log.index("ba"))  # newest first: bb merged after ba

    def test_merge_refuses_out_of_order_lanes(self):
        with self.assertRaises(ValueError):
            trn.merge(self.repo, "main", "train", list(reversed(self.ls)))

    def test_exchange_bundle_written_for_full_groups(self):
        ls = lanes(("a", "ba", [], 30), ("b", "bb", ["a"], 30))
        out = Path(self._t.name, "exchange")
        written = trn.write_bundles(self.repo, "main", ls, 35, out)
        self.assertEqual(len(written), 1)              # the last group is the open PR, not yet at the cap
        self.assertTrue(written[0].is_file() and written[0].stat().st_size > 0)


class TrainReceipt(unittest.TestCase):
    def test_receipt_is_written_with_a_verdict_and_per_pr_w0_receipt_refs(self):
        with tempfile.TemporaryDirectory() as d:
            repo = Path(d, "r"); repo.mkdir()
            git(repo, "init", "-q", "-b", "main"); commit(repo, "x")
            git(repo, "checkout", "-qb", "ba"); commit(repo, "a"); git(repo, "checkout", "-q", "main")
            rec = Path(d, "w0"); rec.mkdir()
            Path(rec, "pr-1.json").write_text(json.dumps(Receipts().good()), encoding="utf-8")
            m = {"base": "main", "train_branch": "train", "pr_cap_hours": 35, "receipts_dir": str(rec),
                 "lanes": lanes(("a", "ba", [], 5))}
            out = Path(d, "out", "train.json")
            doc = trn.make_receipt(repo, m, out)
            self.assertEqual(doc["schema"], "train-receipt/v1")
            self.assertEqual(doc["verdict"], "pass")
            self.assertEqual(doc["prs"][0]["lanes"], ["a"])
            self.assertEqual(json.loads(out.read_text(encoding="utf-8")), doc)
            self.assertTrue((out.parent / doc["prs"][0]["w0_receipt"]).is_file())
            Path(rec, "pr-1.json").unlink()
            self.assertEqual(trn.make_receipt(repo, m, out)["verdict"], "fail")


class Consolidated(unittest.TestCase):
    """An already-merged train (one consolidated PR, lanes merged earlier): the order is observed from git, not asserted."""

    def setUp(self):
        self._t = tempfile.TemporaryDirectory()
        d = self._t.name
        self.repo = Path(d, "repo"); self.repo.mkdir()
        git(self.repo, "init", "-q", "-b", "main"); commit(self.repo, "base.txt")
        git(self.repo, "checkout", "-qb", "train")
        for n in ("a", "b"):
            git(self.repo, "checkout", "-qb", f"l{n}", "train"); commit(self.repo, f"{n}.txt")
            git(self.repo, "checkout", "-q", "train"); git(self.repo, "merge", "--no-ff", "-q", "-m", f"merge l{n}", f"l{n}")
        self.rec = Path(d, "w0"); self.rec.mkdir()
        Path(self.rec, "pr-96.json").write_text(json.dumps(Receipts().good()), encoding="utf-8")
        self.out = Path(d, "out", "trn0.json")
        self.m = {"mode": "consolidated", "pr": 96, "base": "main", "train_branch": "train", "pr_cap_hours": 35,
                  "cap_deviation": {"authority": "user ruling: few large PRs (journal CL-0042)"},
                  "receipts_dir": str(self.rec),
                  "lanes": [{"id": "a", "branch": "la", "deps": [], "lane_hours": 30},
                            {"id": "b", "branch": "lb", "deps": ["a"], "lane_hours": None}]}

    def tearDown(self):
        self._t.cleanup()

    def test_observed_merge_order_pr_number_unknown_hours_and_disclosed_cap_deviation(self):
        self.m["lanes"][1]["lane_hours"] = 20
        doc = trn.make_receipt(self.repo, self.m, self.out)
        self.assertEqual(doc["verdict"], "pass", doc["problems"])
        self.assertEqual([x["lane"] for x in doc["merge_order"]], ["a", "b"])
        self.assertEqual(doc["prs"][0]["n"], 96)
        self.assertEqual(doc["prs"][0]["lane_hours"], 50)
        self.assertTrue((self.out.parent / doc["prs"][0]["w0_receipt"]).is_file())
        self.assertTrue(any("35" in d and "ruling" in d for d in doc["deviations"]))

    def test_null_lane_hours_are_recorded_as_unknown_not_invented(self):
        doc = trn.make_receipt(self.repo, self.m, self.out)
        self.assertEqual(doc["verdict"], "pass", doc["problems"])
        self.assertEqual(doc["prs"][0]["lane_hours"], 30)
        self.assertEqual(doc["prs"][0]["lane_hours_unknown"], ["b"])

    def test_over_cap_without_a_recorded_authority_fails(self):
        self.m["lanes"][1]["lane_hours"] = 20
        del self.m["cap_deviation"]
        self.assertEqual(trn.make_receipt(self.repo, self.m, self.out)["verdict"], "fail")

    def test_lane_not_contained_in_the_train_fails(self):
        git(self.repo, "checkout", "-qb", "lc", "main"); commit(self.repo, "c.txt"); git(self.repo, "checkout", "-q", "train")
        self.m["lanes"].append({"id": "c", "branch": "lc", "deps": [], "lane_hours": 1})
        doc = trn.make_receipt(self.repo, self.m, self.out)
        self.assertEqual(doc["verdict"], "fail")
        self.assertTrue(any("not merged" in p for p in doc["problems"]))

    def test_dependency_merged_after_its_dependant_fails(self):
        self.m["lanes"][0]["deps"] = ["b"]
        doc = trn.make_receipt(self.repo, self.m, self.out)
        self.assertEqual(doc["verdict"], "fail")
        self.assertTrue(any("before its dependency" in p for p in doc["problems"]))

    def test_missing_w0_receipt_fails(self):
        Path(self.rec, "pr-96.json").unlink()
        self.assertEqual(trn.make_receipt(self.repo, self.m, self.out)["verdict"], "fail")


class Cli(unittest.TestCase):
    def test_check_exits_1_on_out_of_order_manifest_and_0_when_clean(self):
        with tempfile.TemporaryDirectory() as d:
            repo = Path(d, "r"); repo.mkdir()
            git(repo, "init", "-q", "-b", "main"); commit(repo, "x")
            git(repo, "branch", "ba"); git(repo, "branch", "bb")
            rec = Path(d, "receipts"); rec.mkdir()
            Path(rec, "pr-1.json").write_text(json.dumps(Receipts().good()), encoding="utf-8")

            def run(ls):
                m = Path(d, "m.json")
                m.write_text(json.dumps({"schema": "train/v1", "base": "main", "train_branch": "train", "pr_cap_hours": 35,
                                         "receipts_dir": str(rec), "lanes": ls}), encoding="utf-8")
                return subprocess.run([sys.executable, str(GOV / "trn0_train.py"), "check", "--manifest", str(m),
                                       "--repo", str(repo)], capture_output=True, text=True)
            self.assertEqual(run(lanes(("b", "bb", ["a"], 5), ("a", "ba", [], 5))).returncode, 1)
            git(repo, "checkout", "-q", "ba"); commit(repo, "a"); git(repo, "checkout", "-q", "main")   # ba moved, bb not stacked
            r = run(lanes(("a", "ba", [], 5), ("b", "bb", ["a"], 5)))
            self.assertEqual(r.returncode, 1, r.stdout)
            git(repo, "branch", "-f", "bb", "ba")
            r = run(lanes(("a", "ba", [], 5), ("b", "bb", ["a"], 5)))
            self.assertEqual(r.returncode, 0, r.stdout + r.stderr)


if __name__ == "__main__":
    unittest.main()
