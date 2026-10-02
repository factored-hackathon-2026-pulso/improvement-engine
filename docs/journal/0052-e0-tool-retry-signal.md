# E0 tool retry case signal

## Decision and scope

Carry the existing nullable E0 `tool_call.retry_count` through `SafeEvent` and
the local run projection, then report one descriptive case-level rate. This
does not add source tables, inspect tool parameters, or alter the source
dataset. The retry metric is measured separately from the existing technical
error signal; retry count is not an error label or proof of customer harm.

## Metric contract

- Numerator: distinct Arranque cases with any known tool-call
  `retry_count > 0`.
- Denominator: distinct Arranque cases with at least one known retry count,
  including zero.
- Missing: all Arranque cases without a known retry count. A case with no tool
  call or only null retry counts is missing, not a zero-retry case.
- A case with multiple calls counts once in numerator/denominator. Any known
  positive retry makes that case positive; partial-null calls do not erase the
  known observation.
- The proposal may describe the observation and recommend investigation. It
  must not claim cause, prevented cost, savings, or business lift. The draft
  remains simulated, unverified, and unexecuted.

## Implementation

`E0Fact::ToolCall.retry_count` is propagated through `SafeEvent` into
`LocalObservedEvent`. The local signal uses the read-only investigation lab and
is persisted alongside the existing run metrics. The draft evidence extension
is aggregate-only: retry cases, known-denominator cases, and missing cases.

## Verification

The supported-range test first failed against the unchecked `u64` to `u32`
projection, then passed after conversion became checked and oversized values
were rejected. The retry metric test verifies distinct case-level counting,
known-zero denominator membership, explicit missing cases, aggregate-only
proposal evidence, and non-causal wording. A timeline assertion caught and
prevented retries from being labeled as technical errors.

Post-fix Windows verification on the current branch:

- `cargo +1.98.1 fmt --all -- --check` and `git diff --check` passed.
- Targeted Rust integration targets passed: `local_simulation` 8/8,
  `cli_e2e` 6/6, and `source_adapters` 8/8.
- `Invoke-Pester -Path .\tests\run-local-e0-e2e.Tests.ps1 -PassThru` passed
  10/10.
- A real local E0 smoke completed in local-simulation mode using 200 Arranque
  discovery cases. `e0_tool_retry_case_rate` was 0/187 known cases with 13
  missing; the replay count remained null. The output proposal was
  `simulated_unverified` and `not_executed`.
- The generated result and timeline were checked for email-pattern and known
  sentinel leakage; none was found. The wrapper prints only allowlisted
  aggregates.

Independent adversarial re-review of the corrected retry slice is still
pending. No PR or merge is claimed by this journal entry.
