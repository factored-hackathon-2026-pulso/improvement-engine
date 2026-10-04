# claude-0023: e2e-core stand-in aligned to Annex D (PR #86, ADR 0011)

Branch `claude/r4-pin-c814c2b`. Stand-in only; no core-bridge runtime change, no image rebuild (image
`localhost/pulso-core-runtime:c814c2b-d522a4f`).

## RED (live, 13 failed / 31 passed / 1 skipped), exact reasons
- `test_01 ...admission_is_created_and_a_replay...`: admission POST 422 `pulso:invalid_request` (the body carried an
  offset `deadline` (`+00:00`), a client-chosen `evaluation_context_ref`, digest over the wrong set).
- `test_01 ...evaluate_only_invocation_runs_the_native_evaluation...`: no admission existed, so the evaluate-only run
  answered 202 `manual_reconcile` / `unexpected_outcome` / `escalated` instead of 200.
- `test_02 ...auth_negatives_never_reach_a_run` and `test_05 ...restart...`: raw `httpx.post` with a hand-minted token
  without the `job_id` claim, 403 `job_mismatch` (ADR 0011 item 1; the previous fix only covered `Bridge.invoke`).
- `test_07` x9 (frozen-before-human, bot credential, wrong hash, approval, replay, publish, promote, revoke, no-JWS-bytes):
  all downstream of the missing admission: proposal stuck in `candidate` (never `evaluated`), `illegal_transition`
  ("se necesita evaluated/approved"), then `release` None.
- Arms already passed through the deprecated aliases; they were moved to the annex names anyway.

## Fix (RED first: `tests/unit/test_annexd_dto_first_red.py`, 3 failed -> 3 passed)
- `dto.evaluation_context_ref(...)`: the contract formula (`evc-` + sha256 hex of `tenant|job|binding|proposal|candidate|attempt`
  [:40]); the unit test evaluates `contract.json idempotency.admissions.derivation.evaluation_context_ref` literally and
  compares. The engine uses it for the writer commitment and live asserts it equals the admission response ref.
- `dto.admission(...)`: no `evaluation_context_ref`, Z-suffixed `deadline`, lowercase-hex `request_digest`; checked against
  `bridge-contract/schemas/EvaluationAdmissionRequest.schema.json` (closed property set, required, patterns).
- `dto.arm_request(...)`: `execution_profile` (`evolution_task` / `attention_stateful_complementary`), `sandbox_session_ref`,
  `deadline` (Z), no `agent_id`; the task_builder bank arm has no annex profile so it keeps the `mode` alias (ADR 0011 item 3).
  `Idempotency-Key` header (already sent by `Bridge.arm_run`) equals the body key.
- Live tests: stale/unknown-budget/other-tenant admissions vary `candidate_hash`/`evaluation_attempt` instead of an
  explicit ref; raw-token invokes carry `job_id`. `demo/src/pulso_demo/driver.py` uses the same DTO helpers.

## GREEN
- e2e-core run.ps1: unit 28 passed; live 44 passed / 1 skipped (bank lost-response needs a stateful scenario).
- Throwaway PG16 (pulso-dev, cgroups disabled, 127.0.0.1, PULSO_REQUIRE_POSTGRES=1): lint 3 ruff jobs pass; local/core
  Pester 100 passed / 0 failed / 1 skipped; local/core python 20 passed; `test_image.py` 24 passed; demo/run.ps1 real_local
  outcome ok. No runtime defect found.
