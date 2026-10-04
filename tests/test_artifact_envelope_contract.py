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
    spec22_coverage_report,
    validate_source_contract,
    validate_source_ref,
    validate_source_snapshot,
    validate_spec22_coverage,
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

    def test_spec22_initial_family_inventory_entries_are_all_present(self) -> None:
        report = spec22_coverage_report()
        self.assertEqual(report["family_count"], 11)
        self.assertEqual(report["missing_families"], [])
        self.assertEqual(report["errors"], [])
        self.assertEqual(
            report["missing_wire_contracts"],
            ["signal", "workflow_bridge", "tree", "release_ack", "memory_wiki", "read_model"],
        )

    def test_spec22_strict_gate_fails_when_a_family_is_removed(self) -> None:
        report = spec22_coverage_report()
        report["families"] = [item for item in report["families"] if item["name"] != "tree"]
        # The data-only evaluator lets CI test a mutated inventory without
        # writing to the repository fixture manifest.
        from contracts.validate_fixtures import validate_spec22_inventory

        self.assertIn("missing_family:tree", validate_spec22_inventory(report["families"]))

    def test_spec22_strict_gate_reports_unpublished_wire_contracts(self) -> None:
        errors = validate_spec22_coverage(strict=True)
        self.assertEqual(
            errors,
            [
                "wire_contract_not_published:signal",
                "wire_contract_not_published:workflow_bridge",
                "wire_contract_not_published:tree",
                "wire_contract_not_published:release_ack",
                "wire_contract_not_published:memory_wiki",
                "wire_contract_not_published:read_model",
            ],
        )

    def test_spec22_read_model_internal_types_are_not_published_wire_schema(self) -> None:
        from copy import deepcopy

        from contracts.validate_fixtures import validate_spec22_inventory

        read_model = next(
            family for family in spec22_coverage_report()["families"] if family["name"] == "read_model"
        )
        self.assertEqual(read_model["contract_status"], "partial")
        self.assertNotIn("contracts/validate_fixtures.py", read_model["contract_refs"])
        self.assertNotIn("crates/core/src/run_activity.rs", read_model["contract_refs"])

        promoted_without_schema = deepcopy(spec22_coverage_report()["families"])
        next(item for item in promoted_without_schema if item["name"] == "read_model")["contract_status"] = "published"
        self.assertIn(
            "published_schema_missing_ref:read_model",
            validate_spec22_inventory(promoted_without_schema),
        )

    def test_spec22_inventory_rejects_invalid_fixture_refs(self) -> None:
        from copy import deepcopy

        from contracts.validate_fixtures import validate_spec22_inventory

        families = deepcopy(spec22_coverage_report()["families"])
        family = next(item for item in families if item["name"] == "source_snapshot")
        family["fixture_refs"] = "not-a-list"
        self.assertIn("invalid_fixture_refs:source_snapshot", validate_spec22_inventory(families))

    def test_spec22_inventory_rejects_absolute_and_parent_fixture_refs(self) -> None:
        from copy import deepcopy

        from contracts.validate_fixtures import validate_spec22_inventory

        for unsafe_ref in ("C:/outside/fixture.json", "../../outside/fixture.json", "/outside/fixture.json"):
            with self.subTest(ref=unsafe_ref):
                families = deepcopy(spec22_coverage_report()["families"])
                family = next(item for item in families if item["name"] == "source_snapshot")
                family["fixture_refs"] = [unsafe_ref]
                self.assertIn(
                    f"unsafe_fixture_ref:source_snapshot:{unsafe_ref}",
                    validate_spec22_inventory(families),
                )

    def test_spec22_inventory_rejects_fixture_paths_outside_repository(self) -> None:
        from copy import deepcopy

        from contracts.validate_fixtures import validate_spec22_inventory

        families = deepcopy(spec22_coverage_report()["families"])
        family = next(item for item in families if item["name"] == "source_snapshot")
        family["fixture_refs"] = ["contracts/fixtures/spec22-v1/../../../../outside.json"]
        self.assertIn(
            "unsafe_fixture_ref:source_snapshot:contracts/fixtures/spec22-v1/../../../../outside.json",
            validate_spec22_inventory(families),
        )

    def test_spec22_report_returns_validation_errors_for_malformed_manifest(self) -> None:
        from unittest.mock import patch

        with patch(
            "contracts.validate_fixtures._load",
            return_value={"families": [{"contract_status": {}, "contract_refs": [], "fixture_refs": []}]},
        ):
            report = spec22_coverage_report()

        self.assertIn("invalid_family_entry", report["errors"])


if __name__ == "__main__":
    unittest.main()
