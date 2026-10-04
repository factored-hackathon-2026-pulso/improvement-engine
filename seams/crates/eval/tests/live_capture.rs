//! V1 LIVE CAPTURE of the evaluate-level outcomes on the real image. IGNORED by default: it WRITES the golden fixtures
//! `tests/fixtures/v1/<outcome>.json` (provenance: stack image, agent-core sha, date, command, label) from observations
//! of a real stack; `eval::capture::capture_six` then computes "captured N of 6" from those files.
//!
//! Needs one kept e2e stack on Podman machine `pulso-dev` (see core-client/tests/live_k3.rs for the stack and env
//! commands; this test does not publish, so it can share a stack with `live_k3_acceptance`):
//!   cargo test --offline -j 2 --manifest-path seams/Cargo.toml -p eval --test live_capture -- --ignored --nocapture --test-threads 1
//! Outcomes: pass, failed_infra (gateway not scripted: honest infra failure), fail (a suite whose expectation the agent
//! never meets), candidate_changed (the engine's candidate hash is not the proposal's), evaluation_result_lost (FAULT
//! INJECTED: a TCP relay forwards the evaluate-only invoke, lets Core finish and drops the answer; the replay with the
//! same key recovers it). quota_exceeded cannot be produced on this Core: see tests/fixtures/v1_attempts/.
#[path = "../../core-client/tests/live_common/mod.rs"]
mod live_common;
use core_client::authorizer::Jws;
use core_client::authoring::{Alias, CredentialRequest, DryRunRequest};
use core_client::client::CallError;
use core_client::evaluate::{EvaluationRun, SuiteRef};
use core_client::registry::RegistryClient;
use core_client::writer::FrozenProposal;
use core_client::{CoreClient, OpError};
use live_common::*;
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::Duration;

const COMMAND: &str = "cargo test --offline -j 2 --manifest-path seams/Cargo.toml -p eval --test live_capture -- --ignored --nocapture --test-threads 1 (env: seams/scripts/live-env.ps1)";

struct Ctx {
    c: CoreClient,
    fx: Fx,
    reg: RegistryClient,
    bot: Jws,
    base: String,
    image: String,
    sha: String,
    date: String,
}

fn ctx() -> Ctx {
    let (c, fx, ten) = (client("v1-capture"), Fx::from_env(), tenant());
    let v = c.version().expect("version");
    v.check_pin().expect("pin");
    let base = c.read_alias(&ten, None, &agent(), Alias::Staging).expect("staging").release_id.expect("release");
    let bot = Jws::new(c.issue_credential("job-v1-cred", &CredentialRequest::new(&ten, "constructor", "registry_write")).expect("credential").jws().to_string());
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
    Ctx {
        c,
        fx,
        reg: RegistryClient::new(&env("PULSO_CORE_ADDR"), Duration::from_secs(60)),
        bot,
        base,
        image: env("PULSO_LIVE_STACK_IMAGE"),
        sha: v.agent_core_sha,
        date: core_client::authorizer::iso_z(now)[..10].to_string(),
    }
}

impl Ctx {
    fn provenance(&self, label: &str) -> Value {
        json!({"stack_image": self.image, "agent_core_sha": self.sha, "date": self.date, "command": COMMAND, "label": label, "namespace": env("PULSO_LIVE_NAMESPACE")})
    }

    fn freeze(&self, variant_name: &str, tag: &str) -> (FrozenProposal, SuiteRef) {
        let var = variant(variant_name);
        let plan = plan(&var, tag);
        let fz = self.c.freeze_draft(&self.fx, &plan, &writer_run(tag), Some(&self.base)).expect("freeze");
        (fz, var.suite)
    }

    fn run(&self, tag: &str, attempt: u32) -> EvaluationRun {
        EvaluationRun {
            tenant_id: tenant(),
            job_id: format!("job-evalonly-{tag}"),
            logical_key: "evalonly".into(),
            attempt,
            pulso_run_ref: format!("pr-job-evalonly-{tag}"),
            lab_grant_ref: "grant-contract".into(),
            writer_release_id: env("PULSO_LIVE_WRITER_RELEASE"),
            writer_agent_version: "1.0.0".into(),
            budget_ref: "bud-e2e".into(),
            deadline: deadline(1),
        }
    }

    fn proposal_state(&self, pid: &str) -> String {
        self.reg.proposal(pid, &self.bot).expect("proposal read").state
    }

    /// The observation stored in a fixture, from the real answer of one evaluate attempt.
    fn observe(&self, pid: &str, r: &Result<core_client::evaluate::Evaluation, OpError>) -> Value {
        match r {
            Ok(ev) => {
                let ops: Vec<String> = ev
                    .receipt
                    .fact("pulso_writer_receipts")
                    .and_then(|f| f.value["write_receipts"].as_array().cloned())
                    .unwrap_or_default()
                    .iter()
                    .filter_map(|w| w["op"].as_str().map(str::to_string))
                    .collect();
                json!({"kind": "stage_receipt", "http_status": ev.receipt.http_status, "state": format!("{:?}", ev.receipt.state), "outcome": ev.receipt.outcome,
                    "write_ops": ops, "native_evaluation": ev.native, "proposal_state_after": self.proposal_state(pid)})
            }
            Err(OpError::Call(CallError::Api(a))) => json!({"kind": "http_error", "http_status": a.status, "code": a.code, "stage": "evaluation_admission_or_stage", "proposal_state_after": self.proposal_state(pid)}),
            Err(OpError::Call(CallError::Transport { sent, message })) => json!({"kind": "timeout", "request_sent": sent, "detail": message, "proposal_state_after": self.proposal_state(pid)}),
            Err(e) => panic!("not an evaluate outcome: {e}"),
        }
    }

    fn record(&self, outcome: &str, label: &str, observation: Value, extra: Value) {
        let mut doc = json!({"outcome": outcome, "provenance": self.provenance(label), "observation": observation});
        for (k, v) in extra.as_object().cloned().unwrap_or_default() {
            doc[k] = v;
        }
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(eval::capture::FIXTURE_DIR);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(format!("{outcome}.json")), serde_json::to_string_pretty(&doc).unwrap() + "\n").unwrap();
        eprintln!("RECORDED {outcome} ({label})");
    }
}

/// Core's own stored report of a proposal (`pulso_bridge.eval_reports`, read-only SQL in the stack's Postgres).
fn db_report(pid: &str) -> Value {
    let podman = std::env::var("PULSO_PODMAN").unwrap_or_else(|_| r"C:\Users\alexg\AppData\Local\Programs\Podman\podman.exe".into());
    let container = format!("pulso-{}-core-postgres-1", env("PULSO_LIVE_NAMESPACE"));
    assert!(pid.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-'), "proposal ids are uuids");
    let q = format!("select json_build_object('verdict', verdict, 'gate_failed', gate_failed, 'report_digest', report_digest, 'report', report)::text from pulso_bridge.eval_reports where proposal_id = '{pid}' limit 1");
    let out = std::process::Command::new(podman).args(["--connection", "pulso-dev", "exec", &container, "psql", "-U", "postgres", "-d", "core_runtime", "-At", "-c", &q]).output().expect("podman exec");
    serde_json::from_slice(out.stdout.trim_ascii()).unwrap_or_else(|_| json!({"unavailable": String::from_utf8_lossy(&out.stderr).chars().take(200).collect::<String>()}))
}

#[test]
#[ignore = "needs a kept e2e stack; writes fixtures; see module docs"]
fn live_v1_pass_and_failed_infra() {
    let x = ctx();
    // pass: gateway scripted
    x.fx.script_closing_reply().unwrap();
    let tg = tag("v1p");
    let (fz, suite) = x.freeze("accept_alt", &tg);
    let r = x.c.evaluate_frozen(&x.fx, &fz, &suite, &x.run(&tg, 1));
    let obs = x.observe(&fz.proposal_id, &r);
    x.record("pass", "real", obs, json!({"proposal_id": fz.proposal_id, "suite": {"id": suite.id, "version": suite.version}}));
    // failed_infra: the scripted llm-gateway double has no rule, so the agent's only model call fails (honest infra failure)
    x.fx.config(&json!({"llm_replace": true, "llm_rules": []})).unwrap();
    let tg = tag("v1i");
    let (fz, suite) = x.freeze("accept_alt", &tg);
    let r = x.c.evaluate_frozen(&x.fx, &fz, &suite, &x.run(&tg, 1));
    let obs = x.observe(&fz.proposal_id, &r);
    x.record("failed_infra", "real", obs, json!({"proposal_id": fz.proposal_id, "cause": "llm-gateway double without a scripted rule (llm_rules replaced by [])"}));
    x.fx.script_closing_reply().unwrap();
}

#[test]
#[ignore = "needs a kept e2e stack; writes fixtures; see module docs"]
fn live_v1_fail_and_candidate_changed() {
    let x = ctx();
    x.fx.script_closing_reply().unwrap();
    // fail: suite 3.0.0 expects scenario `resuelto` to escalate; the agent resolves it
    let tg = tag("v1f");
    let (fz, suite) = x.freeze("fail_suite", &tg);
    let r = x.c.evaluate_frozen(&x.fx, &fz, &suite, &x.run(&tg, 1));
    let obs = x.observe(&fz.proposal_id, &r);
    let report = db_report(&fz.proposal_id);
    x.record("fail", "real", obs, json!({"proposal_id": fz.proposal_id, "suite": {"id": suite.id, "version": suite.version},
        "cause": "scenario `resuelto` expects {outcome: escalated} but the seeded task agent completes it (the suite, not the code, is what fails)",
        "core_eval_report": report}));
    // candidate_changed, natural variant: the failed gate sent the proposal back to draft, so a second evaluation of the same
    // proposal is refused because its candidate is no longer the frozen one
    let again = x.c.evaluate_frozen(&x.fx, &fz, &suite, &x.run(&tg, 2));
    let natural = x.observe(&fz.proposal_id, &again);
    // candidate_changed, explicit: the engine's candidate hash differs from the proposal's frozen one
    let tg2 = tag("v1c");
    let (fz2, suite2) = x.freeze("accept_alt", &tg2);
    let other = variant("fail_suite");
    let other_hash = x.c.dry_run(&format!("job-{tg2}-other"), &DryRunRequest::new(&tenant(), &agent(), Some(&x.base), other.plan_changes.clone())).expect("dry run").candidate_hash.expect("hash");
    assert_ne!(other_hash, fz2.candidate_hash);
    let mut changed = fz2.clone();
    changed.candidate_hash = other_hash.clone();
    let r = x.c.evaluate_frozen(&x.fx, &changed, &suite2, &x.run(&tg2, 1));
    let obs = x.observe(&fz2.proposal_id, &r);
    x.record("candidate_changed", "real", obs, json!({"proposal_id": fz2.proposal_id, "frozen_candidate_hash": fz2.candidate_hash, "engine_candidate_hash": other_hash,
        "cause": "the engine evaluates against a candidate hash that is not the hash frozen in the proposal",
        "natural_variant": {"description": "second evaluation of a proposal whose failed gate returned it to draft", "observation": natural}}));
}

/// One-shot TCP relay: forwards each request to Core; for the evaluate-only invoke (`/core-tasks/invoke` with a writer stage that
/// is NOT the first) it lets Core finish and never answers the client (FAULT INJECTION).
fn relay(upstream: String, drop_path: &'static str) -> (String, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap().to_string();
    let dropped = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let d2 = dropped.clone();
    std::thread::spawn(move || {
        for conn in l.incoming().flatten() {
            let (up, d3) = (upstream.clone(), d2.clone());
            std::thread::spawn(move || {
                let mut c = conn;
                let mut buf = Vec::new();
                let mut chunk = [0u8; 8192];
                let (head_end, want) = loop {
                    let n = c.read(&mut chunk).unwrap_or(0);
                    if n == 0 {
                        return;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                    if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&buf[..p]).to_ascii_lowercase();
                        let len = head.lines().find_map(|l| l.strip_prefix("content-length:")).and_then(|v| v.trim().parse::<usize>().ok()).unwrap_or(0);
                        break (p + 4, len);
                    }
                };
                while buf.len() < head_end + want {
                    let n = c.read(&mut chunk).unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                }
                let first_line = String::from_utf8_lossy(&buf[..buf.iter().position(|b| *b == b'\r').unwrap_or(0)]).to_string();
                let mut u = TcpStream::connect(&up).unwrap();
                u.write_all(&buf).unwrap();
                let mut resp = Vec::new();
                u.read_to_end(&mut resp).unwrap();
                if first_line.contains(drop_path) && d3.load(std::sync::atomic::Ordering::SeqCst) == 0 {
                    d3.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    std::thread::sleep(Duration::from_secs(8)); // the answer is dropped: the client times out first
                    return;
                }
                let _ = c.write_all(&resp);
            });
        }
    });
    (addr, dropped)
}

#[test]
#[ignore = "needs a kept e2e stack; writes fixtures; FAULT INJECTION; see module docs"]
fn live_v1_result_lost_fault_injected() {
    let x = ctx();
    x.fx.script_closing_reply().unwrap();
    let tg = tag("v1l");
    let (fz, suite) = x.freeze("accept_alt", &tg);
    let (addr, dropped) = relay(env("PULSO_BRIDGE_ADDR"), "/core-tasks/invoke");
    let mut cfg = core_client::ClientConfig::new(&addr, &env("PULSO_SERVICE_KID"), seed("PULSO_SERVICE_SEED_HEX"), "v1-lost");
    cfg.timeout = Duration::from_secs(4);
    let lossy = CoreClient::new(cfg).with_attempts(1);
    let run = x.run(&tg, 1);
    let lost = lossy.evaluate_frozen(&x.fx, &fz, &suite, &run);
    assert!(matches!(lost, Err(OpError::Call(CallError::Transport { sent: true, .. }))), "{lost:?}");
    assert_eq!(dropped.load(std::sync::atomic::Ordering::SeqCst), 1, "the relay dropped exactly one answer");
    let obs = x.observe(&fz.proposal_id, &lost);
    // the evaluation DID run in Core: after the timeout the proposal is `evaluated`; the same-key replay returns the stored receipt
    let replay = x.c.replay_evaluation(&fz, &suite, &run).expect("replay");
    assert_eq!(replay.verdict(), Some("pass"), "an identical replay recovers the lost result without a second run");
    x.record("evaluation_result_lost", "fault-injected", obs, json!({"proposal_id": fz.proposal_id,
        "fault": "TCP relay forwarded the evaluate-only invoke, waited for Core's answer and never relayed it; client timeout 4 s, 1 attempt",
        "recovery": {"replay_same_key_verdict": replay.verdict(), "replay_http_status": replay.receipt.http_status}}));
}
