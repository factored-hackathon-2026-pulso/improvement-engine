# Local E2E runner — executable composition and boundaries

## Decision

The first executable composes the existing core behind an explicit
`local-simulation` mode; it does not emulate a native Agent Core runtime or use
`test-support` APIs. The runner package is the composition root for filesystem
inputs and run persistence. The core facade owns the constrained local
detection-to-draft simulation, and source-adapters owns immutable source
preparation. Core does not depend on the adapter crate.

For E0, the runner receives only `PreparedSource::agent_inputs()` and safe
per-case event projections. It never invokes the separate evaluator/labels
loader. Run output records source manifest/snapshot identity, simulation
version/seed/determinism, detection metric, candidate admissions, uncertain
verification, an explicitly unverified/non-executable proposal, and a
structural-only simulated evaluation. Formal route remains `do_nothing`; no
business lift or release eligibility is claimed. The Core/model/verification
ports used in this mode are locally scripted and make no provider or network
request.

The detector retains case ordinals independently of event rows so cases with
no supported event are included in the denominator as missing rather than
silently disappearing. The current technical-error-rate metric is available
only for the E0 history projection. The original-bank CSV adapter currently
seals a recursive manifest but has no approved typed event projection, so the
CLI must return `unsupported_source`, never a successful empty discovery.
Next source slice: safely project documented call-center contact fields and
add a reviewed, explicit signal such as repeated contact/resolution/SLA; do not
infer technical errors from unrelated bank tables.

The E0 adapter follows the shipped `platform_history.json` contract and the
actual Parquet physical encodings without widening the agent-visible data:
identity-check `correct` is an integer count; all-null optional Arrow `Null`
columns become absent values; optional list fields accept typed `List` /
`LargeList` or bounded JSON arrays encoded as `Utf8` / `LargeUtf8` (16 KiB,
128 items, 256 bytes per item), and every item is immediately domain-projected
or hashed before entering `AgentInputSet`. Unsupported encodings fail closed.
Evaluator outcomes remain structurally separate. Approval-to-tool lineage is
represented causally on the later tool-call event as a child of its prior
approval, using event-kind-scoped source ordinals so per-table ordinal
collisions cannot mislink the timeline. Decisions with missing timestamps or
timestamps after the observation cutoff are redacted.

Discovery consumes only the source adapter's `Arranque` cases. `Reproduccion`
cases are counted as explicitly excluded in the run result and are not mapped
into the discovery input. The package manifest commits the complete allowlisted
discovery-source snapshot and platform contract, not evaluator-only
`labels.parquet`, `timeline.parquet`, or `case_close.parquet`. A changed replay
row can alter the manifest and provenance-derived identifiers but must not
change discovery metric values, hypothesis text, or candidate semantics. The
binary regression test changes only a replay event and asserts those semantic
outputs remain identical. The exact configured UTC cutoff is persisted in the
result and every timeline event, alongside the source manifest commitment.

The runner does not fabricate opportunity drafts when the discovery metric has
no positive support. A zero numerator or zero measured denominator ends with
`complete_no_opportunity`, an explicit `scout/no_opportunity` timeline event,
no candidates, and no proposal/evaluation. Positive support follows the
simulated candidate/verifier/draft path; formal route remains `do_nothing`.

## CLI and persistence

Example:

```powershell
cargo +1.98.1 run --offline --target-dir target-e2e -p improvement-engine-runner -- `
  local-sim --mode local-simulation --source e0 `
  --input D:\.codex\factored\pulso_muestra_e0 `
  --output output\local-runs --tenant-id pulso_local `
  --observed-cutoff 2026-10-02T18:00:00Z --arranque-cases 200
```

Each run gets a generated identity and a new output directory containing
`result.json` and `events.ndjson`. The runner writes both to a staging directory
using create-new files, flushes them, then renames the directory into place;
existing final/staging directories are never overwritten. No source row,
customer/case identifier, transcript, holdout label, or evaluator outcome is
written to the run output.

## Verification and remaining work

- Core local-simulation tests prove the E0 descriptive metric, eventless-case
  missingness, candidate flow, uncertain verifier gate, unverified draft and
  explicit unsupported source behavior.
- Runner unit tests cover explicit local-mode opt-in, required cutoff, safe-code
  rejection, atomic result/timeline persistence and no-overwrite behavior.
- A binary integration test constructs a synthetic E0 Parquet package, starts
  the actual executable, checks persisted simulated outputs, and verifies that
  source sentinel IDs/labels are absent. Replay-only changes are also tested
  not to affect discovery signal, hypothesis or candidate semantics.
- The actual original-bank partition tree (1,097 CSV files, about 140 MB total)
  was passed to the local CLI. It returned `unsupported_source`, no signal,
  zero candidates and formal `do_nothing`, with a source manifest/snapshot and
  run timeline persisted. File count/size and the sanitized result summary were
  inspected; source rows were not printed. This proves ingestion/provenance
  only, not discovery on original-bank business data.
- Final actual E0 smoke is the only smoke evidence for this slice: 200 Arranque
  discovery cases, 1,800 Reproduccion cases excluded, technical-error signal
  denominator 187 / numerator 0 / missing 13, and terminal status
  `complete_no_opportunity`. It emits no candidates, verifier, proposal, or
  evaluation; formal route is `do_nothing`, and five timeline events are
  persisted. The exact observed cutoff `2026-10-02T18:00:00Z` is persisted in
  the result and all five events. Sanitized output inspection checks that
  known PII sentinels and evaluator-label fields are absent. This does not prove
  native Agent Core, holdout evaluation, causal lift, release, or production
  behavior. Earlier local output that emitted candidates from this
  zero-positive metric is obsolete and is not evidence for the final behavior.
- Source-adapter tests: 6 passed. Runner tests: 4 unit and 2 binary integration
  passed. Core local-simulation tests: 3 passed. `cargo fmt --all -- --check`
  and `git diff --check` passed. Full workspace tests, workspace clippy, and a
  post-change original-bank CLI rerun remain to be performed.
