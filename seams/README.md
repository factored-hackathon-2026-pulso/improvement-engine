# seams

Claude-owned seams between the Python host and Codex's Rust engine. This is a
nested Cargo workspace with its own `[workspace]` and `Cargo.lock`; the root
`Cargo.toml` does not list it and Codex's `crates/**` stay untouched.

- Members: `crates/*`. Each lane adds its own crate directory.
- ABI rule: the `abi` crate and `core-client` must never depend on Codex's
  `crates/core`. A graph test (`core-client/tests/graph.rs`) enforces this.
- K0 crates are std-only so the build works offline.

## Build and test

Use a private target dir (lane convention: `scripts/env/lane-target-dir.ps1 -Lane claude-seams`):

    CARGO_TARGET_DIR=D:/cargo-targets/claude-seams cargo test --offline -j 2 --manifest-path seams/Cargo.toml

Run one cargo process at a time on this machine.

## core-client typed operations (K2)

`core-client` types all 11 `/internal/v1` operations of `bridge-contract/` over the K1 transport, with our own 1.3.0
DTOs (`serde_json::Value`-based, no `crates/core`): `version`, `invoke`, `read_task`, `run_arm`, `run_arm_by_id`,
`read_arm`, `read_arm_by_key`, `admit_evaluation`, `read_alias`, `dry_run` (strict) / `dry_run_raw`, `issue_credential`.
`ops::TYPED_OPERATIONS` maps each route to its method (`tests/ops_coverage.rs` compares it with `contract.json`).

- `canon`: JCS (RFC 8785), lowercase-hex digests, `Idempotency-Key`, `task_binding_ref`, arm `execution_id`,
  `evc-` evaluation_context_ref (a `|` in any component is rejected), Z-RFC3339 helpers.
- Rules enforced client-side: `valid:false` dry-run is `OpError::DryRunRefused` (never success) and a valid answer
  must carry the digest of the body we sent; arms single-flight digest excludes `deadline` and fills `agent_id`
  before hashing (`ArmRequest::single_flight_digest`); admission digest excludes `deadline`; the receipt's
  `task_binding_ref`, the arm `execution_id` and the admission ref must be the ones derived from our request.
- FakeCore generated from the goldens: `python seams/crates/core-client/gen/gen_fakecore.py` turns
  `bridge-contract/examples/flows/*.json` into `tests/common/golden_flows.json`; `tests/common/golden.rs` replays a
  named sequence of golden steps and fails on any request that differs from the Python bridge golden.
  `tests/golden_drift.rs` fails when the goldens change without regeneration (`--check` does the same from the shell).
- Not sent: `traceparent` (optional in the goldens), `credentials`, `trace`, the deprecated `evaluation_context_ref`.
- Live check (`tests/live.rs`, `#[ignore]`): `live_typed_replay_is_one_run` needs a stack with a seeded scout release.

## core-client K3 status (writer path): partial

Done and tested against the FakeCore: draft plan, dry-run, commitment, writer-stage invoke, commitment verification
(`CommitmentMismatch`), alias readback check, JCS/digest parity with Python `rfc8785` (seeded fuzz fixture).
NOT done, so K3 acceptance ("Prompt+EvalSuite published to staging on the real image, readback equals commitment") is
NOT met: (1) a Rust client + authorizer for Core `/v1/registry` approve/publish; (2) sealing the draft artifact goes
through the e2e fixtures server `/_e2e/config`, not a bridge contract route (BRG1 gap candidate); (3) the live test
is `#[ignore]` and was not run against the real image in this branch.
