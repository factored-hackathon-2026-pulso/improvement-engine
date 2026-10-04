"""GT05 collector helpers (the Rust-steps thread itself is exercised by the collector run and needs the e2e-core deps)."""
import hashlib
import sys
import tempfile
import unittest
from pathlib import Path

GOV = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(GOV))
import gt05_collect as col  # noqa: E402


class Helpers(unittest.TestCase):
    def test_run_record_binds_report_and_exe_bytes(self):
        with tempfile.TemporaryDirectory() as d:
            rp, exe = Path(d, "r.json"), Path(d, "steps_cli.exe")
            rp.write_text("{}", encoding="utf-8")
            exe.write_bytes(b"bin")
            rec = col.run_record(rp, exe, {"calls": 5, "misses": 0}, 0, "cmd")
            self.assertEqual(rec["schema"], "gt05-run/v1")
            self.assertEqual(rec["mode"], "rust")
            self.assertEqual(rec["report_sha256"], "sha256:" + hashlib.sha256(b"{}").hexdigest())
            self.assertEqual(rec["steps_exe_sha256"], hashlib.sha256(b"bin").hexdigest())
            self.assertEqual((rec["calls"], rec["misses"], rec["exit_code"]), (5, 0, 0))

    def test_missing_replay_counters_are_a_miss_never_a_zero(self):
        with tempfile.TemporaryDirectory() as d:
            rp, exe = Path(d, "r.json"), Path(d, "e")
            rp.write_text("{}", encoding="utf-8")
            exe.write_bytes(b"b")
            rec = col.run_record(rp, exe, {}, 0, "cmd")
            self.assertNotEqual(rec["misses"], 0)
            self.assertEqual(rec["calls"], 0)

    def test_pytest_summary_parser(self):
        self.assertEqual(col.parse_pytest_summary("..\n7 passed, 1 skipped in 3.2s\n"), (7, 0, 1))
        self.assertEqual(col.parse_pytest_summary("1 failed, 6 passed in 1s"), (6, 1, 0))
        self.assertEqual(col.parse_pytest_summary("no tests ran"), (0, 0, 0))

    def test_ratchet_receipt_never_passes_on_skips_only_or_nonzero_exit(self):
        ok = col.ratchet_receipt("cmd", 0, "7 passed in 1s")
        self.assertEqual((ok["exit_code"], ok["passed"], ok["failed"]), (0, 7, 0))
        self.assertEqual(col.ratchet_receipt("cmd", 1, "1 failed, 6 passed")["exit_code"], 1)
        self.assertEqual(col.ratchet_receipt("cmd", 0, "7 skipped in 1s")["passed"], 0)


if __name__ == "__main__":
    unittest.main()
