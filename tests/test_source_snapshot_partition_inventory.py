import json
import unittest
from pathlib import Path

from contracts.validate_fixtures import validate_source_snapshot


FIXTURE = (
    Path(__file__).parents[1]
    / "contracts"
    / "fixtures"
    / "sources-v1"
    / "validation"
    / "source-snapshot.json"
)


class SourceSnapshotPartitionInventoryTests(unittest.TestCase):
    def setUp(self):
        self.snapshot = json.loads(FIXTURE.read_text(encoding="utf-8"))

    def test_legacy_snapshot_without_inventory_seal_remains_valid(self):
        self.assertEqual(validate_source_snapshot(self.snapshot), [])

    def test_optional_inventory_seal_accepts_sha256_digest(self):
        self.snapshot["sources"][0]["partition_inventory_digest"] = (
            "sha256:" + "a" * 64
        )
        self.assertEqual(validate_source_snapshot(self.snapshot), [])

    def test_optional_inventory_seal_rejects_malformed_digest(self):
        self.snapshot["sources"][0]["partition_inventory_digest"] = "not-a-digest"
        self.assertIn("invalid_snapshot_source", validate_source_snapshot(self.snapshot))

    def test_optional_inventory_seal_rejects_null(self):
        self.snapshot["sources"][0]["partition_inventory_digest"] = None
        self.assertIn("invalid_snapshot_source", validate_source_snapshot(self.snapshot))


if __name__ == "__main__":
    unittest.main()
