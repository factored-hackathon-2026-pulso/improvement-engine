
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
