# 0489 — DGATE: paired fixture safety and C-10 envelope

## Scope

`combine_gate_results` reduces the safety status from a `PairedEvaluationReceipt`
and one independently evaluated improvement status into the checked-in C-10
wire shape. The safety status and its reason code are derived from the sealed
pair verdict; a caller-supplied safety status or reason that disagrees with
that verdict is rejected.
Both required gates must be present exactly once. A failed gate dominates an
unevaluable gate; otherwise an unevaluable gate prevents a pass. The result
always serializes `quality_claims` as `forbidden`.

The output references are deterministic `fixture_bundle` aliases derived from
the pair's artifact digests, not Agent Core registry readbacks. The `run_id`
is derived from the pair's opaque evaluation digest. The gate reason is a
closed enum, not free text, so this envelope cannot carry customer text.

## Trust and product boundary

This is a deterministic fixture-level reducer, not a production release gate.
`PairedEvaluationReceipt` commits to caller-supplied minimized fixture
observations; it does not authenticate an executor, prove sandbox isolation or
execution, or measure business lift. A resulting C-10 `pass` therefore says
only that the paired fixture found no regression and the supplied improvement
gate passed. It must not be promoted into an Agent Core release decision.
`quality_claims` remains forbidden. Improvement reason codes are also checked
against their status; free text is never accepted.

The reducer consumes the checked-in seven-part engine-steps pack as it exists
on `main`; local `contracts/engine-steps/pack/verify_pack.py` returned
`sha256:e6136a19f16cbadd97d21905dc3823da8a59a21327fb9c4aeb387175b44c897e`,
matching the `main` manifest fetched from GitHub. No contract/schema/fixture
was changed. The shared journal did not yet contain a `CONTRACT-PUBLISHED`
receipt at implementation time; Codex requested that the contract publisher
confirm the journal pointer/digest before claiming the CF0 administrative
receipt complete.

## TDD and review evidence

- RED: added integration tests first; the focused Rust target failed because
  `GateKind`, `GateObservation`, and `combine_gate_results` did not exist.
- GREEN: implemented the reducer; `cargo +1.98.1 test -p
  improvement-engine-core --test paired_scenario --all-features -j1` passed
  23 tests, including C-10 shape, missing/duplicate gates, candidate
  regression, infrastructure failure, `not_evaluable`, invalid metadata,
  same-artifact rejection, and bounded reason-code serialization.
- `cargo +1.98.1 fmt --all -- --check` and `git diff --check` passed after
  formatting.
- Independent adversarial review found that the first draft could report
  `pass` independently of a paired regression. The API was changed to bind
  safety status to the integrity-checked pair receipt; regression and failed
  infrastructure now fail closed / become not-evaluable. The reviewer also
  prompted rejection of same-artifact comparisons and removal of free-text
  reasons. A second review found a mismatched gate-reason/status edge case;
  safety reasons are now paired-verdict-derived, and improvement reason codes
  must be consistent with their status.
- Full `$env:CARGO_BUILD_JOBS=1; .\scripts\verify-local-ci.ps1` passed on the
  final content tree: pinned toolchain, formatting, workspace Clippy, 202 core
  unit tests (2 opt-in PostgreSQL tests ignored), 4 adapter unit tests, Rust
  integration/CLI/source suites, 57 core doctests, 16 Python tests (1
  container-opt-in skip), artifact fixtures, and 25 Pester tests.
- PostgreSQL destructive regressions remain unrun because no isolated DB and
  explicit destructive-test consent are configured. No database, container,
  or production execution is claimed by this slice.
