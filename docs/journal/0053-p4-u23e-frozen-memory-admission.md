# P4 U23-E Frozen memory admission

## Slice

Added a crate-private U23-E admission seam over the existing U33-E Frozen
publication state. It does not create a second memory registry or ledger and
does not route through generic U22/U33 publication. Admission re-attests the
exact immutable publication reference and current head, tenant/world/scope,
Frozen replay cutoff, current grant revision, and revocation state before
recording a payload-free use receipt in the same U33-E state.

The receipt key is the semantic tuple
`(publication_commitment, scope, run_id)`. Exact retries return the canonical
receipt. A different valid grant or access timestamp for the same tuple is a
conflict and cannot create a second receipt. A changed access snapshot,
publication revision, head, revoked grant/snapshot, scope drift, or use after
cutoff fails closed.

The durable U33-E adapter deliberately returns `DependencyUnavailable`. It
must remain unavailable until one PostgreSQL transaction can check the exact
publication/head, U05 grant revision/liveness and revocation, then write the
canonical U23-E receipt. This slice is not yet selected by the local CLI or a
production runtime; it establishes the sealed Rust contract and in-memory
semantic adapter only.

## TDD and verification

The changed-grant regression was run against the earlier identity that
included grant/time and failed as expected: a second valid grant created a new
receipt instead of returning a semantic conflict. The implementation changed
the identity to the publication/scope/run tuple while retaining grant/time in
the receipt's temporal commitment.

Focused U23-E tests:

```text
cargo +1.98.1 test --locked --offline --target-dir target-u23e-cycle -p improvement-engine-core --lib --features test-support e0_frozen_memory_publication::tests::u23e_
```

Result after the fix: `11 passed; 0 failed`.

Core unit suite:

```text
cargo +1.98.1 test --locked --offline --target-dir target-u23e-cycle -p improvement-engine-core --lib --features test-support
```

Result: `122 passed; 0 failed`.

Clippy:

```text
cargo +1.98.1 clippy --locked --offline --target-dir target-u23e-cycle -p improvement-engine-core --all-targets -- -D warnings
```

Result: passed. `rustfmt --edition 2024` and `git diff --check` also pass.

## Review disposition

An adversarial P4 review found that the original receipt ID included the grant,
revision and timestamp, permitting multiple records for one logical run. The
fix and three regressions cover changed valid grant, changed valid timestamp,
and wrong access snapshot. Independent review of the corrected semantic slice
returned GO. The focused U23-E suite and Clippy were rerun before the local
commit; this is not a merge or durable PostgreSQL approval. Durable admission
remains dependency-blocked pending a real single-transaction adapter test.
