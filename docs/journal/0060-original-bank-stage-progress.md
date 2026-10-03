# OriginalBank source-preparation progress

## Scope

Add privacy-safe stage visibility to the local OriginalBank E2E path. The
source adapter reports aggregate inventory, manifest-scan, and contact-
projection progress; the runner emits the existing opt-in JSONL progress
channel; the Windows PowerShell wrapper validates and renders only approved
aggregate progress. This does not alter discovery, source projection,
manifest serialization, source digest, or result semantics.

Each stage emits a start and terminal record and at most 100 deterministic
intermediate file-boundary updates. Intermediate cadence is derived from the
number of eligible CSV files with a ceiling stride; terminal events always
report exact completed/total files and byte sizes. Errors from inventory,
manifest parsing, and contact projection produce one sanitized stage-level
`failed` record before the error is propagated. A single large CSV has no
within-file heartbeat; its byte count advances when that file completes.

Progress fields are schema version, fixed event/phase/stage/status values,
aggregate file/byte counts, and elapsed milliseconds. The wrapper rejects
unknown values and drops all other Cargo output; it never displays paths,
table names, row/customer identifiers, source hashes, source content, proposal
content, or raw diagnostics. Synthetic fixtures exercise both manifest and
contact-stage failures. No bank data or provider was used by the tests.

## TDD and verification

- Initial source-progress CLI regression RED: OriginalBank emitted no
  `inventory`, `manifest_scan`, or `contact_projection` events.
- After the first implementation, the focused CLI regression passed 1/1 and
  confirmed the manifest and snapshot digests were identical with progress on
  and off.
- Event-volume RED: the bounded-progress test failed against an emit-every-file
  mutation with 208 events for 205 CSVs (expected at most 102 including start
  and terminal records).
- GREEN: `CARGO_BUILD_JOBS=2`, isolated `CARGO_TARGET_DIR=target-stage-progress`,
  `cargo +1.98.1 test --locked --offline -p improvement-engine-runner --test cli_e2e original_cli_`
  passed 4/4; includes bounded manifest/contact progress, exact totals,
  unchanged digest, and exactly one failed event for malformed manifest and
  contact-projection fixtures.
- `pwsh -NoProfile -Command 'Invoke-Pester -Path ./tests/run-local-e0-e2e.Tests.ps1 -PassThru | Format-List *'`
  passed 20/20. The fake Cargo emits approved progress plus a private sentinel;
  only safe aggregate progress is visible.
- `rustfmt +1.98.1 --edition 2024 --check crates/source-adapters/src/lib.rs crates/runner/src/main.rs crates/runner/tests/cli_e2e.rs`
  and `git diff --check` passed.

Full local CI and a fresh real-data E0+OriginalBank run were not run in this
slice; Cargo work was serialized with the parallel P2/P4 lanes. The focused
run verifies the source-preparation progress contract, not end-to-end runtime
or discovery quality. Independent adversarial review remains pending.

## Adversarial follow-up

Root review found the contact byte-size preflight could return before the
projection wrapper, leaving no terminal event for that stage. The stage now
starts before the preflight and catches that error into one `failed` event. A
deterministic adapter regression removes the contact
file at the manifest-completed callback, forcing the metadata read to fail.
The pre-fix RED was captured in session `20666`: 1 test failed as expected
because the contact stage had no `started` event. After the fix, session
`58423` passed the source-adapter library suite 3/3 and the OriginalBank CLI
suite 4/4. The wrapper Pester suite passed 20/20 after pending-size display was
added; Rustfmt check and `git diff --check` pass. `started` records use
zero byte totals as initialization placeholders, not measured zero source
size; the wrapper renders these as `size pending`. If stderr writes fail, the
observer records the first error while the current synchronous adapter stage
may finish, then the CLI aborts before detection or persistence.

The root adversarial review found no remaining blocker. Full local CI and a
fresh real-data E0+OriginalBank progress smoke have not yet run; both remain
gated on the serialized Cargo queue. No commit was pushed and no PR was opened.
