# Cumulative implementation status

This document is the entry point for the long-lived cumulative implementation
PR. It distinguishes merged capability from work in progress; it never treats a
green unit test as proof that a broader product flow is complete.

## Integrated into `main`

- U29: tenant-scoped platform observation ingestion, source-contract
  provenance, truthful coverage and PostgreSQL RLS contract.
- U34: durable, fenced and truthful operator run-control receipts.
- U13: autonomous Scout drafts with sealed source/Core/model provenance.
- U33: scoped immutable published-memory revisions, head CAS and idempotent use
  receipts. Its present in-memory adapter is not production-durability proof;
  the U33 corrective slice covers an overflow atomicity regression.

## Implemented in the cumulative branch (not yet merged)

- U13-A / Issue #43: opaque, verified Scout-candidate admission. Candidate
  batches are canonical and atomic; durable reload validates member and batch
  commitments; downstream code receives a read-only capability rather than a
  forgeable draft.

- U30 / Issue #41: deterministic platform sensor. It consumes the U29 safe
  projection and emits only sealed, mapping-resolution-bound signals; it never
  reconstructs observation batches or coverage.
- U14: independent verifier. It accepts only the U13-A opaque capability and
  emits a provenance-bound supported/refuted/uncertain report; persistent
  reports and U11 Jev-adapter wiring remain later dependent work.
- U16: provisional WorkflowBridge. It accepts only a U14 verification report
  and sealed internal catalogue/source-validation facts, preserves its
  commitments and scope, includes `do_nothing`, and caps a supported route at
  `mechanism_proxy`. U20/U35 still own the sealed outcome/oracle and final
  eligibility gates.
- U36: protected-fixture sandbox identity boundary. A protected arm receives
  an opaque issuer capability only through trusted composition; identities are
  nonce-registered to that arm, checked with the sandbox-owned clock, and
  rejected on revocation, expiry, forging, or cross-arm use. This is fixture
  protection, not a production identity-provider implementation.
- U04-B: E0 replay availability clock. A V2 replay binds its complete,
  tenant-scoped immutable source snapshot and a sealed file manifest before
  access; V1 snapshots cannot be replayed because they cannot prove that
  binding. Serialization loses non-serializable file seals and fails closed.
- U34-F: immutable original-run fork. The only in-memory commit entry point
  owns its grant, lifecycle, artifact-policy, idempotency and audit state; it
  atomically predicates a sealed fence before exposing one child, receipt and
  audit event. The local port models the durable adapter contract but is not a
  deployed durable transaction implementation.
- U20: sealed evaluation plan. Trusted composition attests the exact typed
  baseline, oracle, development-suite and final-suite revisions before it
  seals their scope, snapshot and evaluation semantics for a mechanism proxy.
  It cannot execute a candidate, assert the same outcome, make a proposal or
  release a change.
- U24: read-only technical timeline boundary. The internal debug composition
  derives its tenant only from an authenticated, opaque viewer capability and
  reads U07 activity through safe statuses and accessible summaries. It is not
  yet a browser, HTTP, SSO or streaming-control-plane implementation.
- U35: final eligibility gate. It is a deterministic, side-effect-free check
  over a supported U14 report, a mechanism-proxy U16 bridge and its U20 plan;
  it can only mark a proposal eligible and never asserts an outcome or causes
  execution, registry mutation, release or customer exposure.
- U22: governed published-memory admission. A second run receives only an
  opaque provenance capability after U33 atomically predicates the exact
  request, grant revision, live head and snapshot before recording its receipt.
  No raw pages,
  cache, publication, Scout, runtime or release capability is exposed.
- U17: sealed change compiler. Only an opaque authorization composed from a
  U35 eligible proposal and U16 mechanism proxy can compile an untrusted Flow
  into immutable drafts. Its full scope, route, exact canonical Flow body and
  typed executable write predicate are rechecked; U18 remains responsible for
  atomically evaluating that predicate and writing to the registry. This slice
  neither persists, executes nor releases a candidate.
- U23-P / Issue #47: the narrow Frozen/Continuous temporal protocol over
  U04-B's replay cutoff and U22/U33 governed admission. An opaque, trusted
  evidence commitment (not caller-provided timestamps) is part of the U33
  canonical receipt; Frozen forbids outcome feedback and Continuous requires
  an already-observable, provenanced outcome. This is not the
  full U23 E0 runner, CampaignManifest, scoring, or replay-result flow; those
  remain dependent on U09, U20-E and U27.

## Delivery discipline

Changes enter this branch only after their own RED/GREEN evidence and an
independent adversarial review. The branch is not evidence of release or merge
readiness. See `docs/gaps/OPEN_GAPS.md` for dependencies that require a human
owner; no such gap is currently recorded.
