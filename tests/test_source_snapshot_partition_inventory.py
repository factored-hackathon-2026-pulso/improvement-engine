import json
import unittest
from pathlib import Path

from contracts.validate_fixtures import validate_source_snapshot
from contracts.validate_fixtures import validate_source_contract


FIXTURE = (
    Path(__file__).parents[1]
    / "contracts"
    / "fixtures"
    / "sources-v1"
    / "validation"
    / "source-snapshot.json"
)
CONTRACTS = Path(__file__).parents[1] / "contracts" / "sources"


def source_contract(name):
    return json.loads((CONTRACTS / name).read_text(encoding="utf-8"))


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

    def test_original_contact_and_complaint_contracts_cover_dictionary_headers(self):
        expected_headers = {
            "call_center_interactions.v1.json": (
                "interaction_id interaction_date process_date customer_id agent_id "
                "interaction_type channel contact_reason reason_category duration_seconds "
                "wait_time_seconds was_resolved requires_followup detected_sentiment "
                "sentiment_score customer_detected_accent agent_used_accent was_escalated "
                "mentioned_products has_transcript has_recording"
            ).split(),
            "complaints.v1.json": (
                "complaint_id creation_date process_date customer_id case_type category "
                "subcategory reception_channel affected_product_id related_branch_id "
                "origin_interaction_id description claimed_amount currency priority status "
                "assigned_agent_id assignment_date first_response_date resolution_date "
                "closing_date sla_breached resolution_days resolution compensation_granted "
                "resolution_satisfaction is_repeat_complainer"
            ).split(),
        }
        for filename, expected in expected_headers.items():
            with self.subTest(table=filename):
                contract = source_contract(filename)
                self.assertEqual(contract["contract_version"], {"major": 1, "minor": 0})
                self.assertEqual(
                    [column["name"] for column in contract["columns"]], expected
                )
                self.assertEqual(len(expected), len(set(expected)))
                self.assertTrue(contract["read_only"])

    def test_contact_contract_does_not_weaken_required_dictionary_fields(self):
        contract = source_contract("call_center_interactions.v1.json")
        columns = {column["name"]: column for column in contract["columns"]}
        for name in (
            "interaction_id",
            "interaction_date",
            "process_date",
            "customer_id",
            "interaction_type",
            "channel",
            "contact_reason",
            "reason_category",
            "requires_followup",
            "was_escalated",
            "has_transcript",
            "has_recording",
        ):
            with self.subTest(column=name):
                self.assertFalse(columns[name]["nullable"])
        self.assertEqual(columns["duration_seconds"]["logical_type"], "int")
        self.assertEqual(columns["wait_time_seconds"]["logical_type"], "int")

    def test_free_text_complaint_description_is_explicitly_restricted(self):
        contract = source_contract("complaints.v1.json")
        description = next(
            column for column in contract["columns"] if column["name"] == "description"
        )
        self.assertEqual(description["classification"], "restricted")
        self.assertNotIn(
            "restricted", contract["access_policy"]["permitted_classifications"]
        )
        self.assertEqual(validate_source_contract(contract), [])

        contract["access_policy"]["permitted_classifications"].append("restricted")
        self.assertIn(
            "invalid_source_access_policy", validate_source_contract(contract)
        )


if __name__ == "__main__":
    unittest.main()
