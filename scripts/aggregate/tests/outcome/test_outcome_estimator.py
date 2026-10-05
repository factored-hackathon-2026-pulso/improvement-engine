import unittest
from pathlib import Path

from scripts.aggregate.outcome.outcome_estimator import estimate_outcomes


class OutcomeEstimatorContractTests(unittest.TestCase):
    def fixture(self, treated_post_rate=0.50):
        rows = []
        periods = [
            "2024-07", "2024-08", "2024-09", "2024-10", "2024-11", "2024-12",
            "2025-01", "2025-02", "2025-03", "2025-04", "2025-05", "2025-06", "2025-07",
        ]
        for half in ("discovery", "holdout"):
            for period in periods:
                post = period in {"2025-02", "2025-03", "2025-04"}
                for reason, rate in (
                    ("Queja", treated_post_rate if post else 0.50),
                    ("Comercial", 0.50),
                ):
                    denominator = 3000
                    rows.append({
                        "metric": "M1", "dims": {"reason_category": reason, "channel": "Phone"},
                        "half": half, "period": period,
                        "numerator": round(rate * denominator), "denominator": denominator,
                    })
        return rows

    def test_report_names_bounded_sibling_cells_used_as_controls(self):
        report = estimate_outcomes(self.fixture(), "2025-01")
        queja = next(row for row in report["cells"] if row["dims"]["reason_category"] == "complaint")
        self.assertEqual(queja["control_dimension"], "reason_category")
        self.assertEqual(queja["control_siblings"], [{"channel": "phone", "reason_category": "unclassified"}])

    def _write_synthetic_cli_dataset(self, root):
        import csv
        from scripts.aggregate.bank_cells import split_half

        periods = [f"{year}-{month:02d}" for year, month in (
            (2024, 7), (2024, 8), (2024, 9), (2024, 10), (2024, 11), (2024, 12),
            (2025, 1), (2025, 2), (2025, 3), (2025, 4), (2025, 5), (2025, 6), (2025, 7),
        )]
        root.mkdir(parents=True)
        calls_dir = root / "call_center_interactions"
        complaints_dir = root / "complaints"
        surveys_dir = root / "satisfaction_surveys"
        calls_dir.mkdir()
        complaints_dir.mkdir()
        surveys_dir.mkdir()

        call_path = calls_dir / "synthetic.csv"
        with call_path.open("w", newline="", encoding="utf-8") as stream:
            writer = csv.DictWriter(stream, fieldnames=[
                "interaction_id", "customer_id", "interaction_date", "reason_category",
                "channel", "was_resolved", "duration_seconds",
            ])
            writer.writeheader()
            for period in periods:
                for half in ("discovery", "holdout"):
                    for reason in ("Queja", "Comercial"):
                        for index in range(1000):
                            token = f"{period}-{half}-{reason}-{index}"
                            customer_id = next(
                                f"synthetic-{token}-{salt}"
                                for salt in range(100)
                                if split_half(f"synthetic-{token}-{salt}") == half
                            )
                            interaction_id = f"i-{token}"
                            writer.writerow({
                                "interaction_id": interaction_id,
                                "customer_id": customer_id,
                                "interaction_date": f"{period}-15",
                                "reason_category": reason,
                                "channel": "App",
                                "was_resolved": "True" if index % 2 == 0 else "False",
                                "duration_seconds": "3600",
                            })

        with (complaints_dir / "synthetic.csv").open("w", newline="", encoding="utf-8") as stream:
            writer = csv.DictWriter(stream, fieldnames=[
                "complaint_id", "customer_id", "creation_date", "category", "status", "sla_breached",
            ])
            writer.writeheader()
            for period in periods:
                for half in ("discovery", "holdout"):
                    for category in ("Fees", "Service"):
                        for index in range(1000):
                            token = f"{period}-{half}-{category}-{index}"
                            customer_id = next(
                                f"synthetic-{token}-{salt}"
                                for salt in range(100)
                                if split_half(f"synthetic-{token}-{salt}") == half
                            )
                            writer.writerow({
                                "complaint_id": f"p-{token}", "customer_id": customer_id,
                                "creation_date": f"{period}-15", "category": category,
                                "status": "Open" if index % 2 == 0 else "Closed",
                                "sla_breached": "True" if index % 10 == 0 else "False",
                            })

        with (surveys_dir / "synthetic.csv").open("w", newline="", encoding="utf-8") as stream:
            writer = csv.DictWriter(stream, fieldnames=[
                "interaction_id", "customer_id", "survey_type", "main_score", "send_channel",
            ])
            writer.writeheader()
            for period in periods:
                for half in ("discovery", "holdout"):
                    for reason in ("Queja", "Comercial"):
                        for index in range(1000):
                            token = f"{period}-{half}-{reason}-{index}"
                            customer_id = next(
                                f"synthetic-{token}-{salt}"
                                for salt in range(100)
                                if split_half(f"synthetic-{token}-{salt}") == half
                            )
                            writer.writerow({
                                "interaction_id": f"i-{token}", "customer_id": customer_id,
                                "survey_type": "CSAT", "main_score": "1" if index % 5 == 0 else "5",
                                "send_channel": "App",
                            })

    def test_documented_and_legacy_cli_entrypoints_emit_the_same_v3_contract(self):
        import json
        import subprocess
        import sys
        import tempfile

        repo_root = Path(__file__).resolve().parents[4]
        with tempfile.TemporaryDirectory() as temporary:
            temp_root = Path(temporary)
            data_root = temp_root / "synthetic-data"
            self._write_synthetic_cli_dataset(data_root)
            report_bytes = []
            for module, filename in (
                ("scripts.aggregate.outcome.outcome_estimator", "documented.json"),
                ("scripts.aggregate.outcome_estimator", "legacy.json"),
            ):
                output = temp_root / filename
                completed = subprocess.run(
                    [sys.executable, "-m", module, "--data-root", str(data_root), "--out", str(output)],
                    cwd=repo_root, text=True, capture_output=True, timeout=90, check=False,
                )
                self.assertEqual(completed.returncode, 0, completed.stderr)
                serialized = output.read_bytes()
                report = json.loads(serialized)
                self.assertEqual(report["protocol"], "outcome-discovery-v3", module)
                self.assertIn("M10", {cell["metric"] for cell in report["analysis"]["cells"]})
                self.assertIn("empirical_candidate_window_rate", report["placebo"])
                self.assertIn("screen_bound_met", report["placebo"])
                self.assertIn("injected_shift_screen", report)
                self.assertNotIn(b"mde_80_pp", serialized)
                self.assertNotIn(b"synthetic-", serialized)
                self.assertNotIn(b"i-2024-", serialized)
                report_bytes.append(serialized)
            self.assertEqual(report_bytes[0], report_bytes[1])

    def test_legacy_import_is_the_canonical_v3_module(self):
        import scripts.aggregate.outcome_estimator as legacy
        import scripts.aggregate.outcome.outcome_estimator as canonical

        self.assertIs(legacy, canonical)

    def test_estimate_is_descriptive_and_does_not_publish_or_gate_on_mde(self):
        from scripts.aggregate.outcome.outcome_estimator import validate_placebos_and_sensitivity

        rows = self.fixture(treated_post_rate=0.40)
        result = estimate_outcomes(rows, release_period="2025-01")
        target = next(cell for cell in result["cells"] if cell["dims"]["reason_category"] == "complaint")
        self.assertEqual(target["status"], "improved")
        self.assertNotIn("mde_80_pp", target)
        self.assertIn("not_customer_cluster_adjusted", target["uncertainty_method"])
        report = validate_placebos_and_sensitivity(rows, placebo_draws=20)
        self.assertEqual(report["statistical_power_or_mde"], "not_estimated_from_aggregate_cells")
        self.assertNotIn("mde_80_pp", repr(report))

    def test_minimum_support_still_fails_closed(self):
        rows = self.fixture()
        for row in rows:
            row["denominator"] = 100
            row["numerator"] = 50
        result = estimate_outcomes(rows, release_period="2025-01")
        self.assertTrue(all(cell["status"] == "inconclusive" for cell in result["cells"]))
        self.assertTrue(all(cell["reason"] == "underpowered_minimum_support" for cell in result["cells"]))

    def test_cli_builds_only_the_preregistered_t1_source_tables(self):
        import sys
        import tempfile
        from unittest.mock import patch
        from scripts.aggregate.outcome import outcome_estimator

        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            data_root = root / "data"
            data_root.mkdir()
            output = root / "report.json"
            expected_tables = (
                "call_center_interactions", "complaints", "satisfaction_surveys",
            )
            with (
                patch.object(sys, "argv", ["outcome_estimator", "--data-root", str(data_root), "--out", str(output)]),
                patch("scripts.aggregate.bank_cells.build", return_value=(self.fixture(), {"k": 10, "metrics": {"M1": 1}})) as build,
                patch.object(outcome_estimator, "validate_placebos_and_sensitivity", return_value={"protocol": "test"}),
            ):
                self.assertEqual(outcome_estimator.main(), 0)
            build.assert_called_once_with(data_root, k=10, tables=expected_tables)

    def test_placebo_rate_and_threshold_are_withheld_for_sub_k_numerator(self):
        from unittest.mock import patch
        from scripts.aggregate.outcome import outcome_estimator

        rows = self.fixture()
        boundaries = ["2024-01"] + [f"2024-{month:02d}" for month in range(2, 13)] + [
            "2025-01", "2025-02", "2025-03", "2025-04", "2025-05", "2025-06", "2025-07",
        ]

        def fake_estimate(_rows, release_period):
            status = "improved" if release_period == "2024-01" else "inconclusive"
            return {"cells": [{"metric": "M1", "dims": {}, "status": status, "reason": None}]}

        with (
            patch.object(outcome_estimator, "_release_boundaries", return_value=boundaries),
            patch.object(outcome_estimator, "estimate_outcomes", side_effect=fake_estimate),
            patch.object(outcome_estimator, "_injected_shift_screen", return_value={"by_support_bin": []}),
        ):
            report = outcome_estimator.validate_placebos_and_sensitivity(rows, placebo_draws=len(boundaries))

        placebo = report["placebo"]
        self.assertIsNone(placebo["windows_with_candidate_signal"])
        self.assertIsNone(placebo["empirical_candidate_window_rate"])
        self.assertIsNone(placebo["screen_bound_met"])

    def test_placebo_rate_and_threshold_are_withheld_for_sub_k_complement(self):
        from unittest.mock import patch
        from scripts.aggregate.outcome import outcome_estimator

        rows = self.fixture()
        boundaries = [f"2024-{month:02d}" for month in range(1, 13)] + [
            f"2025-{month:02d}" for month in range(1, 9)
        ]

        def fake_estimate(_rows, release_period):
            is_signal = boundaries.index(release_period) < 11
            status = "improved" if is_signal else "inconclusive"
            return {"cells": [{"metric": "M1", "dims": {}, "status": status, "reason": None}]}

        with (
            patch.object(outcome_estimator, "_release_boundaries", return_value=boundaries),
            patch.object(outcome_estimator, "estimate_outcomes", side_effect=fake_estimate),
            patch.object(outcome_estimator, "_injected_shift_screen", return_value={"by_support_bin": []}),
        ):
            report = outcome_estimator.validate_placebos_and_sensitivity(rows, placebo_draws=len(boundaries))

        placebo = report["placebo"]
        self.assertTrue(placebo["summary_suppressed"])
        self.assertIsNone(placebo["windows_with_candidate_signal"])
        self.assertIsNone(placebo["empirical_candidate_window_rate"])
        self.assertIsNone(placebo["screen_bound_met"])

    def test_cli_rejects_output_inside_repository(self):
        import sys
        from unittest.mock import patch
        from scripts.aggregate.outcome import outcome_estimator

        repo_root = Path(__file__).resolve().parents[4]
        output = repo_root / "outcome-report-test.json"
        with (
            patch.object(sys, "argv", ["outcome_estimator", "--data-root", str(repo_root), "--out", str(output)]),
            patch("scripts.aggregate.bank_cells.build", return_value=(self.fixture(), {"k": 10, "metrics": {}})),
            patch.object(outcome_estimator, "validate_placebos_and_sensitivity", return_value={}),
            patch.object(Path, "mkdir"),
            patch.object(Path, "write_text"),
        ):
            with self.assertRaisesRegex(ValueError, "outside the repository"):
                outcome_estimator.main()

    def test_cli_rejects_output_inside_data_root_before_reading_or_creating(self):
        import sys
        import tempfile
        from unittest.mock import patch
        from scripts.aggregate.outcome import outcome_estimator

        with tempfile.TemporaryDirectory() as temporary:
            data_root = Path(temporary) / "sources"
            data_root.mkdir()
            output = data_root / "nested" / "report.json"
            with (
                patch.object(sys, "argv", ["outcome_estimator", "--data-root", str(data_root), "--out", str(output)]),
                patch("scripts.aggregate.bank_cells.build") as build,
            ):
                with self.assertRaisesRegex(ValueError, "outside the data root"):
                    outcome_estimator.main()
                build.assert_not_called()
            self.assertFalse(output.parent.exists())
            self.assertFalse(output.exists())

    def test_cli_refuses_existing_output_before_reading_and_preserves_bytes(self):
        import sys
        import tempfile
        from unittest.mock import patch
        from scripts.aggregate.outcome import outcome_estimator

        with tempfile.TemporaryDirectory() as temporary:
            data_root = Path(temporary) / "sources"
            destination = Path(temporary) / "reports"
            data_root.mkdir()
            destination.mkdir()
            output = destination / "existing.json"
            original = b"existing output stays untouched\n"
            output.write_bytes(original)
            with (
                patch.object(sys, "argv", ["outcome_estimator", "--data-root", str(data_root), "--out", str(output)]),
                patch("scripts.aggregate.bank_cells.build") as build,
            ):
                with self.assertRaisesRegex(ValueError, "already exists"):
                    outcome_estimator.main()
                build.assert_not_called()
            self.assertEqual(output.read_bytes(), original)

    def test_cli_does_not_clobber_output_created_after_preflight_check(self):
        import sys
        import tempfile
        from unittest.mock import patch
        from scripts.aggregate.outcome import outcome_estimator

        with tempfile.TemporaryDirectory() as temporary:
            data_root = Path(temporary) / "sources"
            destination = Path(temporary) / "reports"
            data_root.mkdir()
            destination.mkdir()
            output = destination / "racing.json"
            concurrent_bytes = b"created concurrently; preserve me\n"

            def build_then_race(_data_root, *, k, tables):
                self.assertEqual(k, 10)
                self.assertEqual(tables, outcome_estimator.T1_SOURCE_TABLES)
                output.write_bytes(concurrent_bytes)
                return self.fixture(), {"k": 10, "metrics": {}}

            with (
                patch.object(sys, "argv", ["outcome_estimator", "--data-root", str(data_root), "--out", str(output)]),
                patch("scripts.aggregate.bank_cells.build", side_effect=build_then_race),
                patch.object(outcome_estimator, "validate_placebos_and_sensitivity", return_value={"protocol": "test"}),
            ):
                with self.assertRaisesRegex(ValueError, "already exists"):
                    outcome_estimator.main()
            self.assertEqual(output.read_bytes(), concurrent_bytes)

    def test_rejects_rows_that_are_not_exact_bank_cell_schema(self):
        rows = self.fixture()
        rows[0]["customer_id"] = "never-present-in-this-contract"
        with self.assertRaisesRegex(ValueError, "bank_cells NDJSON schema"):
            estimate_outcomes(rows, release_period="2025-03")

    def test_rejects_any_published_count_that_breaks_k_rule(self):
        rows = self.fixture()
        rows[0]["numerator"] = 5
        with self.assertRaisesRegex(ValueError, "k=10"):
            estimate_outcomes(rows, release_period="2025-03")

    def test_missing_sibling_month_fails_closed(self):
        rows = [
            row for row in self.fixture()
            if not (row["half"] == "holdout" and row["dims"]["reason_category"] == "Comercial"
                    and row["period"] == "2025-03")
        ]
        result = estimate_outcomes(rows, release_period="2025-01")
        target = next(row for row in result["cells"] if row["dims"]["reason_category"] == "complaint")
        self.assertEqual(target["status"], "inconclusive")
        self.assertEqual(target["reason"], "incomplete_published_sibling_control")

    def test_placebo_screen_is_deterministic_and_does_not_claim_power(self):
        from scripts.aggregate.outcome.outcome_estimator import validate_placebos_and_sensitivity

        rows = self.fixture()
        first = validate_placebos_and_sensitivity(rows, placebo_draws=20)
        second = validate_placebos_and_sensitivity(rows, placebo_draws=20)
        self.assertEqual(first, second)
        self.assertEqual(first["placebo"]["sampling"], "unique_release_windows_without_replacement")
        self.assertEqual(
            first["placebo"]["interpretation"],
            "finite_horizon_empirical_placebo_signal_frequency_not_calibrated_type_i_error",
        )
        if first["placebo"]["summary_suppressed"]:
            self.assertIsNone(first["placebo"]["empirical_candidate_window_rate"])
            self.assertIsNone(first["placebo"]["screen_bound_met"])
        else:
            self.assertEqual(
                first["placebo"]["empirical_candidate_window_rate"],
                round(
                    first["placebo"]["windows_with_candidate_signal"]
                    / first["placebo"]["evaluated_windows"],
                    6,
                ),
            )
        self.assertEqual(first["placebo"]["screen_bound_fraction"], 0.05)
        self.assertEqual(first["statistical_power_or_mde"], "not_estimated_from_aggregate_cells")
        self.assertNotIn("acceptance_passed", first["placebo"])
        self.assertTrue(all("n_pre" not in row and "n_post" not in row for row in first["underpowered_cells"] or []))

    def test_injected_shift_screen_reports_algorithm_response_not_statistical_power(self):
        from scripts.aggregate.outcome.outcome_estimator import validate_placebos_and_sensitivity

        report = validate_placebos_and_sensitivity(self.fixture(), placebo_draws=5)
        self.assertEqual(report["protocol"], "outcome-discovery-v3")
        screen = report["injected_shift_screen"]
        self.assertEqual(screen["boundary"], "2025-01")
        self.assertEqual(screen["injections_pp"], [2, 5, 10])
        self.assertEqual(
            screen["interpretation"],
            "deterministic_aggregate_shift_response_not_statistical_power_or_causal_effect",
        )
        self.assertEqual(
            {row["support_bin"] for row in screen["by_support_bin"]},
            {"<500", "500-999", "1000-1999", ">=2000"},
        )
        self.assertTrue(all("detection_rate_by_injection" in row for row in screen["by_support_bin"]))
        self.assertNotIn("statistical_power", screen)

    def test_injected_shift_reduces_exact_aggregate_total_without_mutating_source(self):
        from scripts.aggregate.outcome.outcome_estimator import _cell_key, _inject_reduction

        rows = self.fixture()
        target = next(row for row in rows if row["dims"]["reason_category"] == "Queja")
        key = _cell_key(target)
        before = [dict(row, dims=dict(row["dims"])) for row in rows]
        shifted = _inject_reduction(rows, key, shift_pp=2, boundary="2025-01", window_months=3)
        for half in ("discovery", "holdout"):
            original_post = sum(
                row["numerator"] for row in before
                if _cell_key(row) == key and row["half"] == half and row["period"] in {"2025-02", "2025-03", "2025-04"}
            )
            changed_post = sum(
                row["numerator"] for row in shifted
                if _cell_key(row) == key and row["half"] == half and row["period"] in {"2025-02", "2025-03", "2025-04"}
            )
            self.assertEqual(original_post - changed_post, 180)
        self.assertEqual(rows, before)

    def test_injected_shift_omits_rows_that_would_break_k10(self):
        from scripts.aggregate.outcome.outcome_estimator import _cell_key, _inject_reduction

        rows = self.fixture()
        target = next(row for row in rows if row["dims"]["reason_category"] == "Queja")
        key = _cell_key(target)
        for row in rows:
            if row["dims"]["reason_category"] == "Queja" and row["period"] == "2025-02":
                row["numerator"] = 65
        shifted = _inject_reduction(rows, key, shift_pp=2, boundary="2025-01", window_months=3)
        for half in ("discovery", "holdout"):
            self.assertFalse(any(
                _cell_key(row) == key and row["half"] == half and row["period"] == "2025-02"
                for row in shifted
            ))
        self.assertTrue(all(
            row["numerator"] == 0 or row["numerator"] >= 10
            for row in shifted
        ))
        self.assertTrue(all(
            row["denominator"] - row["numerator"] == 0 or row["denominator"] - row["numerator"] >= 10
            for row in shifted
        ))

    def test_injected_shift_suppresses_support_bins_with_fewer_than_ten_cells(self):
        from scripts.aggregate.outcome.outcome_estimator import _injected_shift_screen

        report = _injected_shift_screen(self.fixture(), boundary="2025-01", window_months=3)
        high_support = next(row for row in report["by_support_bin"] if row["support_bin"] == ">=2000")
        self.assertIsNone(high_support["registered_cells"])
        self.assertEqual(set(high_support["detection_rate_by_injection"].values()), {None})
        self.assertIsNone(high_support["algorithmic_80pct_shift_threshold_pp"])

    def test_injected_shift_keeps_missing_targets_in_registered_cell_denominator(self):
        from unittest.mock import patch
        import scripts.aggregate.outcome.outcome_estimator as estimator

        rows = []
        for row in self.fixture():
            rows.append(row)
            if row["metric"] == "M1":
                for channel in ("Web", "Chat", "Email", "Branch", "App"):
                    rows.append(dict(row, dims={**row["dims"], "channel": channel}))
        target_key = estimator._cell_key(estimator._validate_rows(rows)[0])

        def inject(data, key, **_kwargs):
            return None if key == target_key else data

        def improved_for_each_available_target(_rows, _boundary, *, window_months):
            del window_months
            return {"cells": [{
                "metric": row["metric"],
                "dims": row["dims"],
                "status": "improved",
            } for row in estimator._validate_rows(_rows)]}

        with (
            patch.object(estimator, "_inject_reduction", side_effect=inject),
            patch.object(estimator, "estimate_outcomes", side_effect=improved_for_each_available_target),
        ):
            report = estimator._injected_shift_screen(rows, boundary="2025-01", window_months=3)

        high_support = next(row for row in report["by_support_bin"] if row["support_bin"] == ">=2000")
        self.assertEqual(high_support["registered_cells"], 12)
        expected_rate = round(11 / 12, 6)
        self.assertEqual(high_support["detection_rate_by_injection"], {"2": expected_rate, "5": expected_rate, "10": expected_rate})
        self.assertEqual(high_support["algorithmic_80pct_shift_threshold_pp"], 2)

    def test_placebo_estimates_each_distinct_window_once(self):
        from unittest.mock import patch
        import scripts.aggregate.outcome.outcome_estimator as estimator

        rows = self.fixture()
        boundaries = estimator._release_boundaries(rows, estimator.WINDOW_MONTHS)
        original = estimator.estimate_outcomes
        with (
            patch.object(estimator, "estimate_outcomes", wraps=original) as estimate,
            patch.object(estimator, "_injected_shift_screen", return_value={}),
        ):
            estimator.validate_placebos_and_sensitivity(rows, placebo_draws=100)
        self.assertEqual(estimate.call_count, len(boundaries) + 1)

    def test_placebo_sampling_selects_distinct_boundaries_without_replacement(self):
        from unittest.mock import patch
        import scripts.aggregate.outcome.outcome_estimator as estimator

        rows = self.fixture()
        boundaries = estimator._release_boundaries(rows, estimator.WINDOW_MONTHS)
        self.assertGreater(len(boundaries), 1)

        class DeterministicSampler:
            def __init__(self, seed):
                self.seed = seed

            def sample(self, population, count):
                return list(population[:count])

            def choice(self, population):
                raise AssertionError("placebo boundaries must not be sampled with replacement")

        with (
            patch.object(estimator.random, "Random", DeterministicSampler),
            patch.object(estimator, "_injected_shift_screen", return_value={}),
        ):
            report = estimator.validate_placebos_and_sensitivity(rows, placebo_draws=len(boundaries))
        self.assertEqual(report["placebo"]["evaluated_windows"], len(boundaries))

    def test_source_free_text_is_mapped_before_report_serialization(self):
        from scripts.aggregate.outcome.outcome_estimator import _validate_rows

        rows = self.fixture()
        rows[0]["dims"]["reason_category"] = "CUST-123456789 Maria Perez"
        safe = _validate_rows(rows)
        self.assertNotIn("CUST-123456789 Maria Perez", repr(safe))
        self.assertIn("unclassified", {row["dims"]["reason_category"] for row in safe})

    def test_unknown_dimension_keys_and_metrics_fail_closed(self):
        from scripts.aggregate.outcome.outcome_estimator import _validate_rows

        row = self.fixture()[0]
        row["dims"]["customer_id"] = "synthetic-customer-id"
        with self.assertRaisesRegex(ValueError, "safe metric vocabulary"):
            _validate_rows([row])

        row = self.fixture()[0]
        row["metric"] = "synthetic-free-text-metric"
        with self.assertRaisesRegex(ValueError, "invalid metric"):
            _validate_rows([row])

    def test_pqr_categories_remain_distinct_and_unknown_category_fails_closed(self):
        from scripts.aggregate.outcome.outcome_estimator import _validate_rows

        categories = ("Transactions", "Fees", "Technical", "Branch", "Service")
        rows = [
            {
                "metric": "M4", "dims": {"category": category},
                "half": "discovery", "period": f"2025-{index:02d}",
                "numerator": 20, "denominator": 100,
            }
            for index, category in enumerate(categories, start=1)
        ]
        safe = _validate_rows(rows)
        self.assertEqual({row["dims"]["category"] for row in safe}, {
            "transactions", "fees", "technical", "branch", "service",
        })
        rows[0]["dims"]["category"] = "Private free text category"
        with self.assertRaisesRegex(ValueError, "unrecognized PQR category"):
            _validate_rows(rows[:1])

    def test_duplicate_source_cells_reject_but_aliases_merge_and_reapply_k(self):
        from scripts.aggregate.outcome.outcome_estimator import _validate_rows

        row = self.fixture()[0]
        with self.assertRaisesRegex(ValueError, "duplicates a source cell/month"):
            _validate_rows([row, dict(row, dims=dict(row["dims"]))])
        alias = dict(row, dims={**row["dims"], "reason_category": "Reclamo"})
        merged = _validate_rows([row, alias])
        self.assertEqual(len(merged), 1)
        self.assertEqual(merged[0]["dims"]["reason_category"], "complaint")
        self.assertEqual(merged[0]["denominator"], row["denominator"] * 2)

    def test_configuration_rejects_invalid_floats_and_non_integer_windows(self):
        rows = self.fixture()
        for kwargs in (
            {"materiality": float("nan")}, {"materiality": -0.1},
            {"window_months": 1.5}, {"min_window_n": True}, {"alpha": float("nan")},
        ):
            with self.subTest(kwargs=kwargs), self.assertRaisesRegex(ValueError, "invalid estimator configuration"):
                estimate_outcomes(rows, "2025-01", **kwargs)


if __name__ == "__main__":
    unittest.main()
