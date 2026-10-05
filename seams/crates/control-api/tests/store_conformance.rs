//! CPG: one conformance suite for the `Store` port, run against BOTH `MemStore` and `PgStore` (parametrised like PGC).
//! `PgStore` runs on a throw-away database of `PULSO_TEST_PG_ADMIN` (skipped, with a SKIP line, when unset;
//! `PULSO_REQUIRE_POSTGRES=1` turns the absence into a failure). Restart persistence only applies to durable stores.
mod common;
use common::*;
use control_api::pgstore::PgStore;
use core_client::canon::sha256_hex;
use control_api::store::{BindingRec, MemStore, PutOutcome, Store};
use postgres::{Client, Config, NoTls};
use serde_json::{Value, json};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

struct TempDb {
    admin: Config,
    name: String,
}
static N: AtomicU32 = AtomicU32::new(0);

impl TempDb {
    fn create() -> Option<TempDb> {
        let Ok(dsn) = std::env::var("PULSO_TEST_PG_ADMIN") else {
            assert!(std::env::var("PULSO_REQUIRE_POSTGRES").is_err(), "PULSO_TEST_PG_ADMIN not set");
            eprintln!("SKIP: PULSO_TEST_PG_ADMIN not set (PgStore conformance blocked)");
            return None;
        };
        let admin: Config = dsn.parse().expect("admin dsn");
        let name = format!("cpg_{}_{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst));
        admin.connect(NoTls).expect("admin connect").batch_execute(&format!("CREATE DATABASE {name}")).unwrap();
        Some(TempDb { admin, name })
    }
    /// Connection URL of the throw-away database, rebuilt from the admin DSN (password included, never printed).
    fn url(&self) -> String {
        let dsn = std::env::var("PULSO_TEST_PG_ADMIN").unwrap();
        let (head, _) = dsn.rsplit_once('/').unwrap();
        format!("{head}/{}", self.name)
    }
    fn client(&self) -> Client {
        let mut c = self.admin.clone();
        c.dbname(&self.name);
        c.connect(NoTls).unwrap()
    }
}
impl Drop for TempDb {
    fn drop(&mut self) {
        let _ = self.admin.connect(NoTls).and_then(|mut c| c.batch_execute(&format!("DROP DATABASE IF EXISTS {} WITH (FORCE)", self.name)));
    }
}

struct Fixture {
    name: &'static str,
    durable: bool,
    db: Option<TempDb>,
    shared: Arc<MemStore>,
}

impl Fixture {
    /// A store handle on the fixture's state: a new connection for Postgres, the one shared instance for memory.
    fn open(&self) -> Arc<dyn Store> {
        match &self.db {
            Some(db) => Arc::new(PgStore::connect(&db.url()).expect("connect PgStore")),
            None => self.shared.clone(),
        }
    }
    fn boxed(&self) -> Box<dyn Store> {
        match &self.db {
            Some(db) => Box::new(PgStore::connect(&db.url()).expect("connect PgStore")),
            None => Box::new(Shared(self.shared.clone())),
        }
    }
}

/// `Box<dyn Store>` view of the shared in-memory store (its Arc stays with the fixture).
struct Shared(Arc<MemStore>);
impl Store for Shared {
    fn binding(&self, t: &str, k: &str) -> Option<BindingRec> {
        self.0.binding(t, k)
    }
    fn job_owner(&self, t: &str, j: &str) -> Option<String> {
        self.0.job_owner(t, j)
    }
    fn put_binding(&self, t: &str, k: &str, r: BindingRec) -> bool {
        self.0.put_binding(t, k, r)
    }
    fn binding_ref_tenant(&self, r: &str) -> Option<String> {
        self.0.binding_ref_tenant(r)
    }
    fn preauthorize_binding_ref(&self, r: &str, t: &str) {
        self.0.preauthorize_binding_ref(r, t)
    }
    fn binding_effects(&self, t: &str, j: &str) -> u32 {
        self.0.binding_effects(t, j)
    }
    fn put_artifact(&self, t: &str, e: Value) -> PutOutcome {
        self.0.put_artifact(t, e)
    }
    fn get_artifact(&self, t: &str, i: &str) -> Option<Value> {
        self.0.get_artifact(t, i)
    }
    fn put_doc(&self, n: &str, t: &str, i: &str, d: Value) {
        self.0.put_doc(n, t, i, d)
    }
    fn put_doc_new(&self, n: &str, t: &str, i: &str, d: Value) -> bool {
        self.0.put_doc_new(n, t, i, d)
    }
    fn get_doc(&self, n: &str, t: &str, i: &str) -> Option<Value> {
        self.0.get_doc(n, t, i)
    }
    fn list_docs(&self, n: &str, t: &str) -> Vec<(String, Value)> {
        self.0.list_docs(n, t)
    }
    fn jti_claim(&self, s: &str, i: &str, j: &str, e: f64, n: f64) -> bool {
        self.0.jti_claim(s, i, j, e, n)
    }
    fn durable_replay(&self) -> bool {
        self.0.durable_replay()
    }
}

fn each(body: impl Fn(&Fixture)) {
    body(&Fixture { name: "MemStore", durable: false, db: None, shared: Arc::new(MemStore::default()) });
    if let Some(db) = TempDb::create() {
        let fx = Fixture { name: "PgStore", durable: true, db: Some(db), shared: Arc::new(MemStore::default()) };
        body(&fx);
    }
}

fn rec(job: &str, rf: &str) -> BindingRec {
    BindingRec { request_digest: "d1".into(), job_id: job.into(), core_run_id: "run-1".into(), attempt: 2, task_binding_ref: rf.into() }
}

fn envelope(id: &str, media: &str, content: &str) -> Value {
    json!({"schema_version": "1", "artifact": {"id": id, "digest": "abc", "media_type": media}, "encoding": "utf8", "content": content, "byte_length": content.len()})
}

#[test]
fn bindings_are_exactly_once_under_8_threads() {
    each(|fx| {
        let wins: usize = (0..8)
            .map(|_| fx.open())
            .map(|s| std::thread::spawn(move || s.put_binding("t1", "cmd-1", rec("job-1", "ref-1"))))
            .collect::<Vec<_>>()
            .into_iter()
            .map(|h| usize::from(h.join().unwrap()))
            .sum();
        let s = fx.open();
        assert_eq!(wins, 1, "{}: exactly one thread wins the binding", fx.name);
        assert_eq!(s.binding_effects("t1", "job-1"), 1, "{}", fx.name);
        assert_eq!(s.binding("t1", "cmd-1"), Some(rec("job-1", "ref-1")), "{}", fx.name);
        assert_eq!(s.job_owner("t1", "job-1").as_deref(), Some("cmd-1"), "{}", fx.name);
        assert_eq!(s.binding_ref_tenant("ref-1").as_deref(), Some("t1"), "{}", fx.name);
    });
}

#[test]
fn a_job_belongs_to_one_command_key_and_a_ref_to_one_tenant() {
    each(|fx| {
        let s = fx.open();
        assert!(s.put_binding("t1", "cmd-1", rec("job-1", "ref-1")), "{}", fx.name);
        assert!(!s.put_binding("t1", "cmd-2", rec("job-1", "ref-2")), "{}: same job, other key", fx.name);
        assert!(s.binding("t1", "cmd-2").is_none() && s.binding_ref_tenant("ref-2").is_none(), "{}: nothing written", fx.name);
        assert!(!s.put_binding("t2", "cmd-9", rec("job-9", "ref-1")), "{}: ref owned by t1", fx.name);
        assert!(s.binding("t2", "cmd-9").is_none() && s.job_owner("t2", "job-9").is_none(), "{}", fx.name);
        s.preauthorize_binding_ref("ref-1", "t2"); // never re-assigns
        assert_eq!(s.binding_ref_tenant("ref-1").as_deref(), Some("t1"), "{}", fx.name);
        s.preauthorize_binding_ref("ref-pre", "t2");
        assert_eq!(s.binding_ref_tenant("ref-pre").as_deref(), Some("t2"), "{}", fx.name);
    });
}

#[test]
fn bindings_are_tenant_scoped() {
    each(|fx| {
        let s = fx.open();
        assert!(s.put_binding("t1", "cmd-1", rec("job-1", "ref-1")));
        assert!(s.binding("t2", "cmd-1").is_none() && s.job_owner("t2", "job-1").is_none() && s.binding_effects("t2", "job-1") == 0, "{}", fx.name);
        assert!(s.put_binding("t2", "cmd-1", rec("job-1", "ref-t2")), "{}: same key and job id in another tenant is independent", fx.name);
        assert_eq!(s.binding("t1", "cmd-1").unwrap().task_binding_ref, "ref-1", "{}: t2 cannot overwrite t1", fx.name);
    });
}

#[test]
fn artifacts_are_idempotent_by_id_and_conflict_on_a_different_ref() {
    each(|fx| {
        let s = fx.open();
        let e = envelope("artifact:aa", "text/plain", "hello");
        assert_eq!(s.put_artifact("t1", e.clone()), PutOutcome::Created, "{}", fx.name);
        assert_eq!(s.put_artifact("t1", e.clone()), PutOutcome::Exists, "{}", fx.name);
        assert_eq!(s.put_artifact("t1", envelope("artifact:aa", "application/json", "hello")), PutOutcome::Conflict, "{}", fx.name);
        assert_eq!(s.get_artifact("t1", "artifact:aa"), Some(e), "{}: the first write stands", fx.name);
        assert!(s.get_artifact("t1", "artifact:zz").is_none());
    });
}

#[test]
fn artifacts_are_tenant_scoped_no_cross_read_or_overwrite() {
    each(|fx| {
        let s = fx.open();
        let e1 = envelope("artifact:aa", "text/plain", "tenant one");
        assert_eq!(s.put_artifact("t1", e1.clone()), PutOutcome::Created);
        assert!(s.get_artifact("t2", "artifact:aa").is_none(), "{}: cross-tenant read", fx.name);
        let e2 = envelope("artifact:aa", "application/json", "tenant two");
        assert_eq!(s.put_artifact("t2", e2.clone()), PutOutcome::Created, "{}: independent namespace", fx.name);
        assert_eq!(s.get_artifact("t1", "artifact:aa"), Some(e1), "{}: cross-tenant overwrite", fx.name);
        assert_eq!(s.get_artifact("t2", "artifact:aa"), Some(e2));
    });
}

#[test]
fn docs_are_namespaced_tenant_scoped_and_listed_by_id() {
    each(|fx| {
        let s = fx.open();
        for id in ["b", "a", "B", "a/1"] {
            s.put_doc("ingest_ledger", "t1", id, json!({"id": id}));
        }
        s.put_doc("grant", "t1", "a", json!({"g": 1}));
        s.put_doc("ingest_ledger", "t2", "a", json!({"other": true}));
        let ids: Vec<String> = s.list_docs("ingest_ledger", "t1").into_iter().map(|(i, _)| i).collect();
        assert_eq!(ids, ["B", "a", "a/1", "b"], "{}: byte order", fx.name);
        s.put_doc("ingest_ledger", "t1", "a", json!({"id": "a", "v": 2}));
        assert_eq!(s.get_doc("ingest_ledger", "t1", "a"), Some(json!({"id": "a", "v": 2})), "{}: put replaces", fx.name);
        assert_eq!(s.get_doc("grant", "t1", "a"), Some(json!({"g": 1})));
        assert_eq!(s.get_doc("ingest_ledger", "t2", "a"), Some(json!({"other": true})), "{}: t1 write never touches t2", fx.name);
        assert!(s.get_doc("ingest_ledger", "t3", "a").is_none() && s.list_docs("ingest_ledger", "t3").is_empty(), "{}", fx.name);
    });
}

#[test]
fn jti_replay_set_claims_once_scopes_apart_and_evicts_expired() {
    each(|fx| {
        let s = fx.open();
        assert!(s.jti_claim("control", "iss", "j1", 100.0, 50.0), "{}", fx.name);
        assert!(!s.jti_claim("control", "iss", "j1", 100.0, 51.0), "{}: replay", fx.name);
        assert!(s.jti_claim("broker", "iss", "j1", 100.0, 51.0), "{}: other verifier scope", fx.name);
        assert!(s.jti_claim("control", "other", "j1", 100.0, 51.0), "{}: other issuer", fx.name);
        assert!(s.jti_claim("control", "iss", "j1", 400.0, 300.0), "{}: the expired entry was evicted (after exp + skew)", fx.name);
        assert!(!s.jti_claim("control", "iss", "j1", 400.0, 301.0));
    });
}

#[test]
fn state_survives_dropping_and_reopening_the_store() {
    each(|fx| {
        if !fx.durable {
            return;
        }
        let s = fx.open();
        assert!(s.put_binding("t1", "cmd-1", rec("job-1", "ref-1")));
        assert_eq!(s.put_artifact("t1", envelope("artifact:aa", "text/plain", "x")), PutOutcome::Created);
        s.put_doc("grant", "t1", "g1", json!({"revoked": true}));
        assert!(s.jti_claim("control", "iss", "j1", 1e12, 1.0));
        drop(s);
        let s = fx.open();
        assert_eq!(s.binding("t1", "cmd-1"), Some(rec("job-1", "ref-1")));
        assert_eq!(s.binding_effects("t1", "job-1"), 1);
        assert_eq!(s.binding_ref_tenant("ref-1").as_deref(), Some("t1"));
        assert_eq!(s.put_artifact("t1", envelope("artifact:aa", "text/plain", "x")), PutOutcome::Exists);
        assert_eq!(s.get_doc("grant", "t1", "g1"), Some(json!({"revoked": true})));
        assert!(!s.jti_claim("control", "iss", "j1", 1e12, 2.0), "replay across a reopen is rejected");
        assert!(s.durable_replay());
    });
}

fn rig(fx: &Fixture) -> Rig {
    Rig::with_store(|_| {}, fx.boxed())
}

#[test]
fn http_binding_artifact_and_409_paths_behave_the_same_on_every_store() {
    each(|fx| {
        let rig = rig(fx);
        let tok = |purpose: &str| rig.token("cb", "control-api", "binding", "t1", json!({"purpose": purpose}));
        let body = json!({"tenant_id": "t1", "command_key": "k1", "request_digest": "d1", "job_id": "job-1", "core_run_id": "r1", "attempt": 1, "task_binding_ref": "tb-1"});
        let post = |b: &Value| rig.call("POST", "/internal/v1/core-task-bindings", Some(b), Some(&tok("core_task_binding")), &[("Idempotency-Key", b["command_key"].as_str().unwrap())]);
        assert_eq!(post(&body).0, 200, "{}", fx.name);
        assert_eq!(post(&body).0, 200, "{}: replay confirms again", fx.name);
        let mut changed = body.clone();
        changed["request_digest"] = json!("d2");
        assert_eq!(post(&changed).1["code"], "digest_mismatch", "{}", fx.name);
        let mut other = body.clone();
        other["command_key"] = json!("k2");
        assert_eq!(post(&other).1["code"], "binding_conflict", "{}", fx.name);
        let r = rig.upload_schema();
        assert_eq!(r["id"].as_str().map(|s| s.starts_with("artifact:")), Some(true), "{}", fx.name);
    });
}

#[test]
fn quarantine_keeps_metadata_only_on_every_store() {
    each(|fx| {
        let rig = rig(fx);
        let schema = rig.upload_schema();
        let events = vec![domain(1, "team.created", &schema), domain(2, "case.viewed", &schema)];
        let (st, r) = rig.post_batch(&batch(B { revision: Some(0), from: Some(1), to: Some(2), cursor: "s.2".into(), events }));
        assert!(st == 200 || st == 202, "{st} {r}");
        assert_eq!(r["quarantined_event_count"].as_i64(), Some(1), "{}", fx.name);
        let tok = rig.token("ob", "control-api", "observations", "t1", json!({"purpose": "platform_observations"}));
        let (_, q) = rig.call("GET", "/internal/v1/platform/quarantine", None, Some(&tok), &[]);
        assert_eq!(q["items"].as_array().map(Vec::len), Some(1), "{}", fx.name);
        assert!(!q.to_string().contains("DO-NOT-STORE"), "{}", fx.name);
        if let Some(db) = &fx.db {
            let mut c = db.client();
            let all: String = c.query("SELECT doc::text FROM pulso_ca_docs", &[]).unwrap().iter().map(|r| r.get::<_, String>(0)).collect();
            let art: String = c.query("SELECT envelope::text FROM pulso_ca_artifacts", &[]).unwrap().iter().map(|r| r.get::<_, String>(0)).collect();
            assert!(!all.contains("DO-NOT-STORE") && !art.contains("DO-NOT-STORE"), "no raw payload reaches the database");
            assert!(all.contains("event_type_not_admitted"));
        }
    });
}

#[test]
fn jti_replay_is_rejected_across_an_app_restart_on_a_durable_store() {
    each(|fx| {
        if !fx.durable {
            return;
        }
        let first = rig(fx);
        let tok = first.token("cb", "control-api", "binding", "t1", json!({"purpose": "core_task_binding"}));
        let body = json!({"tenant_id": "t1", "command_key": "k1", "request_digest": "d1", "job_id": "job-1", "core_run_id": "r1", "attempt": 1, "task_binding_ref": "tb-1"});
        let send = |rig: &Rig| rig.call("POST", "/internal/v1/core-task-bindings", Some(&body), Some(&tok), &[("Idempotency-Key", "k1")]);
        assert_eq!(send(&first).0, 200);
        assert_eq!(send(&first).1["code"], "pulso:auth_jti_replayed");
        drop(first);
        let second = rig(fx); // the "restarted" process: new App, same database, same clock
        let (st, v) = send(&second);
        assert_eq!((st, v["code"].as_str()), (401, Some("pulso:auth_jti_replayed")), "{v}");
    });
}

#[test]
fn reset_empties_every_table_for_a_fresh_black_box_run() {
    let Some(db) = TempDb::create() else { return };
    let s = PgStore::connect(&db.url()).unwrap();
    assert!(s.put_binding("t1", "cmd-1", rec("job-1", "ref-1")));
    s.put_doc("grant", "t1", "g1", json!({}));
    assert!(s.jti_claim("control", "iss", "j1", 1e12, 1.0));
    s.reset();
    assert!(s.binding("t1", "cmd-1").is_none() && s.binding_ref_tenant("ref-1").is_none() && s.get_doc("grant", "t1", "g1").is_none());
    assert!(s.jti_claim("control", "iss", "j1", 1e12, 2.0), "the replay set is emptied too");
}

// ---- CPGR review: adversarial cases -----------------------------------------------------------------------------

#[test]
fn jti_entry_is_kept_for_clock_skew_between_processes_after_exp() {
    each(|fx| {
        let s = fx.open();
        assert!(s.jti_claim("control", "iss", "j1", 1100.0, 1000.0), "{}", fx.name);
        // another process whose clock is ahead evicts "expired" rows ...
        assert!(s.jti_claim("control", "iss", "j2", 1400.0, 1100.0));
        // ... but a process whose clock is 50 s behind still accepts j1 (exp > its now): it must stay claimed
        assert!(!s.jti_claim("control", "iss", "j1", 1100.0, 1050.0), "{}: replay inside the skew window", fx.name);
    });
}

fn nul_upload(rig: &Rig) -> (u16, Value) {
    let content = "a\u{0}b";
    let digest = sha256_hex(content.as_bytes());
    let body = json!({"schema_version": "1", "binding_ref": SUB, "source_schema_ref": null, "classification": "treated",
                      "information_partition": "p1", "artifact_kind": "schema", "media_type": "text/plain", "encoding": "utf8",
                      "content": content, "content_digest": digest});
    let tok = rig.token("ex", "lab-broker", "artifact_write", "t1", json!({"purpose": "artifact_upload"}));
    rig.call("POST", "/internal/v1/broker/artifacts", Some(&body), Some(&tok), &[("Idempotency-Key", &digest)])
}

#[test]
fn values_the_database_cannot_hold_are_a_4xx_never_a_panic() {
    each(|fx| {
        let rig = rig(fx);
        let out = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| nul_upload(&rig)));
        assert!(out.as_ref().is_ok_and(|(st, _)| (400..500).contains(st)), "{}: NUL in artifact content -> {:?}", fx.name, out.map(|o| o.0));
        let long = "k".repeat(300);
        let tok = rig.token("cb", "control-api", "binding", "t1", json!({"purpose": "core_task_binding"}));
        let body = json!({"tenant_id": "t1", "command_key": long, "request_digest": "d", "job_id": "j", "core_run_id": "r", "attempt": 1, "task_binding_ref": "tb"});
        let out = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| rig.call("POST", "/internal/v1/core-task-bindings", Some(&body), Some(&tok), &[("Idempotency-Key", &long)])));
        assert!(out.as_ref().is_ok_and(|(st, _)| (400..500).contains(st)), "{}: 300-char command key -> {:?}", fx.name, out.map(|o| o.0));
        // the store still works afterwards (no poisoned/half-open connection)
        assert_eq!(rig.upload_schema()["id"].as_str().map(|s| s.starts_with("artifact:")), Some(true), "{}", fx.name);
    });
}

#[test]
fn concurrent_identical_binding_posts_from_separate_processes_all_confirm_once() {
    each(|fx| {
        let body = json!({"tenant_id": "t1", "command_key": "kr", "request_digest": "d1", "job_id": "job-r", "core_run_id": "r1", "attempt": 1, "task_binding_ref": "tb-r"});
        let rigs: Vec<Rig> = (0..6).map(|_| rig(fx)).collect(); // each rig = own App (own write lock) on its own connection
        let results: Vec<(u16, Value)> = std::thread::scope(|sc| {
            let hs: Vec<_> = rigs
                .iter()
                .map(|r| {
                    let b = &body;
                    sc.spawn(move || {
                        let tok = r.token("cb", "control-api", "binding", "t1", json!({"purpose": "core_task_binding"}));
                        r.call("POST", "/internal/v1/core-task-bindings", Some(b), Some(&tok), &[("Idempotency-Key", "kr")])
                    })
                })
                .collect();
            hs.into_iter().map(|h| h.join().unwrap()).collect()
        });
        assert!(results.iter().all(|(st, _)| *st == 200), "{}: {results:?}", fx.name);
        assert_eq!(fx.open().binding_effects("t1", "job-r"), 1, "{}", fx.name);
    });
}

#[test]
fn put_doc_new_has_one_winner_under_8_threads() {
    each(|fx| {
        let store = fx.open();
        let wins: usize = (0..8)
            .map(|i| {
                let s = store.clone();
                std::thread::spawn(move || s.put_doc_new("successor_run", "t1", "successor:rel-1", json!({"by": i})))
            })
            .collect::<Vec<_>>()
            .into_iter()
            .map(|h| usize::from(h.join().unwrap()))
            .sum();
        assert_eq!(wins, 1, "{}", fx.name);
        assert!(!store.put_doc_new("successor_run", "t1", "successor:rel-1", json!({})), "{}", fx.name);
        assert!(store.put_doc_new("successor_run", "t2", "successor:rel-1", json!({})), "tenant-scoped key: {}", fx.name);
    });
}

#[test]
fn every_namespace_the_server_writes_is_accepted_on_every_store() {
    // found live: 0052's closed namespace set lacked these four, so PgStore panicked on the first write (fixed by 0054)
    each(|fx| {
        let store = fx.open();
        for ns in [control_api::correlation::SUCCESSOR_NS, control_api::correlation::CORRELATION_NS, control_api::correlation::EVENT_NS, control_api::correlation::PUBLISHED_NS] {
            assert!(store.put_doc_new(ns, "t1", "k", json!({"ns": ns})), "{}: {ns}", fx.name);
            assert_eq!(store.get_doc(ns, "t1", "k"), Some(json!({"ns": ns})), "{}: {ns}", fx.name);
        }
    });
}
