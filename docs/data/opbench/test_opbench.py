"""Synthetic acceptance tests for OPBENCH-lite's deterministic statistics."""

import random
import unittest

from opbench import (
    E0Case,
    MetricAccumulator,
    Observation,
    assess_accumulator,
    assess_cells,
        assess_e1,
        assess_e1_counts,
    benjamini_hochberg,
    bucket_for_key,
    normalize_channel,
    normalize_reason,
    validate_safe_pack,
)


class OpbenchStatisticsTests(unittest.TestCase):
    def setUp(self):
        self.cells = [
            {"reason_category": reason, "channel": channel}
            for reason in ("complaint", "transactional")
            for channel in ("phone", "web")
        ]

    @staticmethod
    def observations(target_events):
        rows = []
        cells = [
            ("complaint", "phone"),
            ("complaint", "web"),
            ("transactional", "phone"),
            ("transactional", "web"),
        ]
        for split in ("discovery", "replication"):
            for cell_index, cell in enumerate(cells):
                event_count = target_events if cell_index == 0 else 100
                for row_index in range(1000):
                    rows.append(
                        Observation(
                            cell={
                                "reason_category": cell[0],
                                "channel": cell[1],
                            },
                            event=row_index < event_count,
                            split=split,
                        )
                    )
        return rows

    def test_closed_vocabulary_is_stable_and_unknown_values_are_not_exposed(self):
        self.assertEqual(normalize_reason("Queja"), "complaint")
        self.assertEqual(normalize_reason("transaccional"), "transactional")
        self.assertEqual(normalize_reason("private free text"), "unclassified")
        self.assertEqual(normalize_channel("Sucursal"), "branch")
        self.assertEqual(normalize_channel("IVR"), "phone")
        self.assertEqual(normalize_channel("App"), "mobile_app")
        self.assertEqual(normalize_channel("private channel"), "other")

    def test_customer_split_is_deterministic_and_cross_table_stable(self):
        self.assertEqual(
            bucket_for_key("opbench-lite:v1:bank:", "customer-1"),
            bucket_for_key("opbench-lite:v1:bank:", "customer-1"),
        )
        self.assertIn(
            bucket_for_key("opbench-lite:v1:bank:", "customer-1"),
            {"discovery", "replication"},
        )

    def test_bh_adjustment_is_monotonic_and_covers_full_family(self):
        self.assertEqual(benjamini_hochberg([0.01, 0.04, 0.03, 1.0]), [0.04, 0.053333, 0.053333, 1.0])
        self.assertEqual(benjamini_hochberg([0.01, None, 0.03]), [0.03, 1.0, 0.045])

    def test_anomaly_is_replicated_then_refuted_after_outcome_permutation(self):
        observed = self.observations(target_events=400)
        initial = assess_cells("M1", self.cells, observed, family_size=4)
        target = next(
            result for result in initial
            if result["cell"] == {"reason_category": "complaint", "channel": "phone"}
        )
        self.assertEqual(target["status"], "corroborated")
        self.assertTrue(target["replicated"])

        # Permute only the measured outcome flags; retain every coarse reason
        # label and channel label unchanged. A spurious category-as-cause
        # interpretation would incorrectly survive this permutation.
        flattened = []
        for split in ("discovery", "replication"):
            split_rows = [row for row in observed if row.split == split]
            outcomes = [row.event for row in split_rows]
            random.Random(90210).shuffle(outcomes)
            flattened.extend(
                Observation(cell=row.cell, event=outcome, split=split)
                for row, outcome in zip(split_rows, outcomes)
            )
        permuted = assess_cells("M1", self.cells, flattened, family_size=4)
        target_after = next(
            result for result in permuted
            if result["cell"] == {"reason_category": "complaint", "channel": "phone"}
        )
        self.assertEqual(target_after["status"], "refuted")
        self.assertEqual(target_after["cell"]["reason_category"], "complaint")
        self.assertFalse(target_after["replicated"])

    def test_safe_pack_rejects_small_counts_identifiers_and_free_text(self):
        with self.assertRaises(ValueError):
            validate_safe_pack({"numerator": 9})
        with self.assertRaises(ValueError):
            validate_safe_pack({"customer_id": "C001"})
        with self.assertRaises(ValueError):
            validate_safe_pack({"query_signature": "not-for-public-output"})
        with self.assertRaises(ValueError):
            validate_safe_pack({"question_1_text": "customer words"})
        with self.assertRaises(ValueError):
            validate_safe_pack({"title": "customer@example.com"})
        validate_safe_pack({"numerator": None, "suppression_reason": "k_floor"})

    def test_streaming_accumulator_matches_row_assessment(self):
        rows = self.observations(target_events=400)
        accumulator = MetricAccumulator()
        for row in rows:
            accumulator.add(row.cell, row.event, row.split)
            accumulator.add(row.cell, row.event, "overall")
        self.assertEqual(
            assess_cells("M1", self.cells, rows, family_size=4),
            assess_accumulator("M1", self.cells, accumulator, family_size=4),
        )
        overall = assess_accumulator("M1", self.cells + [{"scope": "overall"}], accumulator, family_size=5)[-1]
        self.assertIsNotNone(overall["numerator"])
        self.assertIsNotNone(overall["denominator"])
        self.assertIsNone(overall["effect"])
        target = next(result for result in assess_accumulator("M1", self.cells, accumulator, family_size=4) if result["cell"] == self.cells[0])
        self.assertEqual(target["snapshot"]["numerator"], 800)
        self.assertEqual(target["snapshot"]["denominator"], 2000)

    def test_e0_discovers_modal_signature_and_replicates_without_emitting_it(self):
        cases = [
            E0Case("secret-signature-a", "discovery") for _ in range(150)
        ] + [E0Case("secret-signature-b", "discovery") for _ in range(50)]
        cases += [
            E0Case("secret-signature-a", "replication") for _ in range(180)
        ] + [E0Case("secret-signature-c", "replication") for _ in range(20)]
        result = assess_e1(cases, family_size=181)
        self.assertEqual(result["status"], "corroborated")
        self.assertTrue(result["replicated"])
        self.assertEqual(result["cell"], {"scope": "overall"})
        self.assertNotIn("secret-signature-a", repr(result))

    def test_e1_full_family_adjustment_prevents_overclaiming_a_modest_decline(self):
        result = assess_e1_counts(150, 200, 130, 200)
        self.assertEqual(result["status"], "uncertain")
        self.assertGreater(result["multiple_testing"]["adjusted_q"], 0.05)


if __name__ == "__main__":
    unittest.main()
