//! E4R: platform ingest (ACK, quarantine, gaps, late rows), health, JWT replay across restarts.
//! Rust-only on purpose: the Python double (`ingest_fixture`) does ACK/dedup/CAS but implements no quarantine or gap
//! detection (the exporter does that on its side); the shared black-box file covers the common ACK surface.
mod common;
use common::*;
use serde_json::json;

fn ev(seqs: &[i64], schema: &serde_json::Value) -> Vec<serde_json::Value> {
    seqs.iter().map(|s| domain(*s, "case.viewed", schema)).collect()
}

#[test]
fn ingest_gap_test_fails_to_quarantine() {
    let rig = Rig::new();
    let schema = rig.upload_schema();
    // sequences 1, 2, 5, 6: rows 3 and 4 are missing and nothing in the batch declares the hole
    let b = batch(B { revision: Some(0), from: Some(1), to: Some(6), cursor: "s.6".into(), events: ev(&[1, 2, 5, 6], &schema) });
    let (st, r) = rig.post_batch(&b);
    assert_eq!(st, 202, "{r}");
    assert_eq!((r["disposition"].as_str(), r["quarantine_reason"].as_str()), (Some("quarantined"), Some("sequence_gap")), "{r}");
    assert_eq!(r["gaps"], json!([[3, 4]]));
    assert_eq!((r["accepted_event_count"].as_i64(), r["quarantined_event_count"].as_i64(), r["checkpoint_advanced"].as_bool()), (Some(0), Some(4), Some(false)));
    let cur = rig.cursor();
    assert_eq!((cur["cursor"].is_null(), cur["cursor_revision"].as_i64()), (true, Some(0)), "a quarantined batch never moves the checkpoint");
    // the corrected batch (rows 3 and 4 present) is a different batch and goes through
    let ok = batch(B { revision: Some(0), from: Some(1), to: Some(6), cursor: "s.6".into(), events: ev(&[1, 2, 3, 4, 5, 6], &schema) });
    let (st, r) = rig.post_batch(&ok);
    assert_eq!((st, r["accepted_event_count"].as_i64(), r["disposition"].as_str()), (202, Some(6), Some("accepted")), "{r}");
}

#[test]
fn replaying_a_quarantined_batch_returns_the_same_quarantine_receipt() {
    let rig = Rig::new();
    let schema = rig.upload_schema();
    let b = batch(B { revision: Some(0), from: Some(1), to: Some(3), cursor: "s.3".into(), events: ev(&[1, 3], &schema) });
    let (_, first) = rig.post_batch(&b);
    let (st, again) = rig.post_batch(&b);
    assert_eq!(st, 200);
    assert_eq!((again["disposition"].as_str(), again["gaps"].clone()), (Some("quarantined"), first["gaps"].clone()));
}

#[test]
fn trailing_hole_up_to_the_declared_to_seq_is_a_gap() {
    let rig = Rig::new();
    let schema = rig.upload_schema();
    let b = batch(B { revision: Some(0), from: Some(1), to: Some(5), cursor: "s.5".into(), events: ev(&[1, 2, 3], &schema) });
    let (st, r) = rig.post_batch(&b);
    assert_eq!((st, r["gaps"].clone()), (202, json!([[4, 5]])), "{r}");
}

#[test]
fn hole_against_the_previous_checkpoint_is_a_gap() {
    let rig = Rig::new();
    let schema = rig.upload_schema();
    let (_, r) = rig.post_batch(&batch(B { revision: Some(0), from: Some(1), to: Some(2), cursor: "s.2".into(), events: ev(&[1, 2], &schema) }));
    assert_eq!(r["disposition"], "accepted", "{r}");
    // the next batch starts at 5: rows 3 and 4 were never delivered
    let (_, r) = rig.post_batch(&batch(B { revision: Some(1), from: Some(5), to: Some(6), cursor: "s.6".into(), events: ev(&[5, 6], &schema) }));
    assert_eq!((r["disposition"].as_str(), r["gaps"].clone()), (Some("quarantined"), json!([[3, 4]])), "{r}");
    assert_eq!(rig.cursor()["cursor"], "s.2");
}

#[test]
fn declared_gap_is_acked_and_late_rows_close_it() {
    let rig = Rig::new();
    let schema = rig.upload_schema();
    let mut events = ev(&[1, 2, 5, 6], &schema);
    events.push(finding("finding:gap:3-4", "gap_suspected", json!({"from_sequence": 3, "to_sequence": 4, "backfill_requested": true}), &schema));
    let (st, r) = rig.post_batch(&batch(B { revision: Some(0), from: Some(1), to: Some(6), cursor: "s.6".into(), events }));
    assert_eq!((st, r["disposition"].as_str(), r["accepted_event_count"].as_i64(), r["checkpoint_advanced"].as_bool()), (202, Some("accepted"), Some(5), Some(true)), "{r}");
    assert_eq!(rig.cursor()["open_gaps"], json!([[3, 4]]));
    // the rows commit late: they arrive as late events and close the backfill request
    let late_rows = vec![late(domain(3, "case.viewed", &schema)), late(domain(4, "case.viewed", &schema))];
    let (_, r) = rig.post_batch(&batch(B { revision: Some(1), from: Some(3), to: Some(4), cursor: "s.6b".into(), events: late_rows }));
    assert_eq!((r["accepted_event_count"].as_i64(), r["late_event_count"].as_i64(), r["disposition"].as_str()), (Some(2), Some(2), Some("accepted")), "{r}");
    assert_eq!(rig.cursor()["open_gaps"], json!([]));
}

#[test]
fn unknown_release_type_is_quarantined_but_the_batch_is_acked() {
    let rig = Rig::new();
    let schema = rig.upload_schema();
    let events = vec![domain(1, "case.opened", &schema), domain(2, "release.published", &schema), domain(3, "case.assigned", &schema)];
    let (st, r) = rig.post_batch(&batch(B { revision: Some(0), from: Some(1), to: Some(3), cursor: "s.3".into(), events }));
    assert_eq!(st, 202, "{r}");
    assert_eq!((r["accepted_event_count"].as_i64(), r["quarantined_event_count"].as_i64(), r["disposition"].as_str()), (Some(2), Some(1), Some("accepted")), "{r}");
    assert_eq!(r["unknown_event_types"], json!({"release.published": 1}));
    // acked past the unknown row (it still counts for contiguity), and the checkpoint moved
    assert_eq!(rig.cursor()["cursor"], "s.3");
}

#[test]
fn planned_and_denied_types_quarantine_with_their_class_and_the_payload_is_never_kept() {
    let rig = Rig::new();
    let schema = rig.upload_schema();
    let events = vec![domain(1, "team.created", &schema), domain(2, "auth.password_accepted", &schema), domain(3, "staff.created", &schema), domain(4, "case.opened", &schema)];
    let (_, r) = rig.post_batch(&batch(B { revision: Some(0), from: Some(1), to: Some(4), cursor: "s.4".into(), events }));
    assert_eq!((r["accepted_event_count"].as_i64(), r["quarantined_event_count"].as_i64()), (Some(1), Some(3)), "{r}");
    let tok = rig.token("ob", "control-api", "observations", "t1", json!({"purpose": "platform_observations"}));
    let (st, q) = rig.call("GET", "/internal/v1/platform/quarantine", None, Some(&tok), &[]);
    assert_eq!(st, 200);
    let classes: Vec<(&str, &str)> = q["items"].as_array().unwrap().iter().map(|i| (i["event_type"].as_str().unwrap(), i["classification"].as_str().unwrap())).collect();
    assert_eq!(classes, vec![("auth.password_accepted", "denied"), ("staff.created", "planned"), ("team.created", "planned")]);
    assert!(!q.to_string().contains("DO-NOT-STORE"), "quarantine keeps metadata, never the payload");
    // the deployment pins the exporter binding to tenant t1: another tenant's token is refused outright
    let other = rig.token("ob", "control-api", "observations", "t2", json!({"purpose": "platform_observations"}));
    assert_eq!(rig.call("GET", "/internal/v1/platform/quarantine", None, Some(&other), &[]).0, 403);
}

#[test]
fn ingest_is_tenant_and_scope_bound_and_size_capped() {
    let rig = Rig::new();
    let schema = rig.upload_schema();
    let b = batch(B { revision: Some(0), from: Some(1), to: Some(1), cursor: "s.1".into(), events: ev(&[1], &schema) });
    let digest = b["batch_digest"].as_str().unwrap();
    let h = [("Idempotency-Key", digest)];
    let wrong_scope = rig.token("ob", "control-api", "observations", "t1", json!({"scope": "binding", "purpose": "platform_observations"}));
    assert_eq!(rig.call("POST", "/internal/v1/platform/observations", Some(&b), Some(&wrong_scope), &h).0, 403);
    let other_tenant = rig.token("ob", "control-api", "observations", "t2", json!({"purpose": "platform_observations"}));
    assert_eq!(rig.call("POST", "/internal/v1/platform/observations", Some(&b), Some(&other_tenant), &h).0, 403);
    assert_eq!(rig.call("POST", "/internal/v1/platform/observations", Some(&b), None, &h).0, 401);
    let mut huge = b.clone();
    huge["events"][0]["source_event"]["payload"] = json!({"blob": "x".repeat(600 * 1024)});
    let tok = rig.token("ob", "control-api", "observations", "t1", json!({"purpose": "platform_observations"}));
    assert_eq!(rig.call("POST", "/internal/v1/platform/observations", Some(&huge), Some(&tok), &h).0, 413);
}

#[test]
fn health_is_open_cheap_and_leaks_nothing() {
    let rig = Rig::new();
    let (st, v) = rig.call("GET", "/healthz", None, None, &[]);
    assert_eq!((st, v["status"].as_str()), (200, Some("ok")), "{v}");
    assert_eq!(rig.call("POST", "/healthz", None, None, &[]).0, 404);
}

#[test]
fn a_token_minted_before_this_process_booted_is_refused() {
    // The jti set is in memory: a token captured before a restart could otherwise be replayed once afterwards.
    let rig = Rig::new();
    let stale = rig.token("cb", "control-api", "binding", "t1", json!({"purpose": "core_task_binding", "iat": (T0 as i64) - 30}));
    let body = json!({"schema_version": "1", "tenant_id": "t1", "job_id": "j", "command_key": "c", "request_digest": "d".repeat(64),
                      "attempt": 1, "core_run_id": "r", "bridge_instance_id": "b", "task_binding_ref": "bind-c"});
    let (st, v) = rig.call("POST", "/internal/v1/core-task-bindings", Some(&body), Some(&stale), &[("Idempotency-Key", "c")]);
    assert_eq!((st, v["code"].as_str()), (401, Some("pulso:auth_pre_boot_token")), "{v}");
    let fresh = rig.token("cb", "control-api", "binding", "t1", json!({"purpose": "core_task_binding"}));
    assert_eq!(rig.call("POST", "/internal/v1/core-task-bindings", Some(&body), Some(&fresh), &[("Idempotency-Key", "c")]).0, 200);
}
