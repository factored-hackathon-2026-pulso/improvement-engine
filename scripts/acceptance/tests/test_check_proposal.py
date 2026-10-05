"""Recorded HTTP contract tests for the black-box proposal acceptance check."""

from __future__ import annotations

import unittest
import json
import io
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from threading import Thread
from unittest.mock import Mock, patch
from urllib.error import HTTPError, URLError
from urllib.request import Request

from scripts.acceptance.check_proposal import check_acceptance, get_json, main

FIXTURES = Path(__file__).parents[1] / "fixtures"


def recorded(name: str) -> dict:
    return json.loads((FIXTURES / name).read_text(encoding="utf-8"))


def proposal() -> dict:
    return recorded("proposal.json")


def history() -> dict:
    return recorded("history.json")


def quota(*, used: int = 1, limit: int = 10, rejected: bool = True) -> dict:
    value = recorded("quota.json")
    value.update(used_24h=used, limit=limit, within_limit_accepted=True,
                 over_limit_rejected=rejected)
    return value


class ProposalAcceptanceTests(unittest.TestCase):
    def test_agent_core_proposal_detail_envelope_keeps_sibling_changes_and_pii_scan(self) -> None:
        import copy

        flat = proposal()
        detail = {
            "proposal": {key: value for key, value in flat.items() if key != "changes"},
            "changes": copy.deepcopy(flat["changes"]),
            "last_eval": None,
            "review": None,
        }
        result = check_acceptance(detail)
        self.assertNotIn("proposal_changes_empty", result.failures)
        self.assertNotIn("dossier_docs_missing", result.failures)
        self.assertNotIn("dossier_sections_missing", result.failures)

        detail["changes"][0]["content"] = {"prompt": "Never echo pii:customer-token"}
        self.assertIn("pii_token_detected", check_acceptance(detail).failures)

    def test_recorded_http_response_checks_dossier_but_does_not_attest_history_or_quota(self) -> None:
        result = check_acceptance(proposal(), history=history(), quota=quota())
        self.assertEqual(result.failures, ())
        self.assertEqual(
            set(result.not_exercised),
            {"lifecycle_history_provenance_and_completeness", "quota_scope_and_enforcement_provenance"},
        )
        self.assertEqual(result.exit_code, 2)

    def test_empty_or_unbound_observation_objects_cannot_turn_acceptance_green(self) -> None:
        for unbound_history in ({"actions": []}, {"actions": [{}]}):
            with self.subTest(history=unbound_history):
                result = check_acceptance(proposal(), history=unbound_history, quota=quota())
                self.assertEqual(result.exit_code, 2)
                self.assertIn("lifecycle_history_provenance_and_completeness", result.not_exercised)
                self.assertIn("quota_scope_and_enforcement_provenance", result.not_exercised)

    def test_draft_must_be_auto_detect_and_contain_at_least_one_change(self) -> None:
        for field, value in (("origin", "manual"), ("state", "approved"), ("changes", [])):
            bad = proposal()
            bad[field] = value
            with self.subTest(field=field):
                self.assertTrue(check_acceptance(bad, history=history(), quota=quota()).failures)

    def test_proposal_id_must_match_requested_detail_resource(self) -> None:
        result = check_acceptance(proposal(), expected_proposal_id="different-proposal")
        self.assertIn("proposal_id_mismatch", result.failures)

    def test_dossier_requires_every_explanation_section_and_nonempty_rationale(self) -> None:
        bad = proposal()
        bad["changes"][0]["docs"]["description"] = "Problema observado; evidencia parcial."
        bad["changes"][0]["docs"]["rationale"] = ""
        result = check_acceptance(bad, history=history(), quota=quota())
        self.assertIn("dossier_sections_missing", result.failures)
        self.assertIn("rationale_missing", result.failures)

    def test_dossier_rejects_empty_section_bodies_and_evidence_without_baseline(self) -> None:
        bad = proposal()
        description = bad["changes"][0]["docs"]["description"]
        bad["changes"][0]["docs"]["description"] = description.replace(
            "Evidencia y comparación: en el snapshot dataset auditado, 66,000/117,021 (56.4%) vs 16.6% para los demás motivos.",
            "Evidencia y comparación: sin cifras.",
        ).replace("Riesgo: demora.", "Riesgo: .")
        result = check_acceptance(bad)
        self.assertIn("dossier_sections_missing", result.failures)
        self.assertIn("dossier_evidence_incomplete", result.failures)

    def test_each_changed_artifact_description_must_carry_its_own_dossier_sections(self) -> None:
        import copy

        multi_change = proposal()
        second = copy.deepcopy(multi_change["changes"][0])
        second["docs"]["description"] = "Change to an existing tool link."
        multi_change["changes"].append(second)
        result = check_acceptance(multi_change, history=history(), quota=quota())
        self.assertIn("dossier_sections_missing", result.failures)

    def test_pii_tokens_in_dossier_are_rejected(self) -> None:
        bad = proposal()
        bad["changes"][0]["docs"]["description"] += " pii:customer-123"
        self.assertIn("pii_token_detected", check_acceptance(
            bad, history=history(), quota=quota()
        ).failures)

    def test_pii_tokens_in_artifact_content_are_rejected(self) -> None:
        bad = proposal()
        bad["changes"][0]["content"] = {"prompt": "No divulgar <PII_ACCOUNT>"}
        self.assertIn("pii_token_detected", check_acceptance(
            bad, history=history(), quota=quota()
        ).failures)

    def test_email_and_phone_canaries_in_artifact_content_are_rejected(self) -> None:
        for value in ("test.person@example.invalid", "+1 (415) 555-0134"):
            bad = proposal()
            bad["changes"][0]["content"] = {"prompt": f"Contact {value}"}
            with self.subTest(value=value):
                self.assertIn("pii_token_detected", check_acceptance(
                    bad, history=history(), quota=quota()
                ).failures)

    def test_obvious_pii_canaries_in_proposal_level_text_are_rejected(self) -> None:
        for field in ("title", "rationale", "changelog"):
            bad = proposal()
            bad[field] = "Contact test.person@example.invalid"
            with self.subTest(field=field):
                self.assertIn("pii_token_detected", check_acceptance(
                    bad, history=history(), quota=quota()
                ).failures)

    def test_artifact_content_canaries_are_scanned_even_under_metadata_named_fields(self) -> None:
        for field in ("customer_id", "version"):
            bad = proposal()
            bad["changes"][0]["content"] = {field: "test.person@example.invalid"}
            with self.subTest(field=field):
                self.assertIn("pii_token_detected", check_acceptance(
                    bad, history=history(), quota=quota()
                ).failures)

    def test_iso_date_is_not_misclassified_as_a_phone(self) -> None:
        bad = proposal()
        bad["title"] = "Snapshot from 2026-10-05"
        self.assertNotIn("pii_token_detected", check_acceptance(
            bad, history=history(), quota=quota()
        ).failures)

    def test_engine_lifecycle_mutations_are_rejected(self) -> None:
        recorded = history()
        recorded["actions"].append({"action": "proposal_published", "actor": "engine"})
        self.assertIn("engine_lifecycle_mutation", check_acceptance(
            proposal(), history=recorded, quota=quota()
        ).failures)

    def test_quota_boundary_requires_within_limit_and_over_limit_rejection(self) -> None:
        at_limit = check_acceptance(
            proposal(), history=history(), quota=quota(used=10)
        )
        self.assertEqual(at_limit.failures, ())
        self.assertIn("quota_exceeded_without_rejection", check_acceptance(
            proposal(), history=history(), quota=quota(used=11)
        ).failures)
        self.assertIn("quota_rejection_not_observed", check_acceptance(
            proposal(), history=history(), quota=quota(rejected=False)
        ).failures)
        rejected_within = quota()
        rejected_within["within_limit_accepted"] = False
        self.assertIn("quota_within_limit_not_accepted", check_acceptance(
            proposal(), history=history(), quota=rejected_within
        ).failures)

    def test_missing_http_observations_are_reported_not_exercised_not_as_pass(self) -> None:
        result = check_acceptance(proposal())
        self.assertEqual(set(result.not_exercised), {"lifecycle_history", "quota_boundary"})
        self.assertNotEqual(result.exit_code, 0)

    def test_http_client_is_get_only_and_rejects_non_loopback_before_sending_token(self) -> None:
        with patch("scripts.acceptance.check_proposal.build_opener") as build_opener:
            with self.assertRaises(ValueError):
                get_json("https://example.com/proposals/1")
            build_opener.assert_not_called()

    def test_http_client_builds_read_only_loopback_request(self) -> None:
        class Response:
            def __enter__(self):
                return self

            def __exit__(self, *_args):
                return False

            def read(self, _size=-1):
                return b'{"ok":true}'

        opener = Mock()
        opener.open.return_value = Response()
        with patch("scripts.acceptance.check_proposal.build_opener", return_value=opener):
            self.assertEqual(get_json("http://127.0.0.1:8001/proposals/1"), {"ok": True})
        request = opener.open.call_args.args[0]
        self.assertIsInstance(request, Request)
        self.assertEqual(request.get_method(), "GET")

    def test_http_client_rejects_oversized_json_body(self) -> None:
        class Response:
            def __enter__(self):
                return self

            def __exit__(self, *_args):
                return False

            def read(self, size=-1):
                self.requested_size = size
                return b"x" * (size + 1)

        opener = Mock()
        opener.open.return_value = Response()
        with patch("scripts.acceptance.check_proposal.build_opener", return_value=opener):
            with self.assertRaisesRegex(ValueError, "response exceeds size limit"):
                get_json("http://127.0.0.1:8001/proposals/1")

    def test_cli_reports_stack_down_as_not_exercised_exit_two(self) -> None:
        stderr = io.StringIO()
        with patch("scripts.acceptance.check_proposal.get_json", side_effect=URLError("offline")), \
                patch("scripts.acceptance.check_proposal.sys.stderr", stderr):
            code = main(["--proposal-url", "http://127.0.0.1:8001/v1/registry/proposals/test"])
        self.assertEqual(code, 2)
        self.assertIn("acceptance blocked", stderr.getvalue())

    def test_http_client_rejects_redirect_without_following_it(self) -> None:
        requests: list[str] = []

        class Handler(BaseHTTPRequestHandler):
            def do_GET(self) -> None:
                requests.append(self.path)
                self.send_response(302)
                self.send_header("Location", "/redirect-target")
                self.end_headers()

            def log_message(self, *_args) -> None:
                pass

        server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        thread = Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            url = f"http://127.0.0.1:{server.server_port}/redirect-source"
            with self.assertRaises(HTTPError):
                get_json(url)
            self.assertEqual(requests, ["/redirect-source"])
        finally:
            server.shutdown()
            server.server_close()
            thread.join(timeout=2)


if __name__ == "__main__":
    unittest.main()
