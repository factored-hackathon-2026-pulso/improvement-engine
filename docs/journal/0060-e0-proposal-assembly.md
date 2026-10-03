# 0060 — E0 proposal assembly boundary

Status: focused validation green; full local CI and runner integration remain pending.

- Added a core-level deterministic assembler that fans out every qualifying
  E0 portfolio signal into a descriptive candidate seed; runner wiring is
  intentionally out of scope for this slice.
- Candidate records carry run/snapshot/cutoff, signal digest, metric, and
  detector policy provenance. They do not duplicate metric values or claim
  causal effect, business lift, route readiness, evaluation, or native Agent
  Core connectivity.
- Input validation requires closed E0 metric coverage, provenance/digest
  correspondence, finite disposition reason codes, policy-code/version
  validity, and consistent portfolio status. Recurrence unavailability is
  explicit and digest-free; empty portfolios remain insufficient evidence.
- Candidate states are recomputed against metric numerator/denominator,
  missing observations and recurring support floor; a fabricated candidate
  disposition cannot qualify a zero-denominator or zero-positive measurement.
  The run envelope and copied IDs/digests/cutoff are validated before output.
- Added an unkeyed canonical summary commitment over every public summary
  field; the assembler recomputes it and rejects stale field mutation. It is
  integrity/staleness detection only, not authentication or signed evidence.
  Coverage is recomputed with checked denominator+missing and checked scaling.
- Independent review also required privacy-safe identifier/digest/cutoff
  validation and acceptance of the runner's legitimate completed no-op status.
  Failed or non-local run envelopes are rejected.
- TDD evidence: removing candidate ordering made the permutation test fail;
  changing measured numerator/coverage while retaining the previous summary
  commitment was also observed to assemble a candidate before commitment
  verification was added. An intermediate 7/8 focused run exposed the
  unavailable-versus-measured metric reconciliation bug; its fix was verified.
- Final focused command `cargo +1.98.1 test --locked --offline -p
  improvement-engine-core --features local-simulation --lib
  e0_proposal_assembly::tests` passed 12/12 (exit 0), using isolated
  `target-local-p2-proposal-assembly` and `CARGO_BUILD_JOBS=2`. Direct
  `rustfmt --check` and `git diff --check` are clean. Full local CI remains
  pending root's serialized gate; independent review found no blocker, with
  the unkeyed commitment boundary explicitly documented.
- Files owned by this slice: `crates/core/src/e0_proposal_assembly.rs`,
  `crates/core/src/lib.rs`, this contract note, and this journal entry.
