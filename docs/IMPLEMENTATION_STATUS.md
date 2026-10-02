# Cumulative implementation status

This document is the entry point for the long-lived cumulative implementation
PR. It distinguishes merged capability from work in progress; it never treats a
green unit test as proof that a broader product flow is complete.

## Integrated into `main`

- U29: tenant-scoped platform observation ingestion, source-contract
  provenance, truthful coverage and PostgreSQL RLS contract.
- U34: durable, fenced and truthful operator run-control receipts.
- U13: autonomous Scout drafts with sealed source/Core/model provenance.

## In progress in the cumulative branch

- U13-A / Issue #43: durable admission of a verified Scout candidate for
  downstream consumers. It must close restart, atomic-batch and no-forgery
  invariants before integration.
- U30 / Issue #41: deterministic platform sensor. It consumes the U29 safe
  projection; it must never reconstruct observation batches or coverage.
- U14 preparation: independent verifier contract. It remains blocked from
  implementation until U13-A provides an opaque verified candidate.

## Delivery discipline

Changes enter this branch only after their own RED/GREEN evidence and an
independent adversarial review. The branch is not evidence of release or merge
readiness. See `docs/gaps/OPEN_GAPS.md` for dependencies that require a human
owner; no such gap is currently recorded.
