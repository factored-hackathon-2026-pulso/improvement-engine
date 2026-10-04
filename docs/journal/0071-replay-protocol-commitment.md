# Replay protocol and schedule commitment

## Scope

Extended the bounded replay clock kernel with explicit `Frozen` and
`Prequential` protocol identities and a versioned SHA-256 schedule commitment.
The canonical encoding covers the complete supplied case/event clock inputs,
the selected availability profile and protocol, and the resulting ordered
cohorts with cumulative visible-event IDs. The digest uses a domain tag,
fixed-width big-endian timestamps, length-prefixed UTF-8 IDs, explicit enum
bytes, and collection counts. Input permutation does not affect the schedule
or digest; changing a protocol, availability profile, temporal input or
resulting visibility changes it.

The protocol value and commitment do not execute the campaign. The module does
not freeze/mutate candidate state, score outcomes, read labels, persist a run
manifest/cursor, or verify holdout integrity. `schedule_digest` is specifically
a commitment to the inputs and computed temporal schedule—not a full run
manifest: source snapshot bytes/digest, contracts, transforms/allowlist,
detector/config, fields read, partition membership and counts remain outside
this kernel.

## Plan/spec evidence correction

The plan v5 DREPLAY row says “goldens from FRZ0,” but FRZ0 has no temporal
replay goldens; its C-7 traces concern durable-job claim semantics. This slice
uses inline, deterministic protocol-derived golden tests from V3 §§28.4 and
28.7.2–28.7.3 and does not attribute them to FRZ0. The implementation plan is
not a source of temporal contracts when it conflicts with canonical V3.

## Tests and review

TDD for the schedule commitment added an intentionally failing inline digest
golden (RED), then encoded and asserted the observed canonical result. Focused
replay-clock tests pass 5/5, covering tied cohorts, cumulative visibility,
inclusive cutoffs, future availability/ingestion exclusion, explicit clock
profile, duplicate/invalid input rejection, ordering, protocol/profile/input
digest sensitivity, and an exact digest golden. Independent adversarial review
first found the digest/output mismatch (P2); code and digest were corrected to
include resulting visibility and re-reviewed with no P1/P2 findings.

This is a partial DREPLAY protocol kernel, not the runnable E0/FRZ0 frozen or
prequential campaign required by V3 §28.7.2–28.7.3. Full local Windows
`scripts/verify-local-ci.ps1` exited 0 with `CARGO_BUILD_JOBS=1`: pinned fmt,
Clippy, 205 Rust core unit tests passed / 2 destructive PostgreSQL tests
ignored, 4 source-adapter unit tests passed, the Rust integration/doc gates
passed, 25 Python contract tests passed / 1 opt-in container test skipped,
fixture validation passed (11/11 families; six wire families remain pending),
and Pester passed 20 + 5 tests. No PostgreSQL or Podman service was started.
GitHub Actions status was not used as validation evidence.
