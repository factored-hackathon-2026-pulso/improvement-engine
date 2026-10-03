# 0052 — Original contact snapshot-volume projection

## Goal and boundary

Enable the local engine to prepare and report a minimal original-bank contact
volume projection without treating naive timestamps as UTC or inventing
outcomes. This is a new source projection, not a replacement for the separate
strict event-time/PQR projector.

The projection reads only the contact reason/category and channel fields from
sealed `call_center_interactions` CSV partitions. It counts records by closed
reason/channel enums, does not retain raw labels, and suppresses cells below a
configurable minimum count (default `k=5`; allowed 5–10,000). Version and
threshold are bound into the preparation manifest. The source-byte digest is
checked again while streaming the selected partition into the projection.
The manifest pass hashes and parses the same byte stream so its row/header
metadata cannot be paired with a digest from a separate read. Counts are CSV
record counts, not guaranteed unique `interaction_id` values: the API names the
metric `record_count`, and that identifier is intentionally not read or used for
deduplication. A blank `reason_category` falls back to a nonblank
`contact_reason`; if both are blank, the result is `unclassified`.

The run is explicitly `snapshot_extract_counts`: it does not inspect dates,
filter records by the run cutoff, calculate recurrence from customer IDs,
measure PQR/SLA or contact outcomes, infer technical errors, or claim a cause.
The local motor exposes the descriptive projection only; it emits no signal,
Scout candidate, or improvement proposal from volume alone. This preserves the
line between a high-volume category and a validated problem/opportunity.
The core boundary independently enforces the versioned `k` range of 5–10,000,
safe category/channel enums, unique groups, and overflow-safe count totals; the
adapter or CLI cannot bypass the small-cell policy.

## Implementation and verification

- TDD RED: the adapter regression could not compile because the public safe
  `contact_volumes()` projection API was absent.
- GREEN: the same test passed with synthetic CSV input, closed category/channel
  values, `k` suppression, and assertions that IDs, raw PII-like reason strings,
  dates, and outcome booleans are absent from serialization.
- TDD RED: configurable `k` test could not compile because
  `with_minimum_contact_cell_count` was absent.
- GREEN: configurable `k=6` suppresses a five-row cell; the effective threshold
  and policy version are represented in the prepared-source output/manifest.
- Added a core local-run contract that returns the snapshot projection as a
  descriptive result with no signal or proposal, then wired the existing CLI
  source mapping for `original` only. The E0 mapping path is unchanged.
- Added fail-closed tests for malformed/truncated rows and duplicate headers.
- TDD RED for the review correction: the public `contact_count`/`included_contact_count`
  names did not expose row-count semantics and the API accessors were absent.
  GREEN renamed the safe public fields to `record_count`/`included_record_count`;
  a synthetic fixture repeats one `interaction_id` across five CSV rows and
  confirms the metric reports five records. Blank `reason_category` now falls
  back to nonblank `contact_reason`; both blank map to `unclassified`.
- TDD RED for review P3: a contact table lacking the required reason field and
  carrying a mismatched sealed digest returned unsupported before verifying its
  bytes. GREEN consumes and validates each CSV stream, verifies its digest, and
  only then returns unsupported with no projection. Safe metric increments and
  included totals use checked arithmetic.
- Added a runner-level synthetic end-to-end test. It includes an input row with
  a naive timestamp after the configured cutoff and asserts that snapshot
  counts still represent the extract, rather than falsely applying the cutoff.

Commands run (Windows / PowerShell):

```powershell
cargo test --locked --offline -p improvement-engine-source-adapters unsupported_contact_schema_still_verifies_source_digest
cargo test --locked --offline -p improvement-engine-source-adapters --test source_adapters original_contact_counts_are_explicit_record_counts_and_blank_primary_reason_falls_back
cargo test --locked --offline -p improvement-engine-source-adapters
cargo test --locked --offline -p improvement-engine-core --features local-simulation --test local_simulation
cargo test --locked --offline -p improvement-engine-runner
cargo +1.98.1 fmt --all
git diff --check
```

Post-rebase to `4b4079891bee2b854f328c66af11fdd83b3f967f`, the focused P3 test
passed; source-adapters passed 2 unit + 6 holdout + 9 package validation + 11
source adapter tests, core local simulation passed 9 tests, runner passed 7 unit
+ 5 CLI E2E tests, formatting and diff check passed. The exact suites are being
rerun once more after this journal update so evidence matches the final diff.
No historical source data was added or used; no real-data smoke was run.
GitHub Issue creation was attempted but unavailable in this environment
(GitHub API returned 404 for the private repository); the slice remains linked
to its explicit acceptance criteria in the task journal until the repository
connection is restored.

## Windows CI privacy-assertion false positive

PR #65 Windows CI failed at the serialized-output privacy assertion. The
assertion searched for very short fragments (`c-1`, `a-1`) anywhere in the
whole JSON string. `PreparedSource` legitimately serializes its generated
UUIDv7 snapshot reference; UUID hyphen boundaries can contain those fragments
even when source customer/agent IDs have been removed. This is a test false
positive, not evidence that a source identifier was emitted. The same generic
substring approach for `true` could match unrelated serialized content.

The regression fixture now uses long, unique PII sentinels for interaction,
customer and agent IDs and searches for every sentinel across all seven rows,
plus every timestamp (including the post-cutoff row), source email and raw
category strings. It checks sensitive ID, outcome and operational columns by
parsed JSON key names instead of banning generic substrings. No production
serialization behavior was changed. The post-fix focused Windows command
`cargo +1.98.1 test --locked -p
improvement-engine-source-adapters --test source_adapters
original_contacts_expose_only_suppressed_snapshot_counts_by_safe_categories`
passed twice after the test-only correction (1 passed, 10 filtered each run).
`git diff --check` also passed; the complete source-adapter suite was not rerun.

## Follow-up / risks

- Current output is descriptive, not a detector of abnormality: the projection
  has no historical baseline and intentionally does not label large counts as
  problems. Do not feed it to a proposal/Agent Core artifact builder without a
  separately specified and tested opportunity policy.
- Repeat-contact analysis needs an approved privacy-preserving identity join;
  PQR requires a canonical source contract and reliable temporal semantics.
- No real-data values or prevalence claims are made from the synthetic tests.
