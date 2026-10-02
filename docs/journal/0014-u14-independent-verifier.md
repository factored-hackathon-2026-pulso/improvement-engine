# U14 — independent verifier

## Slice

U14 accepts only the opaque `VerifiedScoutCandidate` admitted by U13-A. It
derives an immutable input commitment over the candidate digest, U13
provenance commitment, source snapshot and complete tenant/job/grant/authority
scope. An independent checker receives that treated view and returns a typed
`supported`, `refuted` or `uncertain` receipt. The verifier rejects a receipt
whose input commitment belongs to a different candidate or scope.

The output is a `VerificationReport`, not an opportunity or proposal: it
cannot author, select, publish or infer causality for an Agent Core artifact.

## TDD evidence

RED (correct failure):

```text
cargo +1.98.1 test -p improvement-engine-core --test independent_verifier --features test-support
error[E0583]: file not found for module `independent_verifier`
```

GREEN (focused):

```text
cargo +1.98.1 test -p improvement-engine-core --features test-support independent_verifier
4 U14 unit tests passed; the public receipt contract test passed.
```

## Boundary and follow-up

This cut deliberately does not fabricate an Agent Core or Jev runtime. A
future adapter may call the existing U11 pinned Jev decision port alongside
deterministic evidence checks, but must return this exact input commitment and
preserve the same opaque-candidate boundary. Persistent report storage,
evaluation-plan linkage and proposal eligibility belong to later dependent
slices (U16/U20/U35), rather than a hidden side effect of U14.
