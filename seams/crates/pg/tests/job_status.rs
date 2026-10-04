mod common;
use common::{migrations_dir, TempDb};
use pg::migrate;

const OLD: [&str; 9] =
    ["queued", "running", "retry_wait", "waiting_dependency", "complete", "deferred", "dead", "superseded", "cancelled"];
const NEW: [&str; 2] = ["leased", "waiting_human"];

fn status_def(c: &mut postgres::Client) -> String {
    c.query_one(
        "SELECT pg_get_constraintdef(oid) FROM pg_constraint WHERE conrelid = 'pulso_jobs'::regclass AND contype = 'c' \
         AND pg_get_constraintdef(oid) LIKE '%status%queued%'",
        &[],
    )
    .unwrap()
    .get(0)
}

#[test]
fn widening_keeps_every_existing_status_and_adds_new_ones() {
    let Some(db) = TempDb::create() else { return };
    let mut c = db.connect();
    let all = migrate::discover(&migrations_dir()).unwrap();
    let base: Vec<_> = all.iter().filter(|m| m.version < 50).cloned().collect();
    migrate::migrate(&mut c, &base).unwrap();
    let before = status_def(&mut c);
    for s in OLD {
        assert!(before.contains(&format!("'{s}'")), "{s}");
    }
    for s in NEW {
        assert!(!before.contains(&format!("'{s}'")), "{s} must be new");
    }
    migrate::migrate(&mut c, &all).unwrap();
    let after = status_def(&mut c);
    for s in OLD.iter().chain(NEW.iter()) {
        assert!(after.contains(&format!("'{s}'")), "{s}");
    }
    // a row with a legacy status and one with a new status both insert
    let id = "01890000-0000-7000-8000-000000000001";
    for (i, st) in ["queued", "leased"].iter().enumerate() {
        let jid = format!("0189000{i}-0000-7000-8000-000000000001");
        c.execute(
            "INSERT INTO pulso_jobs (id, tenant_id, run_ref, kind, logical_key, generation, parent_job_id, status, lane, due_at, input_ref, config_ref) \
             VALUES ($1::text::uuid, 't', $1::text::uuid, 'k', $2, 0, $1::text::uuid, $3, 'l', now(), 'i', 'c')",
            &[&jid, &format!("lk{i}"), st],
        )
        .unwrap_or_else(|e| panic!("{st}: {e} ({id})"));
    }
    assert!(c.execute(
        "INSERT INTO pulso_jobs (id, tenant_id, run_ref, kind, logical_key, generation, parent_job_id, status, lane, due_at, input_ref, config_ref) \
         VALUES ('01890009-0000-7000-8000-000000000001'::uuid, 't', '01890009-0000-7000-8000-000000000001'::uuid, 'k', 'z', 0, '01890009-0000-7000-8000-000000000001'::uuid, 'bogus', 'l', now(), 'i', 'c')", &[]).is_err());
    // idempotent
    assert!(migrate::migrate(&mut c, &all).unwrap().applied.is_empty());
}
