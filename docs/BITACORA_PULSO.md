
## 2026-10-03T14:51Z UTC - CLAUDE: run-input slots re-exposed as bind_context facts
- Core 1.3.0 keeps run-input slots claimed; resolve_path/rule data read only validated slots. `pulso/bind_context` now returns the stage-declared inputs as tool-origin facts; Flows read `facts.binding.value.*`. Writer Flow draft plan path fixed to `facts.draft_plan.value.content.*` (artifact_get nests content). InvocationContext.inputs added (invoke/service.py passes inv.input, one line). Scout and writer E2E on real Core + PG16 pass.

## %Y-%m-%dT%H:%MZ CLAUDE: LLM gateway consumer P0 (core-bridge)

- Metering v2 (llm/metering.py, store/receipts.py, migration l3_002): failed calls with usage metered; crossing call counted then refused (ledger outcome over_cap); usage_known honoured (reservation kept); atomic reservation (meter_reserve); model_call_ledger (metadata only). Optional stage pin PULSO_LLM_STAGE_POLICY[_JSON] (alias/model/price from our config).
- Fail-closed: missing/invalid AGENTCORE_LLM_GATEWAY_URL/TOKEN, token "unset", PULSO_CORE_SHA != pin => exit 2 pulso:runtime_config_invalid naming the piece; PULSO_LLM_MODE=disabled explicit escape. Readiness llm_gateway = GET /healthz (<500); key_files also fails on verifier last_reload_error.
- No new receipt outcome (CX-0073/0075): dependency failure = audit agent_step kind=failed + error_kind; verified with real Core + exporter; fixtures in core-bridge/tests/fixtures/dependency_evidence.
- Suites on PG16 (pulso-dev): llm 31, dependency_evidence 8, runtime 79(+4 skip), l3a 81, l5 105, l3b 112, l6 49, integration 37: all pass.
# 2026-10-03 — CODEX — Consolidated E0 proposal-plan validation and PR preparation

- Rebased and consolidated verified P1/P2/E0 work on live GitHub `main`
  (`589e8dd135da9253a1afeec921bd2768311698d1`); current local head is
  `11615ae2718f1949619229df452bf8c3618ad524`, 14 commits ahead, clean before
  this documentation update.
- Ran `scripts/verify-local-ci.ps1` on Windows at the consolidated head: all
  eight local gates passed, including format, workspace Clippy, Rust tests,
  Python contracts/fixtures, and Pester. The destructive PostgreSQL test was
  intentionally opt-in and ignored.
- Ran the final-head E0 smoke against the augmented sample. It produced one
  descriptive candidate (154/200 recurring-query cases), a replicated but
  descriptive-only holdout (1,433/1,539), and an investigation plan recommending
  mapping review. No exact route was linked; formal route is `do_nothing`,
  review is pending, execution is forbidden, and the plan is not an Agent Core
  artifact. The JSON and NDJSON each contain 16 events and end with the same
  plan status; timeline-parity contracts passed in local CI.
- The repeated OriginalBank scan was stopped before completion because of
  unusually slow I/O (3,696/7,671 files). The previous complete OriginalBank
  smoke remains evidence for the unchanged path; final-head regression tests
  passed, including no-plan behavior. This limitation is explicit in
  `docs/IMPLEMENTATION_STATUS.md` and journal 0065.
- Windows `gh` 2.98 is installed, but its `aleuse` token is invalid and
  `git push` cannot reach `github.com:443`. Published the content-verified
  consolidated snapshot through the authenticated GitHub connector and opened
  PR #78. GitHub compare confirms one commit ahead of main, zero behind, 42
  changed files, and all remote blob SHAs match the local committed files.
- PR #78's `rust-ci` run #127 failed in the PostgreSQL migration, Ubuntu, and
  Windows jobs. The connector exposes no job steps and log retrieval returns
  `BlobNotFound`. The project owner confirmed the GitHub Actions quota is
  exhausted; treat this remote red status as quota-blocked, not as a code
  failure. Local CI passed first; the PR remains reviewable, with merge gated
  by the team's policy for the exhausted-quota checks.

## 2026-10-03T00:00:00Z CLAUDE: PL-L3/PL-L2 platform-contract and platform_live simulator
platform-contract/ (schemas, event catalog, golden, conformance; 19 tests) and platform-sim/platform_live/ (11-table simulator with fault injection; 18 tests) added. See platform-contract/docs/journal-pl-0001.md.
## 2026-10-03 22:14 UTC — E0 exploratory design-intent adapter

- Added the internal versioned `pulso.e0_builder_design.v1` envelope in
  `e0_builder_design.rs`. It accepts only candidate-bound aggregate E0
  evidence plus a matching sealed U20 plan and E0 safety oracle. Tenant and
  source-run identifiers and opaque readiness commitments participate in the
  sealed digest but are excluded from serialized model context.
- Corrected the product boundary after adversarial orchestration review:
  missing/unlinked routes may proceed to exploratory model reasoning, because
  finding a missing route is part of the demo's value. The resulting contract
  remains only `ReviewRequired(UntrustedDesignIntent)` or `DoNothing`, with no
  Flow reference for an unlinked route. No `UntrustedChangeSpec`, compiler,
  registry write, evaluation, promotion, or Core-native claim is exposed.
  Any future conversion requires a distinct exact U35-gated trusted path and
  an identity/version derivation contract.
- TDD evidence: the first compile-only red was a missing import and is not
  counted as behavioral TDD. A controlled digest-guard mutation made
  `rejects_cross_run_or_changed_input_replay` fail at its intended
  `ResponseInputMismatch` assertion (0 passed, 1 failed); the strict guard was
  restored. A subsequent intended behavioral RED for unlinked exploration
  constructed actual U20 plan/safety test-composer values and failed because
  the old route gate returned non-executable (`0 passed, 1 failed, 141
  filtered`). The exploration-only change and corresponding tests are now
  authored; final GREEN/Clippy/fmt validation is pending the serialized Cargo
  slot.
- No hosted CI, Core runtime, model provider, registry mutation, or business
  lift was exercised. This is a design-intent adapter foundation, not the
  E0-to-native-Core E2E.

## 2026-10-03 22:24 UTC — E0 builder readiness binding correction

- The first final focused run exposed `ReadinessMismatch` for the real U20-E
  test fixture. A test-first assertion isolated the exact bad predicate:
  `E0SafetyOracle.source_snapshot_digest` is U20-E's parsed-source binding
  digest, while `ArtifactReference.digest` is the persisted artifact-content
  digest. These are distinct by design in `EvaluationPlan`.
- Added a crate-private read-only accessor for the already-sealed U20 parsed-
  source binding digest and corrected `is_bound_to_plan` to compare the exact
  binding digest. Tenant, plan commitment, and exact source `ArtifactReference`
  comparisons remain intact; no readiness is minted or weakened.
- RED evidence: the focused unlinked test failed at the new `is_bound_to_plan`
  assertion before the predicate fix. GREEN: all 8 focused feature-gated E0
  builder tests passed. Clippy passed with `-D warnings`; formatting check and
  `git diff --check` passed. The CI focused step runs this module with
  `local-simulation` on both OS matrix jobs. `actionlint` is not installed;
  PyYAML parsed the workflow successfully.
- Scope remains exploratory/review-only: this does not wire a model provider,
  authorize U35/compilation, submit Core writes, evaluate native artifacts, or
  demonstrate business lift. Independent adversarial review remains pending.

## 2026-10-03 22:53 UTC — Align numeric and RFC3339 E0 cutoffs

- Added a run-ingress invariant: the strict UTC whole-second parser must yield
  exactly `LocalRunMetadata.cutoff_unix_seconds`, the value used to filter
  event/query timestamps. A mismatch or malformed cutoff now returns
  `InvalidInput` before simulation. The serialized run continues to carry the
  RFC3339 cutoff; no duplicate numeric result field was needed.
- RED evidence: a run with numeric cutoff `1,775,001,599` and timestamp
  `2026-04-01T00:00:00Z` returned `Ok(())` before the invariant. It now rejects
  both this mismatch and an invalid calendar timestamp. The correctly matched
  cutoff (`1,775,001,600`) is accepted.
- Corrected three E0 test fixtures that paired `1,775,000,000` with
  `2026-04-01T00:00:00Z`: the `e0_core_draft_binding` helper and the
  `e0_mechanism_resolution` / `e0_investigation_plan` integration fixtures.
- Validation: `local_simulation::tests` 3/3, feature-gated E0 unit-test filter
  46/46, `e0_mechanism_resolution` integration 1/1, and
  `e0_investigation_plan` integration 8/8. Feature-gated all-target Clippy
  `-D warnings`, `cargo fmt --check`, and `git diff --check` passed.

## 2026-10-03 22:57 UTC — Align local-simulation integration cutoff fixtures

- The full `local_simulation` integration suite initially failed 2/23 because
  two OriginalBank/E0-source test fixtures paired numeric cutoff
  `1,750,000,000` with `2025-07-01T00:00:00Z` (`1,751,328,000`). The source
  timestamp is the intended boundary; both numeric test values now match it.
  This also lets the E0-source projection test reach its intended validation.
- RED: before the fixture correction, the full integration suite reported
  21 passed / 2 failed. GREEN: full suite 23/23. After fixture edits, E0 unit
  filter 46/46; mechanism-resolution integration 1/1; investigation-plan
  integration 8/8; all-target feature-gated Clippy `-D warnings`, rustfmt and
  diff checks pass. No production result shape changed.

## 2026-10-03 22:32 UTC — Remove untrusted Flow identity from builder input

- Adversarial review found that a public `E0RouteCatalog` can declare a
  syntactically valid Flow without proving registry existence. The design
  adapter no longer serializes any Flow id/version/content digest, including
  for a catalog match. The model-input status is `catalog_declared` (not
  `mapped`) plus the untrusted catalog digest; an unlinked route remains
  explicitly `unlinked` with no Flow identity. A local consistency check still
  rejects a supposedly mapped resolver result that lacks its declared ref.
- Added focused tests for both unlinked and catalog-declared serialization,
  and for rejecting model-injected `flow_ref`. Updated the feature contract to
  state that public route catalogs are hints, exact U35 admission and Core
  registry readback are required before relying on Flow identity, and this
  slice does not invoke an LLM/provider or implement provider traces/costs.
- Validation: feature-gated E0 design tests passed 9/9; feature-gated all-target
  Clippy passed with `-D warnings`; `cargo fmt --check`, `git diff --check`, and
  PyYAML workflow parsing passed. `actionlint` remains unavailable locally.
  Independent adversarial re-review is requested; no native Core execution or
  real provider call was performed.

## 2026-10-03T23:20:25Z — CODEX — CX-0161 — Consolidated E0 + P4 current-main validation

- Consolidator base is verified GitHub `main` SHA `0b0c919ddbc517faceda4a269af944f7d75a0c12`. Selective commits: `be475c4` (E0 review-only builder-design seam/cutoff equality) and `6c45d61` (P4 memory expiry public-read boundary and Debug redaction). No stale branch history was merged.
- Windows/Rust 1.98.1 workspace Clippy (`-D warnings`), rustfmt, and `cargo test --locked --workspace --features test-support` completed with every executed test passing; destructive PostgreSQL tests remained explicitly ignored without an isolated database. Focused E0 tests 9/9, `wiki_scratch` 13/13, `published_memory` 8/8. Python contracts: 10 passed/1 skipped; fixture validation passed; Pester local-CI 5/5 and local E0 runner 20/20.
- Actual E0 sample run at `output/e0-codex-e0-p4-2026-10-03/run_3256_1791069550608821200`: 200 discovery cases, 3 candidates, 154 recurring-query cases, 1,433/1,539 selected holdout matches labelled `descriptive_only`, 16 events. The plan recommends `investigate_mapping`, `pending_review`, `not_executable`; terminal state `complete_simulated`; formal route `do_nothing`. This is not provider execution, a native Agent Core proposal/evaluation, causal resolution, or measured lift.
- Podman is inaccessible in this process, so real-Postgres/container tests are not verified. Next implementation slice is a separate fresh-main runner seam that persists bound evidence separately from blocked builder readiness until real U20/E0 safety inputs are available. A disjoint P1 platform explanation is also under test-first port; neither lane may claim provider execution or native proposal creation.

## 2026-10-04T00:20:00Z UTC — CODEX — Current-main E0/P1 consolidation

- Reconstructed the exact merged GitHub main tree locally from verified parent `594be4e` and merged PR #83 head `c45a6ea`; `git merge-tree` produced the same tree SHA reported by GitHub (`72c7bcc`). The local synthetic merge commit is only a validation base; it does not replace GitHub's merge commit.
- Selectively ported the runner builder-preparation slice, P1 explanation and percentage-boundary tests to that tree. PR #83 docs were preserved as the base; only this consolidation's facts are appended.
- The existing runner persistence test exposed incorrect single-document parsing of NDJSON. It now parses each non-empty event line and checks ordered equality with `result.json`'s event timeline, including E0 preparation.
- Independent adversarial review of the E0 explanation found no privacy/lift/Core-state blockers. Follow-up is tightening holdout-to-signal association and naming/documenting the in-memory projection boundary before promotion.
- The exact post-PR-#85 tree is locally reconstructed and matches GitHub's tree SHA `1e9bdf8`; PR #85 touched only debug-console paths, separate from this Rust slice. Local full preflight passes on the final branch: Rust fmt, workspace Clippy `-D warnings`, workspace Rust tests, Python contracts (10 passed/1 Podman-dependent skip), fixtures, Pester (20/20 + 5/5). GitHub fetch/push by shell remains unavailable; publishing is being handled through the authenticated GitHub connector.

## 2026-10-04T00:25:00Z UTC — CODEX — E0 explanation projection added

- Added a deterministic, privacy-safe explanation projection to the local E0 result. It reports aggregate evidence, exact signal/candidate lineage, route and readiness blockers, and explicit non-executable/no-provider/no-Core/no-lift states.
- Independent review accepted the no-PII and truthful-state boundaries. Follow-up hardening binds holdout `candidate_ref` to the selected signal pattern and discovery commitment to the run manifest, validates status/count/suppression invariants, and labels the output as an in-memory projection rather than a durable reread.
- Final exact-main E0 rerun: `output/e0-current-main-explained-2026-10-04/run_4640_1791074275102278600`; 200 cases, 154/200 recurring-query cases, holdout 1,433/1,539 descriptive-only, one candidate, route unlinked, U20/U20-E blockers explicit, non-executable, no provider/Core invocation, and null business lift. Full local preflight passes on this final code. Podman/Postgres-dependent integration remains unrun; no hosted Actions result is claimed.

## 2026-10-04T02:47:00Z UTC — CODEX — CX-0183 — Post-merge worktree audit and selective consolidation

- Verified the exact live `origin/main` base at `41492cb7f2b20fc2d033643798659436dffb861a` after PRs #88 and #90 merged. Created one integration worktree at `improvement-engine/worktrees/improvement-engine-post-merge-consolidation`; no other worktrees/branches were reset, deleted, or pruned.
- Audited dirty worktrees and remote branches. Excluded P1 explanation/U13a admission, original-contact projection, and old memory composition because current `main` already contains newer equivalents. Selected additive U12-E→U13 local evidence composition, fail-closed platform-source privacy primitives, and fixture-only paired-scenario comparison.
- Consolidation adds sanitized candidate content digests to the U12-E/U13 result for debug visibility, labels the positive-count trigger `positive_count_plumbing_v1` (plumbing only; not calibrated for production), and represents oracle matches as `Option<bool>` so infrastructure failures are `not_evaluated` rather than behavioral false.
- Two independent adversarial reviewers found no immediate PII egress or production-authority confusion. Their actionable findings are recorded in `docs/journal/0067-post-merge-selective-consolidation.md`; the platform reader contract remains policy primitives only, not an enforced live exporter. Contract-backed schema/exporter tests remain a pre-live requirement.
- Windows `scripts/verify-local-ci.ps1` passed: Rust 1.98.1 fmt, Clippy `-D warnings`, workspace unit/integration/doc tests, Python contracts/fixture validation, and Pester 20/20 + 5/5. Isolated destructive PostgreSQL tests were not run. The exact U12 CLI candidate-summary regression test passed. No hosted Actions result is claimed.
- Cleanup inventory is `docs/PULSO_LOCAL_CLEANUP_INVENTORY_2026-10-04.md` in the shared workspace. Fresh worktree count is 70 (including two Claude worktrees omitted from the earlier list); inventory paths were reconciled exactly. No pre-existing worktree, generated data, receipt, branch, or container was removed; Podman inventory remains inaccessible.
- Next: commit and publish this consolidated delta as one PR against `main`; do not merge automatically.

## 2026-10-04T02:52:00Z UTC — CODEX — CX-0184 — PR #93 published and refreshed on latest main (reply-to CX-0183)

- Opened consolidated PR [#93](https://github.com/pulso-factored/improvement-engine/pull/93) from `codex/post-merge-consolidation` to `main`. GitHub initially reported non-mergeable because `main` advanced to PR #92 after our base fetch; merged that single unrelated upstream commit locally, reran the complete local CI preflight successfully, and pushed the updated head `838c03a456a332e887082ffc8948509457d79dee`.
- Fresh GitHub PR metadata reports `open`, `mergeable=true`, base `main` at `ca5af1159741a564363f566ec02a8aaa4177fbdf`. Combined commit status endpoint currently returns no status entries; hosted Actions result is therefore pending/unreported, and local tests remain the verified gate.
- No merge performed. User review/merge required.

## 2026-10-04T16:56:02Z UTC — CODEX — CX-0240 — PQR projection adversarial hardening (in progress)

- Re-read the authoritative team plan before continuing. This increment remains in the Codex-owned X-SRC/X-SENS surfaces; Claude's `seams/`, demo, console, and runtime work were not touched. The next independent candidate after this slice is BKF in X-ARTIF; it is not started yet and must consume the frozen BK0 digest and existing registrations without drive-by manifest/lib edits.
- An independent final review found two P2s and one P3: unsafe raw unsupported-reason strings at the runner boundary, per-metric small denominators leaked despite cell `k=5`, and missing CLI-level absent-table coverage. The runner already maps the two supported source diagnostics to bounded safe codes; adapter regression now locks the invalid-timestamp case. Metric disclosure now suppresses all counts/values for any non-zero 1–4 row subgroup (and both sides of the binary SLA split), marked `suppressed_small_denominator`; `k=5` is explicitly documented as heuristic, not formal anonymity. Added CLI regression for an absent complaints table and no fabricated PQR evidence.
- Pinned Windows Rust 1.98.1 formatting passes. The focused source-adapter suite passed 20/20 (including invalid timestamps and small-metric-denominator redaction). The core `local_simulation` integration test is currently compiling/running with `CARGO_BUILD_JOBS=1`; no result is claimed yet. Runner/CLI tests, full `scripts/verify-local-ci.ps1`, the post-change original-source PQR E2E, and the final independent adversarial pass remain outstanding.
- Live GitHub metadata: PR #95 remains OPEN/DRAFT, `mergeable=true`, head `12299f15126ed0c1e246108af21cb2add0ac1d4b`, base `707c5b4bb6394203ba818fbfbf73b9bcc362e753`. The uncommitted PQR hardening is not yet on the PR. Its current description is stale about DREPLAY and needs reconciliation with CX-0236/0237 and current residual acceptance gaps before review-ready status. Hosted checks on the published head failed immediately with zero status entries; given known Actions quota exhaustion they are not considered local verification.
- No additional PR, force push, merge, raw-data disclosure, or Claude-owned edit was made.

## 2026-10-04T17:35:29Z UTC — CODEX — CX-0241 — PQR disclosure boundary follow-up (in progress)

- Followed X-SRC/X-SENS ownership from the current two-team plan. All work remains in the existing PR #95 worktree; Claude `seams/`, demo, console, runtime, and other owned paths were not edited.
- Closed adversarial follow-ups: the core now rejects unbounded `unsupported.missing_fields` values via a closed diagnostic allowlist; a usable complaints source with zero k-qualified aggregates emits `supported_no_reportable_cells`, contributes no detector evidence, and has a non-misleading event; a CLI regression covers present-but-unsupported input, safe diagnostic vocabulary, no finding, and absence of synthetic identifiers/free text.
- TDD: `unsupported_complaint_projection_rejects_unbounded_diagnostic_codes` failed before the core fix because `alex` was accepted, then passed after the allowlist. The no-reportable-cells core regression passes. Focused `local_simulation` integration suite: 30 passed, 0 failed. Pinned format check and `git diff --check` pass. Source adapter integration tests: 20/20 from the prior pass. CLI unsupported/absent regressions, full `scripts/verify-local-ci.ps1`, post-change original-source smoke, and final adversarial pass are outstanding; runner CLI validation is currently active under one Cargo job.
- PR #95 is still OPEN/DRAFT at the last live check (`12299f1` published head); CX-0241 is uncommitted/unpublished. Independent scope audit found stale replay/campaign statements in the PR body; rewrite it to acknowledge coordinator/checkpoint support while limiting claims (not an E0/FRZ0 acceptance campaign, scorer, or lift proof), and preserve true residual gaps before publication.
- Keep the PR consolidated; no new PR, force push, merge, hosted Actions reliance, E0/CSV transfer, or Claude-owned edit. Isolated PostgreSQL behavior, the E0 relationship blocker, Spec-22 goldens, and concrete platform-source integration remain open.

## 2026-10-04T18:00:59Z UTC — CODEX — CX-0241a — Focused E2E gates green; full local CI started

- The core `local_simulation` integration target passes 30/30, including both CX-0241 core regressions. Windows runner CLI E2Es pass for present-but-unsupported complaints (1/1) and absent complaints (1/1); the unsupported case checks bounded diagnostics, no finding, and no fixture identifiers/free text in the output. Pinned `cargo fmt --all --check` and `git diff --check` pass.
- Started `scripts/verify-local-ci.ps1` on this exact worktree with `CARGO_BUILD_JOBS=1`; pinned toolchain and formatting pass, Clippy is underway. Full result remains pending; no other Codex Cargo build will run concurrently.
- PQR CX-0241 remains uncommitted/unpublished in the consolidated PR #95 train. Full local CI, bounded original-source smoke, and final independent adversarial review remain gates before commit/push or PR-body refresh.

## 2026-10-04T18:17:27Z UTC — CODEX — CX-0241b — Bound diagnostic list after final review

- Final independent adversarial review found one additional core-boundary P2: repeated allowlisted diagnostics were accepted without a list-size bound, allowing unbounded serialized status. TDD RED reproduced acceptance of six repeated values; constructor validation now requires 1–5 distinct values from the closed diagnostic vocabulary. Regression covers oversized/repeated and duplicate diagnostics.
- New diagnostic regression passes; complete `local_simulation` integration target passes 30/30. Documentation now names `supported_no_reportable_cells` and distinguishes usable source rows from public, k-qualified evidence.
- Re-started final Windows `scripts/verify-local-ci.ps1` with `CARGO_BUILD_JOBS=1`; toolchain and format passed, Clippy underway. The earlier full-suite attempt stopped during Clippy, before running tests, due the newly found P2; it is superseded and not counted as final verification.
- Changes remain uncommitted/unpublished in existing draft PR #95. Full local CI, bounded original-data smoke, and final independent review are pending before commit/push and PR-description refresh.

## 2026-10-04T19:35:47Z UTC — CODEX — CX-0241c — Local gates complete; final blank-channel review closed

- The final full Windows `scripts/verify-local-ci.ps1` run exited 0 on the CX-0241 implementation: pinned toolchain, format, Clippy, Rust unit/integration/doc tests, Python contract tests, fixture validation, and Pester. Core unit tests numbered 207; CLI E2E passed 24/24; source-adapter integration passed 20/20; Python ran 25 with one Podman-dependent skip; fixture validation reported 11/11 Spec-22 families present and six wire contracts pending; Pester passed 22+5. Isolated/destructive PostgreSQL tests remained ignored by explicit opt-in design. After that suite, added a final valid-timestamp whitespace-channel regression and the source-adapter target passed 21/21.
- The independent reviewer's tentative blank-channel concern was retracted after tracing `csv_value` (trim + filter-empty) into `normalize_channel`. The regression uses five valid timestamps and whitespace-only channels; it asserts `unsupported`, `usable grouping rows`, no aggregates, and no synthetic IDs in serialized output. The regression passed; no production code change was needed because the existing call chain already rejects blank values. An initial invalid-timestamp fixture was corrected before accepting the result. An initial isolated Cargo call omitted `local-simulation`, failed at compile configuration, and was rerun successfully with the required feature.
- Actual local original-source smoke passed against the explicitly selected `target-local-e2e/original-bank-pqr-input`: contacts had 25 partitions, 16,277 descriptive rows, 48 visible cells; complaints had 25 partitions, 1,452 descriptive rows, 19 visible cells. Coverage is partial, and only aggregate counts were emitted. This is final-extract descriptive evidence, not as-of replay, causal impact, ROI, or production acceptance.
- Final adversarial review found no remaining actionable P1/P2 in this PQR increment. Changes remain uncommitted/unpublished at this entry. GitHub shell access (`gh pr view`) is blocked by process network policy; the authenticated connector is usable. Live PR #95 is open/draft/mergeable at the old head; current main is `85dc24ba0f48f7c0766520bfac918c08007e426f`, and `git merge-tree origin/main HEAD` reported a clean merge tree. Before publication, reconcile with current main, rerun local gates on the resulting tree, and update the existing PR body accurately; do not open another PR.
