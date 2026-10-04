"""GT0 collector: turns a real replay run into the artifacts the gate reads. Pure helpers are tested here; the replay
itself is exercised by the gate run (needs the e2e-core dependencies and the sensor exe)."""
import hashlib
import json
import sys
import tempfile
import unittest
from pathlib import Path

GOV = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(GOV))
import gt0_collect as col  # noqa: E402


class Helpers(unittest.TestCase):
    def test_fixtures_digest_is_stable_content_addressed_and_line_ending_insensitive(self):
        with tempfile.TemporaryDirectory() as d:
            Path(d, "a.json").write_bytes(b'{"x":1}\r\n')
            Path(d, "sub").mkdir()
            Path(d, "sub", "b.json").write_bytes(b"{}\n")
            d1 = col.fixtures_digest(Path(d))
            Path(d, "a.json").write_bytes(b'{"x":1}\n')
            self.assertEqual(col.fixtures_digest(Path(d)), d1)
            Path(d, "a.json").write_bytes(b'{"x":2}\n')
            self.assertNotEqual(col.fixtures_digest(Path(d)), d1)
            self.assertTrue(d1.startswith("sha256:"))

    def test_replay_record_binds_report_bytes_and_derives_the_run_id_from_content(self):
        with tempfile.TemporaryDirectory() as d:
            rp = Path(d, "report.json")
            rp.write_text("{}", encoding="utf-8")
            rec = col.replay_record(rp, "sha256:" + "0" * 64, {"misses": 0, "calls": 12}, [])
            self.assertEqual(rec["schema"], "gt0-replay/v1")
            self.assertEqual(rec["report_sha256"], "sha256:" + hashlib.sha256(b"{}").hexdigest())
            self.assertEqual((rec["misses"], rec["calls"]), (0, 12))
            self.assertEqual(rec["run_id"], col.replay_record(rp, "sha256:" + "0" * 64, {"misses": 0, "calls": 12}, [])["run_id"])
            rp.write_text("{ }", encoding="utf-8")
            self.assertNotEqual(rec["run_id"], col.replay_record(rp, "sha256:" + "0" * 64, {"misses": 0, "calls": 12}, [])["run_id"])

    def test_missing_replay_counters_are_a_miss_never_a_zero(self):
        with tempfile.TemporaryDirectory() as d:
            rp = Path(d, "r.json")
            rp.write_text("{}", encoding="utf-8")
            self.assertNotEqual(col.replay_record(rp, "sha256:x", {}, [])["misses"], 0)


if __name__ == "__main__":
    unittest.main()
