import json
import io
import tempfile
import unittest
from contextlib import redirect_stderr
from pathlib import Path

from scripts.aggregate.agent_runs.cli import main


class AgentRunCliTests(unittest.TestCase):
    @staticmethod
    def synthetic_export() -> dict:
        return {
            "_label": "SYNTHETIC CLI TEST",
            "runs": {"pages": [
                {"requested_after": None, "items": [{
                    "run_id": "synthetic-run-private",
                    "status": "closed",
                    "outcome": "completed",
                    "closed_at": "2026-01-01T00:00:00Z",
                }], "next_after": "opaque-runs-cursor"},
                {"requested_after": "opaque-runs-cursor", "items": [], "next_after": "opaque-runs-cursor"},
            ]},
            "events": {"synthetic-run-private": [
                {"requested_after": None, "items": [{
                    "type": "run_closed",
                    "run_id": "synthetic-run-private",
                    "ts": "2026-01-01T00:00:00Z",
                    "payload": {"outcome": "completed"},
                }], "next_after": "opaque-events-cursor"},
                {"requested_after": "opaque-events-cursor", "items": [], "next_after": "opaque-events-cursor"},
            ]},
        }

    def test_complete_synthetic_export_writes_suppressed_aggregate_outside_checkout(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "export.json"
            output = Path(directory) / "agent-run-outcomes.json"
            source.write_text(json.dumps(self.synthetic_export()), encoding="utf-8")
            code = main(["--input", str(source), "--output", str(output)])
            self.assertEqual(code, 0)
            report = json.loads(output.read_text(encoding="utf-8"))
            self.assertEqual(report["protocol"], "agent-run-outcomes.v2")
            self.assertEqual(report["availability"], "below_privacy_floor")
            self.assertEqual(report["cells"], [])
            serialized = output.read_text(encoding="utf-8")
            self.assertNotIn("synthetic-run-private", serialized)

    def test_existing_output_is_never_overwritten(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "export.json"
            output = Path(directory) / "existing-report.json"
            source.write_text(json.dumps(self.synthetic_export()), encoding="utf-8")
            output.write_bytes(b"user-owned bytes\n")
            with redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
                main(["--input", str(source), "--output", str(output)])
            self.assertEqual(output.read_bytes(), b"user-owned bytes\n")

    def test_recorded_fixture_with_nonterminal_cursors_fails_closed(self):
        fixture = Path("scripts/triggers/fixtures/export_recorded.json").resolve()
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "report.json"
            with redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
                main(["--input", str(fixture), "--output", str(output)])
            self.assertFalse(output.exists())

    def test_refuses_output_inside_repository_without_creating_file(self):
        fixture = Path("scripts/triggers/fixtures/export_recorded.json").resolve()
        output = Path("scripts/aggregate/agent_runs/unsafe-output.json").resolve()
        with redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
            main(["--input", str(fixture), "--output", str(output)])
        self.assertFalse(output.exists())


if __name__ == "__main__":
    unittest.main()
