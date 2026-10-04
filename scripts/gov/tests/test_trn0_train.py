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
                "legs": {k: {"status": "pass", "exit_code": 0} for k in ("ci", "pytest", "ratchet")}}

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
