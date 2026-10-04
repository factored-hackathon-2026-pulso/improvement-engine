//! E5R: lab broker grants, QueryReceipt for a sqlite lab query, wiki read.
//! Rust-only: the Python double answers lab calls with canned data and no grants (and hands out a raw row), so only the
//! wiki-read surface is shared (see `contracts/control-api/test_blackbox.py`). The lab file is the ED0L shape
//! (`e2e-core/src/claude_standin/ed0_lab.py`); a Python-built ED0L lab is also served in the black-box file.
mod common;
use common::*;
use serde_json::{Value, json};

const REF: &str = "bind-c1";

fn rig_with_lab(name: &str, rows: &[(&str, &str, &str, i64, i64)]) -> Rig {
    let path = make_lab(name, 10, rows);
    let rig = Rig::with(|c| {
        c.labs.insert("t1".into(), path);
    });
    rig.bind_ref(REF, "t1");
    rig
}

fn query(rig: &Rig, sid: &str, key: &str, metric: &str, window: &str) -> (u16, Value) {
    rig.lab("POST", &format!("/lab/sessions/{sid}/queries"), Some(&json!({"query_key": key, "metric_id": metric, "window_id": window})), "t1", None)
}

#[test]
fn grant_test_fails_expired_grant_accepted() {
    let rig = rig_with_lab("expired", &[("recurrence_rate", "w1", "g_a", 3, 12)]);
    let (st, g) = rig.issue_grant("t1", REF, "lab", 30);
    assert_eq!(st, 201, "{g}");
    let grant = g["grant_ref"].as_str().unwrap().to_string();
    assert!(g["expires_at"].as_str().unwrap().ends_with('Z'));
    let open = || rig.lab("POST", "/lab/sessions", Some(&json!({"binding_ref": REF})), "t1", Some(&grant));
    let (st, s) = open();
    assert_eq!(st, 200, "{s}");
    let sid = s["session_ref"].as_str().unwrap().to_string();
    assert_eq!(query(&rig, &sid, "k1", "recurrence_rate", "w1").0, 202);
    rig.advance(31.0); // the grant is past its expiry
    let (st, v) = open();
    assert_eq!((st, v["code"].as_str()), (403, Some("grant_expired")), "{v}");
    let (st, v) = query(&rig, &sid, "k2", "recurrence_rate", "w1"); // an open session does not outlive its grant
    assert_eq!((st, v["code"].as_str()), (403, Some("grant_expired")), "{v}");
}

#[test]
fn a_grant_is_bound_to_its_tenant_binding_and_scope() {
    let rig = rig_with_lab("scope", &[("recurrence_rate", "w1", "g_a", 3, 12)]);
    rig.bind_ref("bind-other", "t1");
    rig.bind_ref("bind-t2", "t2");
    let (_, g) = rig.issue_grant("t1", REF, "lab", 60);
    let grant = g["grant_ref"].as_str().unwrap();
    let open = |tenant: &str, binding: &str, grant: Option<&str>| rig.lab("POST", "/lab/sessions", Some(&json!({"binding_ref": binding})), tenant, grant);
    assert_eq!(open("t1", "bind-other", Some(grant)).1["code"], "grant_binding_mismatch");
    assert_eq!(open("t2", "bind-t2", Some(grant)).1["code"], "grant_denied"); // another tenant's grant looks unknown
    assert_eq!(open("t1", REF, None).1["code"], "grant_required");
    assert_eq!(open("t1", REF, Some("grant:nope")).1["code"], "grant_denied");
    let (_, wiki_grant) = rig.issue_grant("t1", REF, "wiki", 60);
    assert_eq!(open("t1", REF, wiki_grant["grant_ref"].as_str()).1["code"], "grant_scope_mismatch");
    assert_eq!(open("t1", REF, Some(grant)).0, 200);
}

#[test]
fn issuing_needs_the_issue_scope_a_binding_of_the_tenant_and_a_bounded_ttl() {
    let rig = rig_with_lab("issue", &[]);
    rig.bind_ref("bind-t2", "t2");
    assert_eq!(rig.issue_grant("t1", "bind-t2", "lab", 60).1["code"], "binding_unknown");
    assert_eq!(rig.issue_grant("t1", "missing", "lab", 60).0, 403);
    assert_eq!(rig.issue_grant("t1", REF, "lab", 0).0, 422);
    assert_eq!(rig.issue_grant("t1", REF, "lab", 86_400).0, 422);
    assert_eq!(rig.issue_grant("t1", REF, "sql", 60).0, 422);
    let lab_only = rig.lab_token("t1", "lab");
    let (st, v) = rig.call("POST", &format!("{BROKER}/grants"), Some(&json!({"binding_ref": REF, "scope": "lab", "ttl_seconds": 60})), Some(&lab_only), &[]);
    assert_eq!((st, v["code"].as_str()), (403, Some("pulso:auth_scope_denied")), "{v}");
}

#[test]
fn a_revoked_grant_stops_an_open_session_and_only_its_tenant_can_revoke() {
    let rig = rig_with_lab("revoke", &[("recurrence_rate", "w1", "g_a", 3, 12)]);
    let (grant, sid) = rig.open_session("t1", REF, 600);
    assert_eq!(query(&rig, &sid, "k1", "recurrence_rate", "w1").0, 202);
    let revoke = |tenant: &str| rig.call("POST", &format!("{BROKER}/grants/{grant}/revoke"), None, Some(&rig.lab_token(tenant, "grant_issue")), &[]);
    assert_eq!(revoke("t2").0, 404);
    assert_eq!(revoke("t1").1["state"], "revoked");
    assert_eq!(revoke("t1").0, 200); // idempotent
    let (st, v) = query(&rig, &sid, "k2", "recurrence_rate", "w1");
    assert_eq!((st, v["code"].as_str()), (403, Some("grant_revoked")), "{v}");
    assert_eq!(rig.lab("GET", "/lab/queries/q-k1", None, "t1", None).1["code"], "grant_revoked", "reads of earlier results stop too");
}

#[test]
fn query_receipt_for_a_sqlite_lab_query_serves_aggregates_never_raw_rows() {
    // 1/8 and 3/8 pin the half-even rounding of the Python lab (0.125 -> 0.12, 0.375 -> 0.38); the last row is below k
    // (a tampered lab file): the broker withholds it whatever the file says.
    let rig = rig_with_lab("agg", &[("recurrence_rate", "w1", "g_a", 1, 8), ("recurrence_rate", "w1", "g_b", 3, 8), ("recurrence_rate", "w1", "g_c", 2, 11), ("recurrence_rate", "w1", "g_tiny", 1, 3), ("recurrence_rate", "w2", "g_a", 5, 20)]);
    // k floor 10: the first two rows (count 8) are below it as well, only g_c and w2 qualify
    let (_, sid) = rig.open_session("t1", REF, 600);
    let (st, q) = query(&rig, &sid, "key-0001", "recurrence_rate", "w1");
    assert_eq!(st, 202, "{q}");
    let qref = q["query_ref"].as_str().unwrap();
    let (st, state) = rig.lab("GET", &format!("/lab/queries/{qref}"), None, "t1", None);
    assert_eq!((st, state["state"].as_str()), (200, Some("completed")), "{state}");
    let (st, res) = rig.lab("GET", &format!("/lab/results/{}", state["result_ref"].as_str().unwrap()), None, "t1", None);
    assert_eq!(st, 200, "{res}");
    let cols: Vec<&str> = res["columns"].as_array().unwrap().iter().map(|c| c["name"].as_str().unwrap()).collect();
    assert_eq!(cols, ["metric_id", "window_id", "g_group", "count", "rate", "evidence_ref"]);
    assert_eq!(res["rows"].as_array().unwrap().len(), 1, "{res}");
    assert_eq!(res["rows"][0][2], "g_c");
    assert_eq!(res["rows"][0][3], 11);
    assert_eq!(res["rows"][0][4], json!(0.18)); // 2/11 = 0.1818..
    assert!(!res.to_string().contains("numerator"), "the numerator never leaves the lab");
    assert_eq!(res["total_rows"], 1);
    // the QueryReceipt (DebugApi shape) states what was run, how many rows, and what was withheld
    let (st, rc) = rig.lab("GET", &format!("/lab/receipts/{}", res["receipt_ref"].as_str().unwrap()), None, "t1", None);
    assert_eq!(st, 200, "{rc}");
    assert_eq!((rc["rows"].as_i64(), rc["truncated"].as_bool(), rc["outcome"].as_str()), (Some(1), Some(false), Some("ok")), "{rc}");
    assert_eq!(rc["param_classes"], json!(["metric_id", "window_id"]));
    assert_eq!(rc["quality_findings"], json!(["rows_below_k_withheld:3"]));
    assert!(rc["sql_sanitized"].as_str().unwrap().contains('?') && !rc["sql_sanitized"].as_str().unwrap().contains("recurrence_rate"));
    assert_eq!(rc["sql_digest"].as_str().unwrap().len(), 64);
    assert_eq!(rc["result_digest"], res["result_digest"]);
    assert_eq!((rc["k"].as_i64(), rc["grant_ref"].is_string()), (Some(10), true));
}

#[test]
fn half_even_rate_matches_the_python_lab() {
    let rig = Rig::with(|c| {
        c.labs.insert("t1".into(), make_lab("rounding", 1, &[("recurrence_rate", "w1", "g_a", 1, 8), ("recurrence_rate", "w1", "g_b", 3, 8)]));
        c.min_k = 1;
    });
    rig.bind_ref(REF, "t1");
    let (_, sid) = rig.open_session("t1", REF, 600);
    let (_, q) = query(&rig, &sid, "key-0002", "recurrence_rate", "w1");
    let (_, st) = rig.lab("GET", &format!("/lab/queries/{}", q["query_ref"].as_str().unwrap()), None, "t1", None);
    let (_, res) = rig.lab("GET", &format!("/lab/results/{}", st["result_ref"].as_str().unwrap()), None, "t1", None);
    let rates: Vec<f64> = res["rows"].as_array().unwrap().iter().map(|r| r[4].as_f64().unwrap()).collect();
    assert_eq!(rates, [0.12, 0.38]);
}

#[test]
fn raw_sql_and_unknown_shapes_are_refused_and_queries_are_idempotent_by_key() {
    let rig = rig_with_lab("shape", &[("recurrence_rate", "w1", "g_a", 3, 12)]);
    let (_, sid) = rig.open_session("t1", REF, 600);
    let path = format!("/lab/sessions/{sid}/queries");
    let (st, v) = rig.lab("POST", &path, Some(&json!({"query_key": "k", "sql": "select * from lab_rows"})), "t1", None);
    assert_eq!((st, v["code"].as_str()), (422, Some("raw_sql_refused")), "{v}");
    assert_eq!(rig.lab("POST", &path, Some(&json!({"query_key": "k", "metric_id": "m"})), "t1", None).0, 422);
    let (s1, a) = query(&rig, &sid, "same-key", "recurrence_rate", "w1");
    let (s2, b) = query(&rig, &sid, "same-key", "recurrence_rate", "w1");
    assert_eq!((s1, s2, a["query_ref"].clone()), (202, 202, b["query_ref"].clone()));
    let (st, v) = query(&rig, &sid, "same-key", "recurrence_rate", "w9");
    assert_eq!((st, v["code"].as_str()), (409, Some("query_key_conflict")), "{v}");
}

#[test]
fn lab_data_is_tenant_scoped_and_a_tenant_without_a_lab_gets_lab_unavailable() {
    let rig = rig_with_lab("tenants", &[("recurrence_rate", "w1", "g_a", 3, 12)]);
    rig.bind_ref("bind-t2", "t2");
    let (_, sid) = rig.open_session("t1", REF, 600);
    let (_, q) = query(&rig, &sid, "k1", "recurrence_rate", "w1");
    let qref = q["query_ref"].as_str().unwrap();
    assert_eq!(rig.lab("GET", &format!("/lab/queries/{qref}"), None, "t2", None).0, 404);
    let (_, g) = rig.issue_grant("t2", "bind-t2", "lab", 60);
    let (st, v) = rig.lab("POST", "/lab/sessions", Some(&json!({"binding_ref": "bind-t2"})), "t2", g["grant_ref"].as_str());
    assert_eq!((st, v["code"].as_str()), (404, Some("lab_unavailable")), "{v}");
    assert_eq!(rig.lab("POST", &format!("/lab/sessions/{sid}/queries"), Some(&json!({"query_key": "x", "metric_id": "m", "window_id": "w"})), "t2", None).0, 404);
}

#[test]
fn wiki_read_is_tenant_scoped_and_confines_paths() {
    let rig = Rig::new();
    rig.admin(json!({"wiki": [{"tenant": "t1", "path": "runbooks/recurrence.md", "content": "t1 secret page"}]}));
    let read = |tenant: &str, paths: Value| rig.call("POST", &format!("{BROKER}/wiki/read"), Some(&json!({"paths": paths})), Some(&rig.lab_token(tenant, "wiki")), &[]);
    let (st, v) = read("t1", json!(["runbooks/recurrence.md"]));
    assert_eq!(st, 200, "{v}");
    assert_eq!((v["entries"][0]["path"].as_str(), v["entries"][0]["content"].as_str()), (Some("runbooks/recurrence.md"), Some("t1 secret page")));
    assert_eq!(v["entries"][0]["digest"].as_str().unwrap().len(), 64);
    let (st, v) = read("t2", json!(["runbooks/recurrence.md"]));
    assert_eq!((st, v["code"].as_str()), (404, Some("page_not_found")), "{v}");
    for bad in ["../x", "/etc/passwd", "a/../../b", "C:\\x", ""] {
        assert_eq!(read("t1", json!([bad])).1["code"], "path_invalid", "{bad}");
    }
    assert_eq!(rig.call("POST", &format!("{BROKER}/wiki/read"), Some(&json!({"paths": ["x"]})), Some(&rig.lab_token("t1", "lab")), &[]).0, 403);
}
