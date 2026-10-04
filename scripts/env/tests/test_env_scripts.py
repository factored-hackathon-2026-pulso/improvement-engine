"""Tests for G0e (target dirs) and WTR1 (worktree inventory). Run: python -m unittest discover -s scripts/env/tests"""
import json
import subprocess
import tempfile
import unittest
from pathlib import Path

ENV = Path(__file__).resolve().parents[1]


def ps(script, *args):
    return subprocess.run(
        ["pwsh", "-NoProfile", "-File", str(ENV / script), *args],
        capture_output=True, text=True,
    )


class TargetDirs(unittest.TestCase):
    def test_equal_dirs_fail(self):
        r = ps("check-target-dirs.ps1", "-Assignment", "a=D:/t/x,b=d:/T/x/")
        self.assertEqual(r.returncode, 1, r.stdout + r.stderr)

    def test_distinct_dirs_pass(self):
        r = ps("check-target-dirs.ps1", "-Assignment", r"a=D:\t\a,b=D:\t\b")
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)

    def test_dotdot_alias_and_relative_fail(self):
        r = ps("check-target-dirs.ps1", "-Assignment", r"a=D:\t\x,b=D:\t\y\..\x")
        self.assertEqual(r.returncode, 1, r.stdout + r.stderr)
        self.assertEqual(ps("check-target-dirs.ps1", "-Assignment", r"a=D:rel").returncode, 1)
        self.assertEqual(ps("check-target-dirs.ps1", "-Assignment", r"a=D:\..\..\Windows,b=C:\x").returncode, 1)

    def test_c_drive_fails(self):
        r = ps("check-target-dirs.ps1", "-Assignment", r"a=C:\t\a")
        self.assertEqual(r.returncode, 1)

    def test_lane_target_dir_is_on_d_and_per_lane(self):
        a = ps("lane-target-dir.ps1", "-Lane", "l-env").stdout.strip()
        b = ps("lane-target-dir.ps1", "-Lane", "l-gov").stdout.strip()
        self.assertTrue(a.upper().startswith("D:\\"))
        self.assertNotEqual(a, b)

    def test_lane_target_dir_rejects_c_root_and_bad_lane(self):
        self.assertEqual(ps("lane-target-dir.ps1", "-Lane", "x", "-Root", r"C:\cargo").returncode, 1)
        self.assertEqual(ps("lane-target-dir.ps1", "-Lane", "../x").returncode, 1)

    def test_slot_protocol_documented(self):
        self.assertIn("CARGO_TARGET_DIR", (ENV / "SLOT_PROTOCOL.md").read_text(encoding="utf-8"))


def porcelain(entries):
    out = []
    for path, branch in entries:
        out += [f"worktree {path}", "HEAD abc", f"branch refs/heads/{branch}", ""]
    return "\n".join(out)


class Inventory(unittest.TestCase):
    def run_inv(self, entries, merged, *extra):
        with tempfile.TemporaryDirectory() as d:
            lf, mf = Path(d, "wt.txt"), Path(d, "m.txt")
            lf.write_text(porcelain(entries), encoding="utf-8")
            mf.write_text("\n".join(merged), encoding="utf-8")
            return ps("worktree-inventory.ps1", "-WorktreeListFile", str(lf),
                      "-MergedBranchesFile", str(mf), "-Json", *extra)

    ENTRIES = [
        (r"D:/w/main", "main"),
        (r"D:/w/a-claude-old", "claude/old"),
        (r"D:/w/a-claude-new", "claude/new"),
        (r"D:/w/a-codex-x", "codex/x"),
    ]

    def test_classification_and_dry_run_plan(self):
        r = self.run_inv(self.ENTRIES, ["claude/old", "codex/x", "main"])
        self.assertEqual(r.returncode, 0, r.stderr)
        data = json.loads(r.stdout)
        by = {w["branch"]: w for w in data["worktrees"]}
        self.assertEqual(by["claude/old"]["owner"], "claude")
        self.assertTrue(by["claude/old"]["merged"])
        self.assertFalse(by["claude/new"]["merged"])
        self.assertEqual(by["codex/x"]["owner"], "codex")
        self.assertEqual(data["unmerged"], ["claude/new"])
        plan = data["retirement_plan"]
        self.assertTrue(data["dry_run"])
        self.assertEqual([p["branch"] for p in plan], ["claude/old"])  # never codex, never main

    def test_primary_worktree_never_in_plan(self):
        r = self.run_inv([(r"D:/w/a-claude-main", "claude/first")] + self.ENTRIES[1:], ["claude/first"])
        self.assertEqual(json.loads(r.stdout)["retirement_plan"], [])

    def test_above_cap_exits_1(self):
        r = self.run_inv(self.ENTRIES, [], "-Cap", "3")
        self.assertEqual(r.returncode, 1)

    def test_at_cap_exits_0(self):
        self.assertEqual(self.run_inv(self.ENTRIES, [], "-Cap", "4").returncode, 0)

    def test_default_cap_is_40(self):
        many = [(f"D:/w/c{i}", f"claude/b{i}") for i in range(41)]
        self.assertEqual(self.run_inv(many, []).returncode, 1)


if __name__ == "__main__":
    unittest.main()
