"""Contract fixtures must remain executable without third-party dependencies."""

from __future__ import annotations

import json
import sys
import unittest
from copy import deepcopy
from pathlib import Path


REPOSITORY = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(REPOSITORY))

from contracts.validate_fixtures import (  # noqa: E402
    validate_envelope,
    validate_source_contract,
    validate_source_ref,
    validate_source_snapshot,
)


FIXTURES = REPOSITORY / "contracts" / "fixtures" / "artifact-envelope-v1"


class ArtifactEnvelopeContractTest(unittest.TestCase):
    def load_fixture(self, name: str) -> dict[str, object]:
        return json.loads((FIXTURES / name).read_text(encoding="utf-8"))

    def test_valid_fixture_has_no_contract_errors(self) -> None:
        self.assertEqual(validate_envelope(self.load_fixture("valid-signal.json")), [])

    def test_negative_fixtures_expose_their_specific_boundary(self) -> None:
        cases = {
            "invalid-unknown-major.json": "unsupported_contract_major",
            "invalid-cross-tenant-parent.json": "cross_tenant_reference",
            "invalid-revision-zero.json": "invalid_artifact_revision",
        }

        for fixture, expected_error in cases.items():
            with self.subTest(fixture=fixture):
                self.assertIn(expected_error, validate_envelope(self.load_fixture(fixture)))

    def test_schema_boundaries_rejected_by_semantic_validator(self) -> None:
        valid = self.load_fixture("valid-signal.json")

        unexpected = deepcopy(valid)
        unexpected["unexpected"] = True
        self.assertIn("unexpected_envelope_field", validate_envelope(unexpected))

        unexpected_version_field = deepcopy(valid)
        unexpected_version_field["contract_version"]["future"] = 1
        self.assertIn("unsupported_contract_major", validate_envelope(unexpected_version_field))

        duplicate_parent = deepcopy(valid)
        duplicate_parent["parent_refs"].append(deepcopy(duplicate_parent["parent_refs"][0]))
        self.assertIn("duplicate_parent_ref", validate_envelope(duplicate_parent))

    def test_source_contract_and_reference_fixtures_are_structurally_usable(self) -> None:
        source_fixture_root = REPOSITORY / "contracts" / "fixtures" / "sources-v1"
        load = lambda name: json.loads((source_fixture_root / name).read_text(encoding="utf-8"))
        self.assertEqual(validate_source_contract(load("valid-call-center-interactions.json")), [])
        self.assertEqual(validate_source_snapshot(load("valid-source-snapshot.json")), [])
        self.assertEqual(validate_source_ref(load("valid-source-ref.json")), [])
        self.assertIn(
            "invalid_source_pk",
            validate_source_ref(load("invalid-source-ref-empty-pk.json")),
        )
        self.assertIn(
            "invalid_source_access_mode",
            validate_source_contract(load("invalid-source-contract-writable.json")),
        )

        snapshot = load("valid-source-snapshot.json")
        snapshot["sources"][0]["source_contract_ref"] = False
        self.assertIn("invalid_snapshot_source", validate_source_snapshot(snapshot))

        source_ref = load("valid-source-ref.json")
        source_ref["snapshot_ref"]["tenant_id"] = "other-tenant"
        self.assertIn("cross_tenant_reference", validate_source_ref(source_ref))

        source_ref = load("valid-source-ref.json")
        source_ref["table"] = "INVALID-table"
        self.assertIn("invalid_table", validate_source_ref(source_ref))

        source_contract = load("valid-call-center-interactions.json")
        source_contract["access_policy"]["permitted_classifications"] = ["aggregated"]
        self.assertIn(
            "source_column_classification_not_permitted",
            validate_source_contract(source_contract),
        )

        source_contract = load("valid-call-center-interactions.json")
        source_contract["columns"][4]["name"] = "contact_reason"
        self.assertIn("duplicate_source_column", validate_source_contract(source_contract))

    def test_golden_header_matches_the_executable_source_contract(self) -> None:
        contract = json.loads(
            (REPOSITORY / "contracts" / "sources" / "call_center_interactions.v1.json").read_text(encoding="utf-8")
        )
        header = (
            REPOSITORY
            / "contracts"
            / "fixtures"
            / "sources-v1"
            / "golden"
            / "call_center_interactions.header.csv"
        ).read_text(encoding="utf-8").rstrip("\r\n").split(",")
        self.assertEqual(header, [column["name"] for column in contract["columns"]])


if __name__ == "__main__":
    unittest.main()
