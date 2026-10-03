# 0053 — E0 retry / technical-error co-occurrence evidence

## Goal and scope

Add an aggregate-only association diagnostic to the simulated E0 proposal so
the engine can show whether observed retry-positive cases also had an explicit
technical-error event. This is additional evidence for investigation, not a
new detector, causal claim, business-impact estimate, or Agent Core artifact.

## Contract

- Group discovery events by case ordinal only; no customer, event, query, or
  source identifiers are emitted.
- A case is retry-positive if any known tool retry count is greater than zero.
- Retry coverage is evaluated per discovery case across `tool_call` events.
  Every case must have at least one `tool_call`, and every such call must have
  a retry count. No `tool_call` is unknown/not-applicable, not zero.
- No status reports zero retries. Any complete zero/small support is coarsened
  to `suppressed_below_minimum_support`; incomplete ToolCall coverage yields
  `insufficient_retry_status_coverage`.
- A case has known technical-error status if an event explicitly carries
  `technical_error=true` or `false`; true takes precedence across events.
- The rate denominator is retry-positive cases with known technical-error
  status. Retry-positive cases without that status are excluded, and their
  count is not serialized.
- Policy `e0_retry_error_overlap_k_v2` requires at least five co-occurring and
  five non-co-occurring denominator cases. If either cell is smaller, all
  counts and the rate are withheld, including complementary counts. Status
  and interpretation are coarsened to avoid revealing zero/low support.
- Interpretation always states that co-occurrence is descriptive and is not
  causal or directional evidence.

## TDD record

- First RED: the proposal had no `observed_evidence.retry_error_overlap`; the
  integration-level core test failed at the expected missing field.
- A complementary-suppression regression was then added with five
  co-occurring cases but only three non-co-occurring cases. Temporarily
  weakening the privacy predicate caused the expected RED (`reportable` was
  returned instead of suppression). The final implementation requires both
  cells to meet k=5.
- GREEN: the reportable fixture yields a 5/10 overlap rate (5,000 basis
  points) with k=5 in each cell. A distinct fixture with five co-occurring but
  only three non-co-occurring cases is suppressed, and every overlap count and
  rate is null. The runner CLI regression confirms the versioned diagnostic
  persists inside the proposal and suppresses its small cells.
- A technical-error missingness regression required an unavailable status when
  no retry-positive rows had error classification; status wording was kept
  generic so it does not declare the positive retry count.
- Independent adversarial review found that missing `retry_count` values could
  still be mislabeled `no_retry_observed`. Two RED fixtures (all retry counts
  missing; partial missing with known zeros) reproduced the defect. The fix
  checks per-case retry-count coverage before status selection; a third fixture
  protects the complete all-known-zero case (later privacy coarsening changed
  its serialized status). The CLI fixture with one missing retry count now expects
  `insufficient_retry_status_coverage`.
- A second adversarial pass found an intra-case gap: treating any `Some(0)` as
  complete ignored another ToolCall with a missing count. RED fixtures for
  `[Some(0), None]` and `[Some(1), None]` reproduced the misclassification.
  The DTO distinguishes ToolCall rows from other events; the final rule now
  requires at least one ToolCall per case and complete counts for every such
  call. A case with no ToolCall is unknown, never zero. The positive+missing
  case retains positive evidence internally but the global status and
  denominator remain incomplete and all overlap values are withheld.
- GREEN covers mixed zero/missing and positive/missing calls within one case,
  cases without ToolCalls, all/partial case-level missing status, and complete
  all-zero calls. CLI assertions retain the source fixture's missing status.
- A final privacy review found that categorical `no_retry_observed` and
  `insufficient_error_status_coverage` could reveal zero or positive support
  despite null counts. RED proved those labels leak that distinction. A third
  review found that even `insufficient_status_coverage` disclosed a sub-k
  positive retry when the matching error status was absent. A new one-positive
  / missing-error regression reproduced it. The final implementation removes
  that branch: all complete-coverage non-reportable cases (zero, low support,
  or missing technical-error status) share one generic suppressed status, so
  the serialized category does not imply whether a positive retry was observed.
  The wire contract is bumped to `e0_retry_error_overlap_k_v2`; zero/small
  support and incomplete technical-error status use the generic
  `suppressed_below_minimum_support`. Only incomplete retry-count coverage has
  the separate `insufficient_retry_status_coverage` status, which says nothing
  about whether retries were positive. Only `reportable` discloses association
  values, and then only when both cells satisfy k=5.

## Verification

- `cargo +1.98.1 test --locked --offline -p improvement-engine-core
  --features local-simulation --test local_simulation`: 20 passed, including
  call-level mixed-missing, no-ToolCall, and categorical coarsening regressions.
- `cargo +1.98.1 test --locked --offline -p improvement-engine-runner
  --test cli_e2e`: 6 passed, including persisted overlap status and privacy
  suppression assertions.
- `cargo +1.98.1 clippy --locked --offline -p improvement-engine-core
  -p improvement-engine-runner --all-targets -- -D warnings`: passed.
- `cargo +1.98.1 fmt --all -- --check` and `git diff --check`: passed.
- A fresh v2 local E0 run (`run_1768_1790984365144288000`) used 200
  Arranque discovery cases and completed as
  `complete_simulated`. Technical errors were 0/187 with 13 missing; retries
  were 0/187 with 13 missing; query recurrence was 154/200. The holdout
  reported 1,433 matches / 1,539 queried and `replicated`. The overlap status
  was `insufficient_retry_status_coverage` because 13 retry counts were
  missing, with every overlap count/rate null. The proposal remained
  `simulated_unverified` / `not_executed`; replay count was null.
- Email-pattern scan of the generated result and timeline found zero matches;
  only the aggregate fields listed above were inspected.

The real sample did not exercise a positive retry/error overlap or the
insufficient-status branch; reportable, complementary-suppression,
all/partial missing retry status, mixed per-call missingness, no ToolCall, and
missing error-status behavior are covered with synthetic core fixtures, and
CLI serialization/missingness is covered separately. The smoke is evidence
of local composition only, not causal evidence or production prevalence.

## Limitations

This association does not prove retry caused a technical error, that an error
caused a retry, or that either observation caused a later contact or business
loss. Missing statuses are not converted to no-error. Existing proposal
execution remains simulated, unverified, and not executed.
