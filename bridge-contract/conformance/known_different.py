"""Known-different cases per target: test name (or `prefix*`) -> (divergence id, justification).

Applied as `xfail(strict=True)`: the case must still FAIL against that target; if the target converges the case XPASSes
and the suite fails, forcing this table to shrink. Measured against platform-sim `bridge_mock` (runtime_profile=
contract_mock) on 2026-10-03; the mock is a PRE-annex-D/A03 simulator and is not the implementation under contract.
`real` has no entries: every case must pass against the real composed runtime."""

from __future__ import annotations

# divergence id -> one-line justification (what the mock does differently from the contract / the real runtime)
DIVERGENCES: dict[str, str] = {
    "MOCK-CODES": "mock error `code` values are bare (credentials_invalid, forbidden, invalid_request, not_found, "
                  "method_not_allowed, alias_unknown) - not the closed `pulso:*` list of ErrorEnvelope",
    "MOCK-AUTH-REASONS": "mock collapses every token failure into one 401 credentials_invalid with no closed "
                         "`details.reason`; header/kid/key-binding/claims/exp/ttl cases are indistinguishable",
    "MOCK-PURPOSE-STATUS": "mock answers a wrong purpose as 403 `forbidden`, not 403 `pulso:auth_denied`",
    "MOCK-TENANT": "mock has no deployment tenant set and no tenant_required rule: it only compares the claim to the "
                   "body tenant, and it requires a tenant claim on /version too",
    "MOCK-JTI-ORDER": "mock consumes the jti BEFORE the purpose check (a rejected token burns its jti); the runtime "
                      "consumes last",
    "MOCK-TRACE": "mock generates its own trace_id and ignores `traceparent`",
    "MOCK-LIMITS": "mock has no 1 MiB body cap (no payload_too_large) and spells input_too_large without the prefix",
    "MOCK-STATUS": "mock uses 400 where the contract uses 422 invalid_request (and 400 for non-object bodies)",
    "MOCK-DTO": "mock CoreTaskInvocation predates annex D/A03: attempt >= 0, input_refs/config_ref/"
                "information_partition/budget_ref, NO logical_key/pulso_run_ref/input_artifact_refs/commitment, "
                "free-form Idempotency-Key, request_digest mandatory; receipt is the mock CoreTaskReceipt "
                "(idempotency_key, bridge_instance_id, runtime_profile ...), not the runtime's state body",
    "MOCK-CATALOG": "mock has no stage->agent catalogue, input-slot check, release-pin reason codes or receipt "
                    "`reason`; it spells release errors bare (release_pin_unavailable)",
    "MOCK-VERSION": "mock /version has runtime_profile=contract_mock, no `doubles` list and its own pin constants",
    "MOCK-CRED": "mock CoreCredentialIssue(Request) carry schema_version/runtime_profile and an ISO `exp`; the "
                 "runtime request has NO schema_version (extra=forbid) and returns an integer `exp`; the mock's "
                 "policy is role-only (no (purpose, role) table)",
    "MOCK-ALIAS": "mock alias reader answers unset aliases with bare alias_unknown and lacks status/source/"
                  "observed_at; request validation of the alias name differs",
    "MOCK-WORKER-SUB": "mock accepts any non-empty `sub` and does not compare the token `job_id` claim with the body",
    "MOCK-GOLDEN": "the goldens are recorded from the runtime: every step the mock answers with its own DTO/codes "
                   "differs (see the other MOCK-* ids); flows the mock cannot run are skipped by capability",
    "MOCK-DRYRUN": "mock dry-run request/closed-DTO and tenant-mismatch handling differ from the runtime's "
                   "(bare codes, 400 for schema errors, no base-release existence check: unknown base is a 200)",
}


def _group(div: str, *names: str) -> dict[str, tuple[str, str]]:
    return {n: (div, DIVERGENCES[div]) for n in names}


MOCK: dict[str, tuple[str, str]] = {}
MOCK |= _group("MOCK-CODES",
               "test_missing_token_is_401_auth_invalid", "test_every_denial_is_a_valid_error_envelope_not_problem_json",
               "test_unknown_path_is_404_not_found_envelope", "test_wrong_method_is_an_envelope_not_core_problem_json",
               "test_jti_replay_is_refused_and_a_fresh_jti_is_accepted")
MOCK |= _group("MOCK-AUTH-REASONS",
               "test_malformed_token_is_401_with_a_closed_reason",
               "test_header_must_be_exactly_alg_kid_typ_with_fixed_algorithm", "test_unknown_kid_is_refused",
               "test_signature_from_another_key_is_refused", "test_kid_is_bound_to_one_issuer_and_audience",
               "test_time_and_required_claims", "test_nan_timestamps_cannot_defeat_the_comparisons")
MOCK |= _group("MOCK-PURPOSE-STATUS", "test_wrong_purpose_is_403_auth_denied")
MOCK |= _group("MOCK-TENANT",
               "test_tenant_claim_is_required_on_every_tenant_route",
               "test_tenant_outside_the_deployment_set_is_403_tenant_mismatch",
               "test_tenant_exempt_routes_accept_a_token_without_tenant")
MOCK |= _group("MOCK-JTI-ORDER", "test_a_rejected_token_does_not_burn_its_jti")
MOCK |= _group("MOCK-TRACE", "test_traceparent_trace_id_is_echoed_in_error_envelopes")
MOCK |= _group("MOCK-LIMITS", "test_body_over_the_cap_is_413_payload_too_large",
               "test_input_over_256_kib_is_413_input_too_large_with_zero_effects")
MOCK |= _group("MOCK-STATUS", "test_non_object_bodies_are_422_invalid_request", "test_unknown_field_is_422_and_names_the_field",
               "test_missing_required_field_is_422", "test_body_tenant_must_equal_the_signed_tenant")
MOCK |= _group("MOCK-DTO",
               "test_invocation_schema_accepts_what_the_builder_makes_and_rejects_drift",
               "test_idempotency_key_must_follow_the_formula", "test_request_digest_must_match_when_present",
               "test_scout_happy_path_returns_a_terminal_ok_receipt_with_a_valid_fact",
               "test_same_key_and_body_replays_the_stored_receipt_without_a_second_run",
               "test_same_key_with_another_body_is_409_digest_conflict",
               "test_read_by_core_run_id_returns_the_same_terminal_receipt",
               "test_read_of_a_foreign_tenants_run_is_404_not_found", "test_read_of_an_unknown_id_is_404_not_found")
MOCK |= _group("MOCK-CATALOG", "test_unknown_stage_is_400_stage_unknown",
               "test_stage_agent_mismatch_is_422_before_any_receipt",
               "test_input_slot_outside_the_stage_catalogue_is_400", "test_commitment_on_a_non_writer_stage_is_422",
               "test_release_pin_failures_are_409_release_pin_unavailable",
               "test_a_pin_failure_is_terminal_and_a_retry_with_the_same_key_does_not_re_run")
MOCK |= _group("MOCK-VERSION", "test_version_reports_the_pinned_core_and_validates")
MOCK |= _group("MOCK-CRED", "test_credential_issue_succeeds_for_policy_rows",
               "test_credential_issue_refuses_everything_outside_the_policy",
               "test_credential_issue_for_another_tenant_is_tenant_mismatch", "test_credential_request_is_closed")
MOCK |= _group("MOCK-ALIAS", "test_alias_read_returns_a_valid_alias_state",
               "test_alias_read_of_an_unknown_agent_is_404_alias_unknown", "test_alias_name_outside_staging_prod_is_422")
MOCK |= _group("MOCK-WORKER-SUB", "test_class_i_subject_must_be_a_worker",
               "test_invoke_token_job_must_equal_the_body_job")
MOCK |= _group("MOCK-GOLDEN", "test_golden_flow")
MOCK |= _group("MOCK-DRYRUN", "test_dry_run_body_tenant_must_equal_the_claim", "test_dry_run_request_is_a_closed_dto",
               "test_dry_run_with_an_unknown_base_release_is_404_base_release_unknown")

KNOWN_DIFFERENT: dict[str, dict[str, tuple[str, str]]] = {"real": {}, "mock": MOCK}

# Capabilities the mock does not have at all: those tests SKIP (see worlds/mock.py caps) rather than xfail.
MOCK_SKIPPED_BY_CAPABILITY = {
    "evaluation": "mock has no /evaluation/* routes (admissions, arms run/by-key/by-id)",
    "writer": "mock has no registry/proposals: no frozen candidate can be seeded",
    "arms": "mock has no arm runner / scenario manifests",
    "control": "mock exposes /_sim/* fault injection instead of the runtime's loopback doubles",
}
