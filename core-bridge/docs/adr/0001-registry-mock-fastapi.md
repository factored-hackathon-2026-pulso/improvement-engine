# ADR 0001: Registry wire mock is a minimal FastAPI process (CAP-52 option)

Status: accepted (L1b, plan 17.3.1).

## Context
V3 31.10.1 requires a real HTTP process, not an in-memory double inside the Rust adapter, that reproduces the
`/v1/registry` wire of the pinned agent-core (SHA 86a7674, contracts 1.3.0). Rust (`crates/platform-sim`) is Codex-owned;
Python is already required for the a2 level and for `gen-wire`.

## Decision
`platform-sim/registry_mock` is FastAPI with in-memory state, no SQL, no Rust adapter code and no `agent_core` import.
It mirrors M9 problem handling (`urn:agentcore:problem:*`) and registry errors (`urn:agentcore:registry:*`), EdDSA JWS auth
(`typ=principal+jws`, header exactly `{alg,kid,typ}`, `exp` checked against an injectable clock), the 12 error codes and
statuses, `Idempotency-Key` required on publish (422 before auth, as upstream), limits (50 changes, 262144 B, 200 nodes,
title 200, 10 proposals / rolling 24 h, 20 evals) and the state machine. The seed is the registry-demo release recovered
from `core-bridge/wire/.../golden/hash_vectors.json` (real hashes).

Candidate validation is a documented SUBSET: REG-KIND, REG-VERSION, REG-VERSION-TAKEN, REG-UNREFERENCED, REG-LIMIT.
M1 flow rules, cascade internals and candidate/content hashes beyond the seed are NOT reproduced (hashes are mock hashes and
masked in parity). Those belong to a2 and real.

Fault injection exists only under `/_sim/*`, off by default: `status500`, `latency`, `drop_after_commit`, `disconnect`,
plus clock advance, programmable evaluation (`pass|fail|failed_infra|timeout`, always `contract_fixture`) and reset.

## Consequences
- Fidelity is proven by `registry-wire-contract`: mock and a2 must satisfy 100% of `both` cases against fixtures recorded
  from a2 (`python -m parity.record --target a2`) or from real PG (`--target real`, manual).
- mock != fixture -> `mock_infidelity`; a2/real != fixture -> `wire_drift_detected` (blocks a pin bump).
- A passing mock never proves compatibility with the real server; reports carry `target` and `claims_real_local=false`.
