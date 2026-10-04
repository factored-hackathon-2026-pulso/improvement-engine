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
(Superseded by the W4a live section below: the live K3 run was executed; the platform double and simulated human remain stand-ins.)

## W4a live acceptance (2026-10-04, pulso-dev, image localhost/pulso-core-runtime:c814c2b-920f5e3)

Env for every live test: `. seams\scripts\live-env.ps1 -Namespace <ns>` after `e2e-core
un.ps1 ... -Keep` (values never printed;
teardown with `local\core\stop.ps1` + `reset.ps1 -Confirm`). Evidence: `docs/reports/w4a-live/`.

- K3 (`core-client/tests/live_k3.rs`): version probe, dry-run, freeze, identical replay = same run, NATIVE evaluation driven from Rust
  (`core_client::evaluate`: binding, admission, evaluate-only stage), approve with the simulated-human JWS, publish to staging,
  readback = commitment (`release_id_preview`). Pass in ~2.3 s. Stand-ins: platform double for sealing/bindings (BRG1 candidate,
  `docs/reports/w4a-live/brg1-seal-and-bind-request.md`), human = `LocalSimAuthorizer`.
- V1 (`eval/tests/live_capture.rs` writes `eval/tests/fixtures/v1/*.json`; `eval/tests/v1_capture.rs` computes the count): 5 of 6
  (4 real + 1 fault-injected, split computed by `capture::captured_split`; the lost result is a TCP-relay fault, not a real timeout).
  quota_exceeded not captured (`tests/fixtures/v1_attempts/quota_exceeded.json`). Honest scope of the "real" ones: they are real Core
  responses to DELIBERATELY PROVOKED conditions, not organic: `fail` = a suite whose `resuelto` expectation (escalated) the seeded
  agent never meets (an evaluate-level fail by construction, proves the classifier/Core path, not that any real candidate fails);
  `failed_infra` = llm-gateway double with no scripted rule; `candidate_changed` = engine sends a hash other than the frozen one.
  V3-vs-Core DIVERGENCE: spec V3 31.7.1 (CAP-38, H6) defines `fail` as HTTP 409 `gate_failed` with the EvalReport as payload.
  On this profile (agent_core_real bridge, evaluate-only stage) the observed Core answer is 200, stage completed, verified
  `evaluate` write, `native_evaluation: null`, proposal back in `draft`; the report is only in `pulso_bridge.eval_reports`.
  `eval::outcomes::classify_observation` therefore maps "no verdict + evaluate write + proposal in draft" to `fail`; this is an
  inference, not the V3 wire shape (`classify_http(409, gate_failed)` is kept for a profile that passes it through). Reported as a
  finding in `docs/reports/w4a-live/brg1-seal-and-bind-request.md`.
- K3 readback caveat: `release_id_preview == readback` is deterministic given the hash (`rel-` + candidate_hash[:16]); the real
  evidence is that Core's dry-run (before freeze) and publish (after native evaluation) agree on the hash and that prod stays untouched.
  Draft sealing and the evaluation-binding pre-authorisation go through the e2e double's `/_e2e/config` in EVERY live K3/E2 run.
- Engine offline `stub_handlers` (`--live-stubs`) emit not_exercised/blocked(core), never live results; only `LiveCore` handlers are live.
- E2 (`engine/src/live.rs`, `live_core.rs`, `engine/tests/live_handlers.rs` offline, `live_thread.rs` ignored): arms via `run_arm`
  (keys from job/step/fence/attempt) feed the V2 gate, native_eval, authority (blocked(gate) unless labelled simulated override),
  publish effectful.
- Findings: drafts are validated against the STAGING alias (base = staging, not prod, after a publish); one publish window per stack.
