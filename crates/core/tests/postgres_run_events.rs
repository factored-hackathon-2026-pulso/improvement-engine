//! U07 durable run-event persistence boundary against the V2 schema.

use improvement_engine_core::durable_run_events::{
    DurableRunEvent, JobStatus, JobTransition, PostgresRunEventLedger, RunEventStatus,
};

const TENANT: &str = "tenant_a";
const RUN: &str = "00000000-0000-7000-8000-000000000002";
const RUN_2: &str = "00000000-0000-7000-8000-000000000008";
const CHILD_JOB: &str = "00000000-0000-7000-8000-000000000003";
const CHILD_JOB_2: &str = "00000000-0000-7000-8000-000000000006";
const EVENT_1: &str = "00000000-0000-7000-8000-000000000004";
const EVENT_2: &str = "00000000-0000-7000-8000-000000000005";
const EVENT_3: &str = "00000000-0000-7000-8000-000000000007";
static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn connect() -> postgres::Client {
    assert_eq!(
        std::env::var("PULSO_ALLOW_DESTRUCTIVE_TEST_DB").as_deref(),
        Ok("1")
    );
    let url = std::env::var("PULSO_TEST_POSTGRES_URL").unwrap();
    postgres::Client::connect(&url, postgres::NoTls).unwrap()
}

fn prepare() -> postgres::Client {
    let mut client = connect();
    client
        .batch_execute(include_str!(
            "../../../migrations/0003_pulso_run_events.sql"
        ))
        .unwrap();
    client
        .batch_execute(
            "DROP TRIGGER IF EXISTS pulso_test_reject_run_event ON pulso_run_events; \
             DROP FUNCTION IF EXISTS pulso_test_reject_run_event(); \
             TRUNCATE pulso_run_events, pulso_jobs CASCADE;",
        )
        .unwrap();
    client
}

fn seed_run(client: &mut postgres::Client) {
    client
        .execute(
            "INSERT INTO pulso_jobs (id, tenant_id, run_ref, kind, logical_key, generation, \
             parent_job_id, status, lane, priority, due_at, attempt, lease_version, \
             input_ref, config_ref) \
             VALUES ($1::uuid, $2, $1::uuid, 'detect', 'test-run', 0, $1::uuid, \
             'queued', 'default', 0, now(), 0, 0, 'input:test', 'config:test'), \
             ($3::uuid, $2, $1::uuid, 'verify', 'test-child', 0, $1::uuid, \
             'queued', 'default', 0, now(), 0, 0, 'input:test', 'config:test'), \
             ('00000000-0000-7000-8000-000000000006'::uuid, $2, $1::uuid, \
             'verify', 'test-child-2', 0, $1::uuid, 'queued', 'default', 0, now(), \
             0, 0, 'input:test', 'config:test'), \
             ('00000000-0000-7000-8000-000000000008'::uuid, $2, \
             '00000000-0000-7000-8000-000000000008'::uuid, 'detect', 'other-run', 0, \
             '00000000-0000-7000-8000-000000000008'::uuid, 'queued', 'default', 0, \
             now(), 0, 0, 'input:test', 'config:test')",
            &[&RUN, &TENANT, &CHILD_JOB],
        )
        .unwrap();
}

fn event(code: &str, next_status: JobStatus, event_status: &str) -> DurableRunEvent {
    event_with_id(EVENT_1, JobStatus::Queued, code, next_status, event_status)
}

fn event_with_id(
    id: &str,
    expected_status: JobStatus,
    code: &str,
    next_job_status: JobStatus,
    event_status: &str,
) -> DurableRunEvent {
    event_for_job(
        id,
        CHILD_JOB,
        expected_status,
        code,
        next_job_status,
        event_status,
    )
}

fn event_for_job(
    id: &str,
    job_ref: &str,
    expected_status: JobStatus,
    code: &str,
    next_job_status: JobStatus,
    event_status: &str,
) -> DurableRunEvent {
    DurableRunEvent::new(
        id,
        TENANT,
        RUN,
        Some(JobTransition::new(job_ref, expected_status, next_job_status).unwrap()),
        "execution",
        code,
        RunEventStatus::new(event_status).unwrap(),
        Some("test-reason".into()),
        None,
        None,
        None,
    )
    .unwrap()
}

#[test]
#[ignore = "requires isolated PULSO_TEST_POSTGRES_URL and explicit destructive-test consent"]
fn concurrent_jobs_in_one_run_get_distinct_contiguous_sequences() {
    let _guard = TEST_LOCK.lock().unwrap();
    let mut client = prepare();
    seed_run(&mut client);
    drop(client);
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let mut workers = Vec::new();
    for (event_id, job_id) in [(EVENT_1, CHILD_JOB), (EVENT_3, CHILD_JOB_2)] {
        let barrier = barrier.clone();
        workers.push(std::thread::spawn(move || {
            let mut ledger = PostgresRunEventLedger::new(connect());
            let event = event_for_job(
                event_id,
                job_id,
                JobStatus::Queued,
                "job_claimed",
                JobStatus::Running,
                "running",
            );
            barrier.wait();
            ledger.append_transition(event).unwrap().sequence()
        }));
    }
    let mut sequences: Vec<_> = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect();
    sequences.sort_unstable();
    assert_eq!(sequences, [1, 2]);
}

#[test]
#[ignore = "requires isolated PULSO_TEST_POSTGRES_URL and explicit destructive-test consent"]
fn job_transition_and_run_event_commit_atomically_and_receive_monotonic_run_sequences() {
    let _guard = TEST_LOCK.lock().unwrap();
    let mut client = prepare();
    seed_run(&mut client);
    let mut ledger = PostgresRunEventLedger::new(client);

    let first = ledger
        .append_transition(event("job_claimed", JobStatus::Running, "running"))
        .unwrap();
    let second = ledger
        .append_transition(event_with_id(
            EVENT_2,
            JobStatus::Running,
            "job_completed",
            JobStatus::Complete,
            "completed",
        ))
        .unwrap();
    let run_level = ledger
        .append_transition(
            DurableRunEvent::new(
                EVENT_3,
                TENANT,
                RUN,
                None,
                "run",
                "run_completed",
                RunEventStatus::new("completed").unwrap(),
                None,
                None,
                None,
                None,
            )
            .unwrap(),
        )
        .unwrap();

    assert_eq!(
        (first.sequence(), second.sequence(), run_level.sequence()),
        (1, 2, 3)
    );
    assert_eq!(first.id(), EVENT_1);
    let mut client = connect();
    let status: String = client
        .query_one(
            "SELECT status FROM pulso_jobs WHERE tenant_id=$1 AND id=$2::uuid",
            &[&TENANT, &CHILD_JOB],
        )
        .unwrap()
        .get(0);
    let events = client
        .query(
            "SELECT sequence, event_code, status FROM pulso_run_events \
             WHERE tenant_id=$1 AND run_ref=$2::uuid ORDER BY sequence",
            &[&TENANT, &RUN],
        )
        .unwrap();
    assert_eq!(status, "complete");
    assert_eq!(events.len(), 3);
    assert_eq!(events[0].get::<_, i64>(0), 1);
    assert_eq!(events[1].get::<_, i64>(0), 2);
    assert_eq!(events[0].get::<_, String>(1), "job_claimed");
    assert_eq!(events[1].get::<_, String>(2), "completed");
    let root_sequence: i64 = client
        .query_one(
            "SELECT last_event_sequence FROM pulso_jobs WHERE tenant_id=$1 AND id=$2::uuid",
            &[&TENANT, &RUN],
        )
        .unwrap()
        .get(0);
    assert_eq!(root_sequence, 3);
}

#[test]
#[ignore = "requires isolated PULSO_TEST_POSTGRES_URL and explicit destructive-test consent"]
fn failed_event_append_rolls_back_the_job_transition() {
    let _guard = TEST_LOCK.lock().unwrap();
    let mut client = prepare();
    seed_run(&mut client);
    client
        .batch_execute(
            "CREATE OR REPLACE FUNCTION pulso_test_reject_run_event() RETURNS trigger \
             LANGUAGE plpgsql AS $$ BEGIN \
               IF NEW.event_code = 'force_insert_failure' THEN \
                 RAISE EXCEPTION 'injected append failure'; \
               END IF; RETURN NEW; END $$; \
             CREATE TRIGGER pulso_test_reject_run_event \
             BEFORE INSERT ON pulso_run_events FOR EACH ROW \
             EXECUTE FUNCTION pulso_test_reject_run_event()",
        )
        .unwrap();
    let mut ledger = PostgresRunEventLedger::new(client);
    assert!(
        ledger
            .append_transition(event("force_insert_failure", JobStatus::Running, "running",))
            .is_err()
    );
    let mut client = connect();
    let status: String = client
        .query_one(
            "SELECT status FROM pulso_jobs WHERE tenant_id=$1 AND id=$2::uuid",
            &[&TENANT, &CHILD_JOB],
        )
        .unwrap()
        .get(0);
    let count: i64 = client
        .query_one(
            "SELECT count(*) FROM pulso_run_events WHERE tenant_id=$1 AND run_ref=$2::uuid",
            &[&TENANT, &RUN],
        )
        .unwrap()
        .get(0);
    assert_eq!(status, "queued");
    assert_eq!(count, 0);
    let root_sequence: i64 = client
        .query_one(
            "SELECT last_event_sequence FROM pulso_jobs WHERE tenant_id=$1 AND id=$2::uuid",
            &[&TENANT, &RUN],
        )
        .unwrap()
        .get(0);
    assert_eq!(root_sequence, 0);
    client
        .batch_execute(
            "DROP TRIGGER pulso_test_reject_run_event ON pulso_run_events; \
             DROP FUNCTION pulso_test_reject_run_event()",
        )
        .unwrap();
}

#[test]
#[ignore = "requires isolated PULSO_TEST_POSTGRES_URL and explicit destructive-test consent"]
fn run_rejects_a_child_as_root_and_rejects_a_job_from_another_run() {
    let _guard = TEST_LOCK.lock().unwrap();
    let mut client = prepare();
    seed_run(&mut client);
    let mut ledger = PostgresRunEventLedger::new(client);

    let child_as_root = DurableRunEvent::new(
        EVENT_1,
        TENANT,
        CHILD_JOB,
        None,
        "run",
        "run_started",
        RunEventStatus::new("running").unwrap(),
        None,
        None,
        None,
        None,
    )
    .unwrap();
    assert!(matches!(
        ledger.append_transition(child_as_root),
        Err(improvement_engine_core::durable_run_events::RunEventError::RunNotFound)
    ));

    let cross_run_event = DurableRunEvent::new(
        EVENT_2,
        TENANT,
        RUN_2,
        Some(JobTransition::new(CHILD_JOB, JobStatus::Queued, JobStatus::Running).unwrap()),
        "execution",
        "job_claimed",
        RunEventStatus::new("running").unwrap(),
        None,
        None,
        None,
        None,
    )
    .unwrap();
    assert!(matches!(
        ledger.append_transition(cross_run_event),
        Err(improvement_engine_core::durable_run_events::RunEventError::JobNotInRun)
    ));
    let mut client = connect();
    let child_as_root_insert = client.execute(
        "INSERT INTO pulso_run_events \
         (id, tenant_id, run_ref, sequence, event_at, stage, event_code, status) \
         VALUES ($1::uuid, $2, $3::uuid, 1, now(), 'run', 'run_started', 'running')",
        &[&EVENT_3, &TENANT, &CHILD_JOB],
    );
    assert!(child_as_root_insert.is_err());
    let cross_run_job_insert = client.execute(
        "INSERT INTO pulso_run_events \
         (id, tenant_id, run_ref, job_ref, sequence, event_at, stage, event_code, status) \
         VALUES ($1::uuid, $2, $3::uuid, $4::uuid, 1, now(), 'execution', \
                 'job_claimed', 'running')",
        &[&EVENT_3, &TENANT, &RUN, &RUN_2],
    );
    assert!(cross_run_job_insert.is_err());
    let sequences: Vec<i64> = client
        .query(
            "SELECT last_event_sequence FROM pulso_jobs \
             WHERE tenant_id=$1 AND id IN ($2::uuid, $3::uuid) ORDER BY id",
            &[&TENANT, &RUN, &RUN_2],
        )
        .unwrap()
        .iter()
        .map(|row| row.get(0))
        .collect();
    let events: i64 = client
        .query_one(
            "SELECT count(*) FROM pulso_run_events WHERE tenant_id=$1",
            &[&TENANT],
        )
        .unwrap()
        .get(0);
    assert_eq!(sequences, [0, 0]);
    assert_eq!(events, 0);
}
