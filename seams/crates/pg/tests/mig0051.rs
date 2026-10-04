//! 0051 is additive and idempotent on top of 0001-0004 + 0050 (runner), compatible with pre-existing rows.
mod common;
use common::{migrations_dir, TempDb};

#[test]
fn migration_0051_is_additive_idempotent_and_keeps_existing_rows() {
    let Some(db) = TempDb::create() else { return };
    let mut c = db.connect();
    let all = pg::migrate::discover(&migrations_dir()).unwrap();
    let mut upto50 = all.clone();
    upto50.retain(|m| m.version <= 50);
    pg::migrate::migrate(&mut c, &upto50).unwrap();
    // a job exactly as Codex's tests/jobs create it: no knowledge of effect_state
    c.batch_execute(
        "INSERT INTO pulso_jobs (id, tenant_id, run_ref, kind, logical_key, generation, parent_job_id, status, lane, due_at, input_ref, config_ref) \
         VALUES ('01890000-0000-7000-8000-000000000001', 't', '01890000-0000-7000-8000-000000000001', 'k', 'lk', 0, '01890000-0000-7000-8000-000000000001', 'complete', 'l', now(), 'i', 'c')",
    )
    .unwrap();
    let before: Vec<String> = c.query("SELECT column_name FROM information_schema.columns WHERE table_name = 'pulso_jobs' ORDER BY ordinal_position", &[]).unwrap().iter().map(|r| r.get(0)).collect();
    pg::migrate::migrate(&mut c, &all).unwrap();
    let after: Vec<String> = c.query("SELECT column_name FROM information_schema.columns WHERE table_name = 'pulso_jobs' ORDER BY ordinal_position", &[]).unwrap().iter().map(|r| r.get(0)).collect();
    assert_eq!(&after[..before.len()], &before[..], "existing columns were renamed/reordered/dropped");
    assert_eq!(&after[before.len()..], ["effect_state"]);
    let es: String = c.query_one("SELECT effect_state FROM pulso_jobs", &[]).unwrap().get(0);
    assert_eq!(es, "no_effect");
    // re-running the raw SQL (runner bypassed) twice changes nothing and fails nothing
    let sql = std::fs::read_to_string(migrations_dir().join("0051_pulso_job_claim_commit.sql")).unwrap();
    c.batch_execute(&sql).unwrap();
    c.batch_execute(&sql).unwrap();
    assert_eq!(c.query_one("SELECT count(*) FROM pulso_jobs", &[]).unwrap().get::<_, i64>(0), 1);
    assert!(c.batch_execute("UPDATE pulso_jobs SET effect_state = 'bogus'").is_err());
}
