# U16/U17 cutoff validation and commitment continuity

## Scope

This slice hardens the existing WorkflowBridge-to-compiler path without
changing its authority model or adding persistence, Agent Core execution,
registry writes, evaluations, or release behavior. The U16 `as_of_cutoff`
must now use the same fixed-width UTC timestamp grammar already used by U04-B
(`YYYY-MM-DDTHH:MM:SSZ`). This avoids accepting a date-only or local-time
string whose temporal meaning could differ between source replay and proposal
construction.

The shared validator rejects offsets, fractional seconds, invalid calendar
dates, out-of-range clock fields, leap seconds, and trailing bytes. U16's
canonical bridge commitment already covers its full input, including the
cutoff; U17 carries that exact commitment into the compilation authorization.
The regression now asserts both links of that chain.

## TDD evidence

RED: added
`workflow_bridge_rejects_cutoffs_without_canonical_utc_second_precision`.
Before the implementation change it failed because `2026-09-30` was accepted
as a valid bridge input.

GREEN: reused U04-B's `is_rfc3339_utc` validator as a crate-visible shared
contract, added valid/invalid timestamp cases, verified a cutoff change changes
the sealed bridge commitment, and asserted the compiler retains the exact U16
bridge and U20 plan commitments.

Commands run:

```text
cargo +1.98.1 test -p improvement-engine-core --lib workflow_bridge_rejects_cutoffs_without_canonical_utc_second_precision
cargo +1.98.1 test -p improvement-engine-core --lib workflow_bridge_
cargo +1.98.1 test -p improvement-engine-core --lib eligible_readiness_compiles_one_immutable_draft_without_registry_or_release_effect
cargo +1.98.1 test -p improvement-engine-core
cargo +1.98.1 fmt --all -- --check
cargo +1.98.1 clippy --locked --offline -p improvement-engine-core --all-targets -- -D warnings
git diff --check
```

The first command failed on the intended behavioral assertion; the latter two
targeted commands passed (3 bridge tests and 1 compiler integration test).
The full core crate run passed: 90 unit tests, all applicable integration
tests, and 50 doctests; 3 database/private-data tests were explicitly ignored
by their declared environment/consent guards. Formatting, crate Clippy with
warnings denied, and `git diff --check` passed. The wider workspace suite and
independent adversarial review remain outstanding before integration.

## Limitations

This confirms only the in-process U16/U17 contract chain. It does not prove
source timestamp timezone correctness, real Agent Core schema compatibility,
durable authorization storage, runtime execution, or business outcomes. The
compiler still supports only its existing minimal `Flow add` boundary.
