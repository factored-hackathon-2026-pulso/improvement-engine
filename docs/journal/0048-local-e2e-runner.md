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
  source sentinel IDs/labels are absent.
- The actual original-bank partition tree (1,097 CSV files, about 140 MB total)
  was passed to the local CLI. It returned `unsupported_source`, no signal,
  zero candidates and formal `do_nothing`, with a source manifest/snapshot and
  run timeline persisted. File count/size and the sanitized result summary were
  inspected; source rows were not printed. This proves ingestion/provenance
  only, not discovery on original-bank business data.
- Still required before this vertical is ready: run the actual current E0
  package after adapter schema/cutoff/allowlist fixes; run full workspace tests,
  clippy and formatting; record the sanitized terminal status from both actual
  sample runs. The source adapter's current first integration attempt exposed
  LargeUtf8 identifiers and JSON-valued `state_change`; these are treated as
  contract/schema alignment work, not worked around by reading arbitrary
  values.
