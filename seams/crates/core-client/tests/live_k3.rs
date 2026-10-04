//! K3 LIVE ACCEPTANCE (row K3 of wp_table.csv): Prompt + EvalSuite published to the staging alias on the real image,
//! readback equal to the commitment, driven entirely from Rust. IGNORED by default.
//!
//! Needs ONE fresh e2e stack on Podman machine `pulso-dev` (a window publishes immutable prompt/suite 2.0.0, so one
//! `live_k3_acceptance` run per stack):
//!   pwsh e2e-core\run.ps1 -BaseImage localhost/pulso-core-runtime:c814c2b-920f5e3 -Namespace <ns> -Keep -PytestArgs "-k","nothing_selected"
//!   . seams\scripts\live-env.ps1 -Namespace <ns>
//!   $env:CARGO_TARGET_DIR='D:/cargo-targets/claude-seams-live'
//!   cargo test --offline -j 2 --manifest-path seams/Cargo.toml -p core-client --test live_k3 -- --ignored --nocapture
//!   local\core\stop.ps1 -Namespace <ns>; local\core\reset.ps1 -Namespace <ns> -Confirm
//! Labels: the human is the CLAUDE-STANDIN `LocalSimAuthorizer` (auth.simulated=true, sandbox human staff key of the
//! stack); draft sealing and binding pre-authorisation go through the e2e double's `/_e2e/config` (BRG1 gap candidate).
mod live_common;
use core_client::authorizer::{Jws, LocalSimAuthorizer};
use core_client::authoring::{Alias, CredentialRequest, DryRunRequest};
use core_client::evaluate::EvaluationRun;
use core_client::registry::{RegistryClient, RegistryFlow};
use live_common::*;
use serde_json::json;
use std::time::Duration;

#[test]
#[ignore = "needs a fresh kept e2e stack; see module docs"]
fn live_k3_acceptance() {
    let mut t = Timer::new();
    let (c, fx, ten) = (client("k3-accept"), Fx::from_env(), tenant());
    let tg = tag("k3a");

    // 1. version probe against the pin
    let v = t.time("version_probe", || c.version().expect("version"));
    v.check_pin().expect("pin");

    // 2. base release (prod alias of the seeded world)
    let base = t.time("alias_prod", || c.read_alias(&ten, None, &agent(), Alias::Prod)).expect("prod alias").release_id.expect("prod release");

    // 3. dry-run digest
    let var = variant("accept");
    let plan = plan(&var, &tg);
    let dry = t.time("dry_run", || c.dry_run(&format!("job-{tg}"), &DryRunRequest::new(&ten, &agent(), Some(&base), plan.changes.clone()))).expect("dry run");
    let dry_hash = dry.candidate_hash.clone().expect("hash");

    // 4. freeze through the writer stage, twice with the SAME keys: one run, one proposal (K1 acceptance)
    let run = writer_run(&tg);
    let fz = t.time("freeze_writer_stage", || c.freeze_draft(&fx, &plan, &run, Some(&base))).expect("freeze");
    assert_eq!(fz.candidate_hash, dry_hash, "the frozen candidate equals the dry-run digest");
    let again = t.time("freeze_replay", || c.freeze_draft(&fx, &plan, &run, Some(&base))).expect("replay freeze");
    assert_eq!((again.proposal_id.as_str(), again.task_binding_ref.as_str()), (fz.proposal_id.as_str(), fz.task_binding_ref.as_str()), "an identical replay is the same run");

    fx.script_closing_reply().expect("scripted gateway double");
    // 5. NATIVE evaluation of the frozen proposal (the registry approves only `evaluated`)
    let er = EvaluationRun {
        tenant_id: ten.clone(),
        job_id: format!("job-evalonly-{tg}"),
        logical_key: "evalonly".into(),
        attempt: 1,
        pulso_run_ref: format!("pr-job-evalonly-{tg}"),
        lab_grant_ref: "grant-contract".into(),
        writer_release_id: run.writer_release_id.clone(),
        writer_agent_version: "1.0.0".into(),
        budget_ref: "bud-e2e".into(),
        deadline: deadline(1),
    };
    let ev = t.time("evaluate_only", || c.evaluate_frozen(&fx, &fz, &var.suite, &er)).expect("evaluate");
    assert_eq!(ev.verdict(), Some("pass"), "native evaluation: {:?}", ev.receipt.outcome);

    // 6. approve with the simulated-human JWS, publish to staging, readback
    let bot = t.time("issue_bot_credential", || c.issue_credential(&format!("job-cred-{tg}"), &CredentialRequest::new(&ten, "constructor", "registry_write"))).expect("credential");
    let auth = LocalSimAuthorizer::new(&env("PULSO_HUMAN_KID"), seed("PULSO_HUMAN_SEED_HEX"), &ten, "local-supervisor");
    let reg = RegistryClient::new(&env("PULSO_CORE_ADDR"), Duration::from_secs(60));
    let mut flow = RegistryFlow::new(&reg, &auth, Jws::new(bot.jws().to_string()), &agent(), &fz.proposal_id, &fz.candidate_hash, Some(base.clone()));
    let ap = t.time("approve", || flow.approve()).expect("approve");
    assert!(ap.tamper_refused && ap.replay_refused, "Core refused the tampered hash and the replayed JWS");
    let published = t.time("publish", || flow.publish(&format!("pub-{tg}"))).expect("publish");
    let staging = t.time("alias_staging", || c.read_alias(&ten, None, &agent(), Alias::Staging)).expect("staging");
    core_client::writer::check_alias_readback(&staging, &published.release_id, Some(&base)).expect("staging readback");
    let via_registry = flow.alias_read("staging").expect("registry alias read");
    assert_eq!(via_registry.release_id, published.release_id);
    // the dry-run's release preview is the commitment of the release id
    if let Some(p) = &dry.release_id_preview {
        assert_eq!(p, &published.release_id, "published release equals the dry-run release_id_preview");
    }
    let prod_after = c.read_alias(&ten, None, &agent(), Alias::Prod).expect("prod").release_id;
    assert_eq!(prod_after.as_deref(), Some(base.as_str()), "prod is untouched");
    evidence(
        "k3_acceptance",
        &json!({"image": env("PULSO_LIVE_STACK_IMAGE"), "namespace": env("PULSO_LIVE_NAMESPACE"), "agent_core_sha": v.agent_core_sha,
            "proposal_id": fz.proposal_id, "candidate_hash": fz.candidate_hash, "dry_run_candidate_hash": dry_hash,
            "release_id_preview": dry.release_id_preview, "base_release": base, "published_release": published.release_id,
            "staging_after": staging.release_id, "prod_after": prod_after, "native_verdict": ev.verdict(),
            "approver": ap.approver, "approver_simulated": true, "tamper_refused": ap.tamper_refused, "replay_refused": ap.replay_refused,
            "timings": t.json()}),
    );
}
