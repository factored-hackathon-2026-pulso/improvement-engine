# MEM1 thin memory (DEMO-1)

Date: 2026-10-04
Owner: CLAUDE (lane L-MEM)

## Scope

New crate `seams/crates/memory` (std, `serde_json`, `sha2` only; no Codex crates).
It provides an in-process note artifact with evidence refs, a read-only wiki
view per claim, `confirm`, and `contradict` of a prior claim (the prior is kept,
marked `contradicted`, and linked both ways).

## Behaviours (each RED then GREEN)

1. Note artifact: `claim_key`, `statement`, `evidence_refs`, `status`, labels `scope=demo1_thin`, `durable=false`.
2. Wiki read per claim key.
3. Confirm appends evidence and sets `confirmed`.
4. Contradict a prior claim (same claim key required).
5. Evidence refs must resolve in the `EvidenceStore`; unresolved or empty refs are rejected without mutation; each ref is pinned to a sha256 digest in the artifact.
6. Secrets and PII (email, key and token shapes, bearer, long digit runs) are rejected in claim keys and statements on add and contradict.

## Verification

`cargo test -j 1 -p memory` with `CARGO_TARGET_DIR=D:/cargo-targets/claude-w4e-mem1`: 7 integration tests green; clippy clean. `seams/crates/*` is a workspace glob, so the crate is a member without a Cargo.toml edit.

## Not claimed

DEMO-1 thin memory only. Nothing is persisted: no durable head, no restart or
kill -9 survival (DMEMC), no temporal publication protocol (Codex DMEM1/2),
no wiki grants or QueryReceipt (E5R, WIKI). The evidence store is a caller-supplied
map; resolving refs against real lab or CAS artifacts is an integration step.
The sensitive-content scan is a conservative heuristic, not a privacy guarantee.
