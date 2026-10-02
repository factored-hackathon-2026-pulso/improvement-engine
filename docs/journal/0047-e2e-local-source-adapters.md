# E2E local source adapters

## Objective

Add a reusable, local-only source boundary for the original bank CSV dataset
and the augmented E0 Parquet package. Keep source data out of Git and prevent
E0 evaluator labels from entering discovery inputs.

## Design

- `crates/source-adapters` owns source preparation and immutable snapshot
  identity; it is a library crate in the existing Rust workspace.
- Original CSV files are parsed with the CSV standard (including quoted
  delimiters/newlines), counted and hashed byte-for-byte. No original row
  values are copied into the prepared agent input.
- E0 Parquet is read in Rust through Apache Arrow/Parquet. Only the allowlisted
  interaction tables and fields are projected; categories with a declared
  domain are allowlisted, unknown values become `unknown`, and open-vocabulary
  identifiers are projected as stable SHA-256 category fingerprints. Free text,
  source identifiers, params, `timeline`, `case_close`, and `labels` are excluded
  from the discovery projection.
- The observed cutoff is applied to case-open time before chronological split,
  then to every event at microsecond precision. Future approvals are omitted;
  future decisions on otherwise observable approvals are redacted.
- `tool_call.state_change` is JSON in the E0 contract, so the adapter exposes
  only whether a JSON value was present, never its contents.
- Evaluator labels require a separate explicit `evaluator::load_labels` call
  and return an `EvaluatorOnlyLabels` type that is not part of `PreparedSource`.
- Each prepared source contains a manifest digest and a content-addressed
  `ArtifactReference`; local input paths are not serialized.
- Local source reads are sufficient. LocalStack/MinIO can be used by a later
  composition layer but are not required by this adapter.

## API boundary

`prepare_original_bank` / `prepare_e0_package` take a source path and
`PreparationConfig`. The resulting `PreparedSource` exposes source kind,
snapshot reference, manifest digest, cutoff, and an allowlisted `AgentInputSet`.
E0 interaction facts are ordinal-linked and typed. The original dataset does
not provide an approved technical-error field, so that metric is explicitly
marked unsupported instead of imputed as zero.

## Verification

Synthetic CSV and Parquet fixtures exercise quoted CSV records, source hash
changes, chronological E0 case ordering, injected PII-like category masking,
same-second post-cutoff event exclusion, safe JSON state-change projection, and
the evaluator-only label boundary.

```text
cargo +1.98.1 fmt --all
cargo +1.98.1 test --locked --offline -p improvement-engine-source-adapters --test source_adapters --target-dir target-source-adapters
```

Result: 6 passed, 0 failed. The private target directory is ignored through
`.gitignore`; no source package data is committed.

## Follow-up

The source adapter intentionally does not implement sensor execution,
opportunity qualification, LLM access, evaluator policy, or object storage.
Those belong to the E2E runner and downstream engine modules.
