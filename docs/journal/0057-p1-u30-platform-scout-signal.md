# P1 U30 platform Scout signal candidate

## Goal and boundary

This slice introduces a source-specific bridge from an already-measured,
governed U30 platform signal to a durable platform signal/evidence packet. It preserves
the complete U30 evidence lineage (metric/window/counts, tenant, source and
contract, batch, coverage, mapping, mapping-resolution, projection, signal and
input commitments), plus the Core/model attempt and receipt digests. It stores
the result as an append-only `ArtifactKind::Signal` revision and verifies
readback through the artifact repository.

The artifact is explicitly marked
`not_eligible_for_U13A_U14_requires_platform_independent_verifier`. It is not an
LLM-authored hypothesis, generic U13-A candidate, U14-verified candidate, Opportunity, Proposal, release
instruction, causal conclusion, or measured business lift. It does not invent
U08 source-snapshot/query receipts, U12 source validation, or a list of
platform source-run IDs that the current U30 projection does not provide.
`core_run_id` is the Agent Core task execution ID, not a source-system run ID.

U30 currently carries a tenant and source-batch evidence receipts but no
job/grant/authority binding or U08 snapshot identity. Core/Pulso invocation
authority is separately bound by `CoreTaskScope`, the projection broker, and
matching Core/model receipts; this does not claim that U30 itself authenticated
the job, grant, or authority. The current Core and model receipts expose only
execution IDs and output digests, not structured finding content. This slice
therefore persists an evidence packet and execution lineage only; it does not
persist raw model output or claim to contain an LLM-authored hypothesis. The
exact next seam is a platform-specific independent-verifier contract that
consumes this immutable Signal packet, validates source-batch provenance and
separately authenticated invocation scope, then produces a bounded structured
finding with its own receipt/digest. Only after that consumer contract exists
can substantive finding text be governed and evaluated; it must not be routed
into U13-A/U14 without an explicit bridge.

The local CoreTask binding is currently pinned to Agent Core 0.5.0 by the
existing test contract. Canonical V3 expects 1.3.0; this slice does not silently
change the shared pin. Compatibility with the V3 pin remains a separate gate.

## Safety and persistence behavior

- Invalid input commitment and tenant mismatch fail before projection egress.
- The model prompt receives only metric metadata and aggregates, not raw event
  data, customer content, or opaque tenant/grant identifiers.
- Core/model receipts must match invocation scope, attempt, binding/policy and
  input commitments. Unknown or unsuccessful Core/model outcomes return a
  dependency-blocked disposition, never a candidate.
- The artifact payload carries full U30 provenance and its integrity digests;
  `source_snapshot_ref` is intentionally absent rather than fabricated.
- Identical append retries return the existing immutable revision. A CAS
  conflict is read back only to accept an exact existing payload; storage
  errors are returned. Recovery from an ambiguous storage result requires the
  caller to retry with the same stable artifact ID. Reusing an artifact ID
  with different content fails closed.

## Tests and current validation

The public integration contract is in `crates/core/tests/platform_scout.rs`:
measured U30 input produces only a bounded evidence packet, immutable Signal
append/readback retains evidence receipts and the explicit eligibility
boundary, identical retry is idempotent, cross-tenant scope is rejected before
projection egress, and an unknown Core result blocks candidate creation.
An internal test mutates a committed input and verifies rejection occurs
before projection authorization. The original RED test failed to compile on
the missing platform Scout boundary; the initial GREEN focused integration run
passed 1/1 before adding these regressions. Latest focused outcomes:
`cargo +1.98.1 test --locked --offline -p improvement-engine-core --features
test-support platform_discovery::tests::tampered_platform_input_is_rejected_before_egress_projection`
passed (1 test; 136 filtered); `cargo +1.98.1 test --locked --offline -p
improvement-engine-core --features test-support --test platform_scout` passed
(3/3); `cargo +1.98.1 fmt --all -- --check` and `git diff --check` passed.

The exact focused commands were:

```powershell
cargo +1.98.1 fmt --all -- --check
cargo +1.98.1 test --locked --offline -p improvement-engine-core --features test-support platform_discovery::tests::tampered_platform_input_is_rejected_before_egress_projection
cargo +1.98.1 test --locked --offline -p improvement-engine-core --features test-support --test platform_scout
```

Full local CI, integration with the durable PostgreSQL adapter, independent
adversarial review, and confirmation against the current V3/Core 1.3.0 contract
remain outstanding. No provider call is made by these tests.

## Run-grain correction (2026-10-03)

An adversarial review found that the first sensor draft counted distinct
`source_run_ref` values in its numerator but accepted a caller-selected
`tree_goals` denominator. This did not satisfy V3 §31.6.3 and could make a
measured ratio compare different entities. The sensor contract is now fixed to
`attention_run_handoff_rate` with `population_ref=attention_source_runs`;
the trusted source contract must attest that `expected_population` is the
number of distinct eligible runs in the complete window. Goal-grain coverage
(`tree_goals`) is insufficient and cannot be requested as a sensor spec until
a validated `goal_ref` and matching goal denominator exist.

The metric spec is serialized into the trusted mapping digest, and the exact
population reference is also present in the signal commitment. Coverage
selection filters for the fixed population before accepting a bound
denominator. The public platform-scout regression constructs both complete
run-grain and complete goal-grain source batches: only the former can produce a
measured signal or discovery input. This signal remains evidence-only, not an
opportunity/proposal and not proof of causal impact.

Focused integration validation for this correction passed:
`cargo +1.98.1 test --locked --offline -p improvement-engine-core --features
test-support --test platform_sensor` (12/12) and the equivalent
`--test platform_scout` (4/4). `cargo +1.98.1 fmt --all -- --check` and
`git diff --check` also passed. The dedicated library test
`cargo +1.98.1 test --locked --offline -p improvement-engine-core --features
test-support --lib platform_sensor::tests::trusted_mapping_digest_commits_the_fixed_run_population_grain`
passed (1/1). Full local CI remains deferred while another lane owns the build
slot; durable PostgreSQL integration and independent adversarial review also
remain outstanding.

## Final independent review and local CI (2026-10-03)

P4 independently reviewed the corrected P1 seam and reported no blocking
findings. The source-adapter requirement remains explicit: its trusted
contract must attest that `expected_population` counts distinct eligible
runs; U29 cannot infer that population from observed events alone.

Clippy flagged `PlatformScoutResult` because the inline candidate payload was
large. The candidate is now boxed, preserving the public result behavior; the
existing public integration scenario still validates candidate fields, digest,
immutable artifact readback, and idempotent append. `platform_scout` is
registered as a `test-support`-gated integration target because it uses the
test-only projection broker. Clippy passed with `-D warnings`.

The first full local CI attempt under default Cargo parallelism stalled during
integration-test compilation before any tests started. After several stable
process checks, the exact run was interrupted at the orchestrator's direction;
no individual child was killed and no build artifacts were cleaned. The full
preflight then passed under PowerShell 7 with process-local
`CARGO_BUILD_JOBS=2`: pinned toolchain, formatting, Clippy, Rust unit tests,
Rust integration tests and doc tests, Python contract tests, fixture
validation, and Pester. Rust unit results were core 138 passed / 0 failed / 1
ignored and source-adapters 2 passed / 0 failed. Python contract tests were 11
passed with one container-backend test skipped by its explicit opt-in
requirement. Pester passed both groups: 10/10 E0 wrapper tests and 5/5 local-CI
runner tests. The preflight ended with `Local CI preflight passed for the
selected gates.` Durable PostgreSQL opt-in gates were not selected.

## Deterministic evidence explanation read model — mainline port (2026-10-03)

Starting from GitHub-verified `origin/main` at `0b0c919ddbc517faceda4a269af944f7d75a0c12`, the P1 explanation read model was confirmed absent and ported as a narrow additive API. The previous P1 worktree contained unrelated branch-base differences, so none of its history was merged or cherry-picked.

`PlatformScoutCandidate::explanation()` returns a serializable, deterministic read model containing the measured numerator/denominator and `attention_source_runs` population, mapped layer and metric version, exact window/as-of, source contract, evidence digests, and existing eligibility boundary. Its display sentence states eligible attention-platform source runs and the rounded percentage; the coverage note distinguishes `missing=0` from a count of failed or lost handoffs. The limitation disclaims cause, customer outcome, and business value. The payload excludes tenant/customer content and model free text; it creates no new artifact or eligibility.

TDD RED on the verified mainline was the public `platform_scout` test failing to compile because `PlatformScoutCandidate` had no `explanation()` method (`E0599`). After the minimal additive read model was implemented, the focused scenario passed. Local Windows/Rust 1.98.1 verification: `platform_scout` 4/4, `platform_sensor` 12/12, Core `test-support --all-targets` Clippy with `-D warnings`, `cargo fmt --all -- --check`, and `git diff --check`. Workspace-wide CI was not run. This remains descriptive evidence only.

## Independent review result (2026-10-03)

The read-only adversarial reviewer found no blockers and accepted the slice for a
consolidated PR. Review confirmed same-grain eligible-run denominator, coverage
wording, and explicit non-causal/no-business-impact claims. One non-blocking
suggestion was to add further rounding and percentage-boundary tests; the
reviewer did not execute Cargo. Current public-path tests verify the measured
1/4 case and its 25.00% rendering. No production or business-lift claim is made.

The suggested math coverage was added without production changes:
`percentage_basis_points` tests cover 0%, 100%, one-third/two-thirds rounding,
and large `u64` counts using a wide intermediate. Windows/Rust 1.98.1 validation
passed: the focused math tests (2/2), `platform_scout` (4/4), test-support
all-target Clippy with `-D warnings`, formatting and diff checks.

The follow-up test-only hardening is commit
`9208d8c41ef43596e085c95953f3c8c63b35e2e2` on the same feature branch. Since
worktree creation, GitHub `main` advanced from the verified base
`0b0c919ddbc517faceda4a269af944f7d75a0c12` to
`594be4e4525884c80f9baaee924ffd7c6b41b3e6`; the feature branch is intentionally
left unrebased for selective consolidation and must not be opened as a
stale-history PR.
