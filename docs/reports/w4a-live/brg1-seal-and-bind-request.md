# BRG1 request candidate: contract routes to seal a draft artifact and to pre-authorise an evaluation binding

Status: written request only (journal-ready). Nothing in `bridge-contract/` or the Python bridge was changed. Team: CL.

[DEP-ASK] 2026-10-04 CL -> bridge-contract owner (BRG1)

Today the Rust writer flow (core-client K3) needs two platform effects that have NO `/internal/v1` route and go through the
e2e fixtures double's `/_e2e/config` (a test-only admin channel):

1. Sealing the draft-plan artifact (`draft_plan_ref`) that the `pulso-writer` stage reads from the broker
   (`config.artifacts[{tenant,id,content}]`). Proposed contract route: `PUT /internal/v1/artifacts/{id}` (purpose
   `artifact_seal`, tenant-scoped, body = content, answer = digest; idempotent by id + digest, conflict on other content).
2. Pre-authorising the task binding the evaluate-only writer stage presents (`config.preauthorized_bindings
   [{tenant,binding_ref}]`; `binding_ref = sha256(tenant|idempotency_key)`). Proposed: the bridge derives and the platform
   confirms it through the existing binding callback, so no engine-side call is needed; if the engine must request it,
   `POST /internal/v1/core-task-bindings/preauthorize`.

Evidence: `seams/crates/core-client/tests/live_common/mod.rs` (`Fx`), live run `docs/reports/w4a-live/k3_acceptance.json`.
Until a route exists the live K3 acceptance is labelled "platform = e2e double" and the engine cannot run outside the e2e stack.

Also observed (not a request, a finding; V3-vs-Core DIVERGENCE: spec V3 31.7.1 CAP-38 defines fail as 409 gate_failed, the classifier maps this observed shape to fail by inference): a failed native gate does not surface as HTTP 409 `gate_failed` on the
`agent_core_real` profile: the evaluate-only stage completes with a verified `evaluate` write and `native_evaluation: null`,
and the proposal returns to draft (see `seams/crates/eval/tests/fixtures/v1/fail.json`). A consumer cannot read the failing
report through the contract (it exists only in `pulso_bridge.eval_reports`). Request: expose `{verdict, eval_run_ref,
report_digest}` for `fail` in `pulso_writer_receipts.native_evaluation`.
