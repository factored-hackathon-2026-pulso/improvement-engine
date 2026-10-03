# Journal claude-0006: L5 evaluation, harness, arms

Contract revision: `pulso-two-teams-1`. Pin `86a767474042a566a0dbd6ed23588959f27ebdb3`. Package: L5 (plan 17.3.5).
Code: `src/pulso_core_runtime/{evaluation,harness,registry_service}.py`. Commits: 122dca4, 3e4a041, 179d4f6 (review r2),
8e569a1, 2037218, b3af773. Documentation pass at HEAD `4984d92`. Flow: `docs/flows/core-evaluation-admission-arms.md`;
ADRs 0002 and 0007.

## Purpose
Evaluation that cannot be replayed, forged or run without an admission: `PulsoScenarioHarness` (three modes), admissions
with CAS, the native FIFO port, the fail-closed shared registry service, full-report persistence, arms for task-builder
and stateful-attention campaigns, and the HTTP-campaign wrapper.

## Flow
See the flow document: admit -> `AdmissionGate.begin` -> per-admission service clone -> `BoundEvaluator` ->
`ScenarioEvaluator` under the evaluation gate -> persist the full report with `pulso_evidence` -> tool/HTTP result. Arms:
eight steps with a stored per-key report.

## Input / output
Admission body: `evaluation_context_ref, binding_ref, proposal_id, candidate_hash, suite_id, suite_version, suite_digest,
evaluation_attempt, budget_ref, deadline, request_digest`. Native result `{verdict, eval_run_ref, report_digest}` (and the
stored report). Arm request/report fields per `ArmRequest` / the report dict in `evaluation/arms.py`.

## Transactions and idempotency
`eval_admissions` CAS; registry write key `pulso-eval:<ref>` (ADR 0007) with Core's service-level idempotency; arm
single-flight by tenant-scoped key; budget ledger `meter_spend` atomic and capped (key tenant/execution_id/"evaluation"/0).
Unique run keys per execution so a persistent eval DB never replays a stored run.

## Errors
Admission: `admission_missing`, `admission_cross_tenant`, `admission_proposal_mismatch`, `admission_attempt_mismatch`,
`evaluation_unknown`, `evaluation_in_progress`, `admission_not_admitted`, `admission_expired`, `candidate_changed`,
`suite_mismatch`, `broker_denied`, `budget_unknown`, `evaluation_attempt_exists`, `idempotency_conflict`,
`evaluation_context_invalid`. Arms: `invalid_request`, `mixed_world_rejected`, `sandbox_required`, `supersedes_invalid`,
`idempotency_key_invalid`, `broker_denied`, plus `failed_infra` reasons (`manifest_missing`, `manifest_empty`,
`budget_unknown`, `target_preparation_failed`, `sandbox_timeout_after_send` is `unknown`).

## Permissions
Purposes `evaluation_admit`, `evaluation_arm_run`, `evaluation_arm_read`; broker scopes `evaluation_admit`,
`native_evaluate`, `evaluation_arm`, `artifact_read`, `sandbox` (ADR 0005). Evaluation composes from the unguarded gateway
with `EvalAuthz` (synthetic principals only), never the live authz.

## Config
`PULSO_EVAL_PERMITS` (default 1), `PULSO_EVAL_BUDGETS` (static JSON resolver), `AGENTCORE_EVAL_DSN` (isolation checked by
`assert_isolated`), broker URL.

## Observability
`pulso_evidence{closed_early, closed_early_runs, runs}` in stored native reports, `ArmReport.closed_early*`, usage and
`cost_known`, `/version.doubles[]` lists the static budget resolver.

## Commands (head `4984d92`)
`python -m pytest -c pyproject.toml tests/l5 tests/integration -p no:cacheprovider` with `PULSO_TEST_PG_ADMIN` (PG16 throwaway,
`--cgroups=disabled`); `ci.ps1 -Job core-bridge`. Environment as in journal claude-0002.

## RED / GREEN
First RED (`tests/l5/test_collision.py`, marked subject-to-verification): on a persistent eval DB the stock harness
replays `eval-{scenario.id}`; reproduced on real PG16, then fixed by unique run keys. GREEN as reported: 78 tests in the first
slice; 240 passed at review r2 on PG16 (suite-level count at that time). Findings recorded: dotted sandbox tool ids are
invalid as `EntityRef` (underscore forms used); the plan's "7 run ids" arithmetic is 6/9; `ROUTES` lacked
`/arms/run` and `/arms/by-key` (added). About 77 test functions exist in `tests/l5`. Not re-run here.

## Trade-offs
One admission = one native evaluation (a `failed_infra` is replayed, retry needs a new admission). A single evaluation
semaphore trades throughput for isolation of the process-wide harness. Early close is reported as evidence; Codex scores.

## Gaps
- Arm DTO names pending Codex confirmation (A05).
- Budgets from a static file; control-api budget contract undefined.
- Stateful bank is a test double; real broker sandbox not exercised.
- Persistent replay across process restarts not asserted as verified.
