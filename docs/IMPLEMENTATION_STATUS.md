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
- U13-A / Issue #43: opaque, verified Scout-candidate admission. Candidate
  batches are canonical and atomic; durable reload validates member and batch
  commitments; downstream code receives a read-only capability rather than a
  forgeable draft.

## In progress in the cumulative branch

- U30 / Issue #41: deterministic platform sensor. It consumes the U29 safe
  projection and emits only sealed, mapping-resolution-bound signals; it never
  reconstructs observation batches or coverage.
- U14: independent verifier. It accepts only the U13-A opaque capability and
  emits a provenance-bound supported/refuted/uncertain report; persistent
  reports and U11 Jev-adapter wiring remain later dependent work.

## Delivery discipline

Changes enter this branch only after their own RED/GREEN evidence and an
independent adversarial review. The branch is not evidence of release or merge
readiness. See `docs/gaps/OPEN_GAPS.md` for dependencies that require a human
owner; no such gap is currently recorded.
