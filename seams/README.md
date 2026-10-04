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
Registry path (`registry.rs`, `authorizer.rs`, `tests/registry.rs`, FakeRegistry over std TcpListener): proposal read,
approve (client refuses wrong body hash, JWS bound to another op/proposal/hash/revision, replayed JWS; `probe_approve`
proves Core's own `candidate_changed`/`illegal_transition`), publish (base-release answer is an error), alias/release
reads (`staging` before publish refused). The human is a CLAUDE-STANDIN (`LocalSimAuthorizer`, `auth.simulated=true`),
never logged. Hook (not wired): `RegistryFlow::new(..)` after `freeze_draft` + native evaluation.
Live: `#[ignore] live_k3_registry_approve_publish_readback` (command in its doc comment; needs a stack + evaluated proposal).
STILL NOT met, so K3 acceptance ("Prompt+EvalSuite published to staging on the real image, readback equals commitment")
is NOT met: (1) the live registry run was not executed (no Podman here); (2) sealing the draft artifact goes
through the e2e fixtures server `/_e2e/config`, not a bridge contract route (BRG1 gap candidate); (3) the live test
is `#[ignore]` and was not run against the real image in this branch.

## W4a live acceptance (2026-10-04, pulso-dev, image localhost/pulso-core-runtime:c814c2b-920f5e3)

Env for every live test: `. seams\scripts\live-env.ps1 -Namespace <ns>` after `e2e-coreun.ps1 ... -Keep` (values never printed;
teardown with `local\core\stop.ps1` + `reset.ps1 -Confirm`). Evidence: `docs/reports/w4a-live/`.

- K3 (`core-client/tests/live_k3.rs`): version probe, dry-run, freeze, identical replay = same run, NATIVE evaluation driven from Rust
  (`core_client::evaluate`: binding, admission, evaluate-only stage), approve with the simulated-human JWS, publish to staging,
  readback = commitment (`release_id_preview`). Pass in ~2.3 s. Stand-ins: platform double for sealing/bindings (BRG1 candidate,
  `docs/reports/w4a-live/brg1-seal-and-bind-request.md`), human = `LocalSimAuthorizer`.
- V1 (`eval/tests/live_capture.rs` writes `eval/tests/fixtures/v1/*.json`; `eval/tests/v1_capture.rs` computes the count): 5 of 6
  (pass, failed_infra, fail, candidate_changed real; evaluation_result_lost fault-injected). quota_exceeded not captured
  (`fixtures/v1_attempts/quota_exceeded.json`). A failed gate is NOT an HTTP 409 on this profile: no verdict + proposal back in draft.
- E2 (`engine/src/live.rs`, `live_core.rs`, `engine/tests/live_handlers.rs` offline, `live_thread.rs` ignored): arms via `run_arm`
  (keys from job/step/fence/attempt) feed the V2 gate, native_eval, authority (blocked(gate) unless labelled simulated override),
  publish effectful.
- Findings: drafts are validated against the STAGING alias (base = staging, not prod, after a publish); one publish window per stack.
