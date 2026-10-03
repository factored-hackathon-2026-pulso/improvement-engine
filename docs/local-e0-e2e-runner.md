# Windows local E0 E2E runner

The PowerShell wrapper runs the engine's explicit local-simulation mode against
an E0 package on disk. It is intended for repeatable local checks, not business
evaluation or release.

## Prerequisites

- Windows with PowerShell 7 or newer.
- Rust/Cargo matching rust-toolchain.toml.
- The workspace dependencies already available locally; Cargo is always invoked
  with --locked --offline, so the command does not fetch crates.
- A local E0 package containing the expected datos/ and contratos/ layout.

## Run

From the repository root:

    pwsh -NoProfile -File .\scripts\run-local-e0-e2e.ps1 -InputPath 'D:\.codex\factored\pulso_muestra_e0' -OutputPath '.\output\e0-run-2026-10-02-a' -ObservedCutoff '2026-10-02T18:00:00Z'

InputPath, OutputPath, and ObservedCutoff are required. The cutoff must
be UTC with whole-second precision. Defaults are 200 Arranque cases and a
20-case recurrence-support floor. Override them explicitly when needed:

    pwsh -NoProfile -File .\scripts\run-local-e0-e2e.ps1 -InputPath 'D:\data\e0-package' -OutputPath '.\output\e0-run-custom' -ObservedCutoff '2026-10-02T18:00:00Z' -ArranqueCases 200 -MinimumRecurringQueryCases 20

The output path must not exist, must not overlap the input tree, and neither
path may traverse an existing junction, symlink, or other reparse point in the
path components inspected by the wrapper. Choose a new output path for each
run. The wrapper never deletes or overwrites data. The engine writes its
immutable run result and event timeline below that output directory.

For a direct CLI run, `--progress-jsonl` opts into live, sanitized phase
progress on stderr. Each flushed JSONL record contains only schema version,
one allowlisted phase (`source_preparation`, `detection`,
`post_selection_holdout`, or `persist_outputs`), an allowlisted state
(`started`, `completed`, `skipped`, or `failed`), and elapsed milliseconds
since progress reporting began. It contains no run/tenant/source identifiers, paths,
source values, proposal content, or underlying error text. A skipped holdout
means the post-selection E0 holdout preconditions did not apply; it is not a
zero result. This progress channel is diagnostic only: it does not change the
atomic `result.json` / `events.ndjson` artifacts, and it is not a durable U07
event ledger, OpenTelemetry exporter, health endpoint, or production monitor.
The PowerShell wrapper does not currently expose this option.

This is not a defense against every filesystem alias or race: mapped-drive and
UNC aliases are not canonicalized against one another, and another process
must not mutate/repoint path components concurrently with validation or the
run. The guarantee is limited to rejecting reparse-point components observed
during the wrapper's preflight checks.

## Original-bank snapshot-only discovery

The E0 PowerShell wrapper does not accept the original-bank source. For a local
original-bank run, invoke the binary directly with a fresh output directory:

    cargo +1.98.1 run --locked --offline -p improvement-engine-runner -- local-sim --mode local-simulation --source original --input 'D:\data\bank-extract' --output '.\output\original-run-2026-10-02-a' --tenant-id pulso_local --observed-cutoff '2026-10-02T18:00:00Z' --arranque-cases 1

The original contact adapter emits a second, distinct projection for
descriptive final-extract facts. It groups only rows with a valid literal
source timestamp month and usable reason/channel codes. The `k` denominator is
the count within each such month × reason × channel cell before suppression;
only cells with at least the fixed policy-v1 floor (`k=5`) contribute to
`supported_contact_count`. Rejected rows and suppressed cells do not contribute
to that denominator, and their exact totals are not exposed in agent inputs or
run results. Coverage is always
`partial`; visible month labels are source wall-clock calendar text, not a
timezone-normalized event time. No `observed_cutoff` is attached to this
projection or its finding.

When supported complaint cells exist, the run may emit a descriptive finding
and an unverified, non-publishable local hypothesis envelope, bound to the source snapshot, source manifest,
and projection digests. This envelope is explicitly not a U13/Agent Core
candidate: `agent_core_candidate=dependency_blocked_snapshot_semantics`,
proposal status `simulated_unverified`, execution `not_executed`,
`publication_eligible=false`, and formal route `do_nothing`. It does not create
an Agent Core query receipt, compile an artifact, or prove cause, ROI, efficacy,
or improvement. Agent Core/U13 currently has no offline snapshot-candidate
contract; an output adapter requires a future contract extension. The top-level
run may still carry its required invocation cutoff as run metadata, but that
cutoff is not evidence for the snapshot-only finding or proposal.

## Output and safety

The wrapper prints only allowlisted run status, discovery counts, and a
suppressed replay total for E0,
allowlisted metric IDs with numerator/denominator/missing counts, proposal
status/execution status, formal route, and (when present) a holdout status.
Holdout matching/queried distinct-case counts are shown only when support meets
the configured floor; `insufficient_support` prints `counts=suppressed`, and
the engine always serializes the E0 top-level excluded-replay total as null,
even when holdout evaluation is absent or unavailable. It must serialize all
holdout count/rate fields as null below the floor. A missing
holdout field is summarized as `none`; it is not treated as zero. It does not
print source paths, customer/case/query identifiers, query signatures,
hypotheses, arbitrary interpretation strings, or raw Cargo/JSON diagnostics.
Unexpected result codes, metric IDs, malformed JSON or invalid aggregates fail
closed without printing the result. Treat generated artifacts as sensitive
derived data: hashing is not anonymization.

When the E0 projection includes tool-call retry counts, the engine reports
`e0_tool_retry_case_rate`: numerator is distinct Arranque cases with at least
one known `retry_count > 0`; denominator is distinct cases with at least one
known retry count (including zero); missing is every discovery case without a
known retry count. No tool event or a null retry count is not treated as zero.
Retries are operational observations only: they do not establish failure
cause, customer harm, preventable cost, or savings. They may inform a simulated,
unverified review draft, never an executed Agent Core artifact.

When the draft includes `retry_error_overlap`, it compares positive retries
with explicitly observed technical-error status at the case level. Its
versioned `e0_retry_error_overlap_k_v2` policy requires at least five cases in
both the co-occurring and non-co-occurring cells; otherwise the counts and rate
are null. Retry-positive cases with unknown technical-error status are omitted
from the denominator, and their count is not serialized. A reportable overlap
is descriptive only; it establishes neither causality nor direction. This
detail is present only in the generated run artifact and is not printed by the
PowerShell wrapper. Retry completeness is checked per discovery case across
`tool_call` events: a case needs at least one observed call and every call
needs a known count. A case with no `tool_call`, or mixed known/missing call
counts, is unknown, not zero. `insufficient_retry_status_coverage` means at least one
case had no ToolCall or a missing retry count; known positive evidence does
not make the cross-signal denominator complete, so counts/rate remain
withheld. `suppressed_below_minimum_support` is deliberately generic: it also
covers zero support and incomplete technical-error status, without revealing
whether a positive retry was observed or which cell is small. No serialized
status claims that zero retries were observed. Unknown status is never
presented as no retries or no error.

The wrapper selects local-simulation mode; it makes no provider or Agent
Core request. The observed query recurrence is descriptive only. It is not
evidence of customer friction, causality, holdout efficacy, business lift, or
production behavior.

## Test

Run with Pester 3.4.0 installed:

    Invoke-Pester -Path .\tests\run-local-e0-e2e.Tests.ps1

The Windows CI job requires the exact Pester 3.4.0 module to already be
available and does not install or upgrade it. The tests use a temporary local
cargo shim to verify argument construction, safe summary filtering, mandatory
cutoff, output/input isolation (including input and output junction paths),
no-overwrite, and sanitized failure behavior. They do not build Rust or prove
the E0 package is valid. For a real run, use the command above against the
local package.
