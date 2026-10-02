//! Durable temporal/event identity checks for U33 memory-use receipts.

use improvement_engine_core::{
    ArtifactDraft, ArtifactKind, ArtifactRepository, PostgresArtifactRepository,
};
use postgres::{Client, NoTls};
use serde_json::json;

const TENANT: &str = "tenant-p4";
const MEMORY: &str = "018f50a1-7f00-7000-8000-000000000071";
const MEMORY_B: &str = "018f50a1-7f00-7000-8000-000000000072";
const EVENT: &str = "platform-event-0001";
const CROSS_SCOPE_EVENT: &str = "platform-event-cross-scope-race";
const RECEIPT: &str = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const TEMPORAL: &str = "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const RACE_RECEIPT: &str =
    "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
const CROSS_SCOPE_RECEIPT_A: &str =
    "sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";
const CROSS_SCOPE_RECEIPT_B: &str =
    "sha256:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";

fn connect() -> Client {
    assert_eq!(
        std::env::var("PULSO_ALLOW_DESTRUCTIVE_TEST_DB").as_deref(),
        Ok("1")
    );
    Client::connect(
        &std::env::var("PULSO_TEST_POSTGRES_URL").expect("isolated test database"),
        NoTls,
    )
    .expect("isolated PostgreSQL connection")
}

#[test]
#[ignore = "requires isolated PULSO_TEST_POSTGRES_URL and explicit destructive-test consent"]
fn temporal_use_receipt_is_bound_to_event_scope_cutoff_and_current_head() {
    let mut client = connect();
    client
        .batch_execute(include_str!(
            "../../../migrations/0001_pulso_artifact_revisions.sql"
        ))
        .unwrap();
    client
        .batch_execute(include_str!(
            "../../../migrations/0002_pulso_memory_control.sql"
        ))
        .unwrap();
    client
        .batch_execute(include_str!(
            "../../../migrations/0003_pulso_run_events.sql"
        ))
        .unwrap();
    client
        .batch_execute(include_str!(
            "../../../migrations/0004_pulso_memory_temporal_receipts.sql"
        ))
        .unwrap();
    let public_execute_allowed: bool = client
        .query_one(
            concat!(
                "SELECT EXISTS (",
                " SELECT 1 FROM pg_proc p",
                " CROSS JOIN LATERAL aclexplode(COALESCE(p.proacl, acldefault('f', p.proowner))) acl",
                " WHERE p.oid = to_regprocedure('public.pulso_record_memory_use_temporal(text,text,text,text,text,text,text,text,text,uuid,bigint,text,bigint,text,text,bigint,bigint)')",
                " AND acl.grantee = 0 AND acl.privilege_type = 'EXECUTE'",
                ")"
            ),
            &[],
        )
        .unwrap()
        .get(0);
    assert!(
        !public_execute_allowed,
        "the SECURITY DEFINER temporal writer must not be executable by PUBLIC"
    );
    client.batch_execute("TRUNCATE pulso_memory_use_receipts, pulso_memory_tombstones, pulso_memory_lineage, pulso_memory_heads, pulso_artifact_heads, pulso_artifact_revisions").unwrap();

    let mut artifacts = PostgresArtifactRepository::new(client);
    let artifact = artifacts.append(None, ArtifactDraft::new(
        TENANT,
        MEMORY,
        1,
        ArtifactKind::MemoryWiki,
        json!({"available_at_unix_seconds": 100, "purpose": "investigation", "pages": {"index.md": "verified"}}),
        None,
    )).unwrap();
    let reference = artifact.reference();
    let artifact_b = artifacts.append(None, ArtifactDraft::new(
        TENANT,
        MEMORY_B,
        1,
        ArtifactKind::MemoryWiki,
        json!({"available_at_unix_seconds": 100, "purpose": "investigation", "pages": {"index.md": "other scope"}}),
        None,
    )).unwrap();
    let reference_b = artifact_b.reference();
    let mut client = artifacts.into_inner();
    client
        .query_one(
            "SELECT pulso_seed_memory_head($1,$2,$3,$4,$5,$6,$7::text::uuid,$8,$9)",
            &[
                &TENANT,
                &"investigation",
                &"world-a",
                &"campaign-a",
                &"continuous",
                &"train",
                &reference.id,
                &1_i64,
                &reference.digest,
            ],
        )
        .unwrap();
    client
        .query_one(
            "SELECT pulso_seed_memory_head($1,$2,$3,$4,$5,$6,$7::text::uuid,$8,$9)",
            &[
                &TENANT,
                &"investigation",
                &"world-b",
                &"campaign-a",
                &"continuous",
                &"train",
                &reference_b.id,
                &1_i64,
                &reference_b.digest,
            ],
        )
        .unwrap();

    // Two workers racing on one source event must converge on one durable
    // receipt. The row lock on the scoped head serializes these admissions.
    drop(client);
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let mut workers = Vec::new();
    for _ in 0..2 {
        let barrier = barrier.clone();
        let artifact_id = reference.id.clone();
        let artifact_digest = reference.digest.clone();
        workers.push(std::thread::spawn(move || {
            let mut client = connect();
            barrier.wait();
            client.query_one(
                "SELECT pulso_record_memory_use_temporal($1,$2,$3,$4,$5,$6,$7,$8,$9,$10::text::uuid,$11,$12,$13,$14,$15,$16,$17)",
                &[&TENANT, &RECEIPT, &TEMPORAL, &EVENT, &"investigation", &"world-a", &"campaign-a", &"continuous", &"train", &artifact_id, &1_i64, &artifact_digest, &1_i64, &"run-a", &"grant-a", &150_i64, &200_i64],
            )
        }));
    }
    for worker in workers {
        worker
            .join()
            .unwrap()
            .expect("racing exact retries converge");
    }
    let client = connect();

    // A tenant-scoped source event cannot be concurrently admitted under two
    // different scopes; exactly one writer may claim the global event key.
    drop(client);
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let mut workers = Vec::new();
    for (world, artifact_ref, run, receipt) in [
        (
            "world-a",
            reference.clone(),
            "run-cross-a",
            CROSS_SCOPE_RECEIPT_A,
        ),
        (
            "world-b",
            reference_b.clone(),
            "run-cross-b",
            CROSS_SCOPE_RECEIPT_B,
        ),
    ] {
        let barrier = barrier.clone();
        workers.push(std::thread::spawn(move || {
            let mut client = connect();
            barrier.wait();
            client.query_one(
                "SELECT pulso_record_memory_use_temporal($1,$2,$3,$4,$5,$6,$7,$8,$9,$10::text::uuid,$11,$12,$13,$14,$15,$16,$17)",
                &[&TENANT, &receipt, &TEMPORAL, &CROSS_SCOPE_EVENT, &"investigation", &world, &"campaign-a", &"continuous", &"train", &artifact_ref.id, &1_i64, &artifact_ref.digest, &1_i64, &run, &"grant-a", &150_i64, &200_i64],
            )
        }));
    }
    let race_results = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        race_results.iter().filter(|result| result.is_ok()).count(),
        1
    );
    let conflict = race_results
        .iter()
        .find_map(|result| result.as_ref().err())
        .expect("one scope must receive a conflict");
    assert_eq!(
        conflict.code(),
        Some(&postgres::error::SqlState::UNIQUE_VIOLATION)
    );
    let mut client = connect();

    let admit = |client: &mut Client,
                 event: &str,
                 run: &str,
                 receipt: &str,
                 temporal: Option<&str>,
                 use_at: Option<i64>,
                 cutoff: Option<i64>,
                 world: &str| {
        client.query_one(
            "SELECT pulso_record_memory_use_temporal($1,$2,$3,$4,$5,$6,$7,$8,$9,$10::text::uuid,$11,$12,$13,$14,$15,$16,$17)",
            &[&TENANT, &receipt, &temporal, &event, &"investigation", &world, &"campaign-a", &"continuous", &"train", &reference.id, &1_i64, &reference.digest, &1_i64, &run, &"grant-a", &use_at, &cutoff],
        )
    };
    admit(
        &mut client,
        EVENT,
        "run-a",
        RECEIPT,
        Some(TEMPORAL),
        Some(150),
        Some(200),
        "world-a",
    )
    .expect("first event admission persists");
    admit(
        &mut client,
        EVENT,
        "run-a",
        RECEIPT,
        Some(TEMPORAL),
        Some(150),
        Some(200),
        "world-a",
    )
    .expect("exact retry is idempotent");
    assert!(
        admit(
            &mut client,
            EVENT,
            "run-b",
            RECEIPT,
            Some(TEMPORAL),
            Some(150),
            Some(200),
            "world-a"
        )
        .is_err(),
        "event identity cannot be replayed for another run"
    );
    assert!(
        admit(
            &mut client,
            "platform-event-0002",
            "run-c",
            RECEIPT,
            Some(TEMPORAL),
            Some(90),
            Some(90),
            "world-a"
        )
        .is_err(),
        "cutoff before snapshot availability is rejected"
    );
    assert!(
        admit(
            &mut client,
            "platform-event-0003",
            "run-c",
            RECEIPT,
            Some(TEMPORAL),
            Some(150),
            Some(200),
            "world-b"
        )
        .is_err(),
        "memory cannot cross scope"
    );
    assert!(
        admit(
            &mut client,
            EVENT,
            "run-a",
            RECEIPT,
            Some(TEMPORAL),
            Some(150),
            Some(200),
            "world-b"
        )
        .is_err(),
        "one source event cannot be reused for another scope"
    );
    assert!(
        admit(
            &mut client,
            "null-temporal",
            "run-null",
            RECEIPT,
            None,
            Some(150),
            Some(200),
            "world-a"
        )
        .is_err()
    );
    assert!(
        admit(
            &mut client,
            "null-use-at",
            "run-null",
            RECEIPT,
            Some(TEMPORAL),
            None,
            Some(200),
            "world-a"
        )
        .is_err()
    );
    assert!(
        admit(
            &mut client,
            "null-cutoff",
            "run-null",
            RECEIPT,
            Some(TEMPORAL),
            Some(150),
            None,
            "world-a"
        )
        .is_err()
    );

    // Hold a successful use transaction open while revocation runs on another
    // connection. Revocation must wait for the shared head lock, then commit a
    // tombstone before later uses can pass.
    client.batch_execute("BEGIN").unwrap();
    admit(
        &mut client,
        "event-before-revoke",
        "run-race",
        RACE_RECEIPT,
        Some(TEMPORAL),
        Some(150),
        Some(200),
        "world-a",
    )
    .expect("admission obtains the scoped-head lock");
    let (pid_tx, pid_rx) = std::sync::mpsc::channel();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let artifact_id = reference.id.clone();
    let artifact_digest = reference.digest.clone();
    let revoker = std::thread::spawn(move || {
        let mut revoker = connect();
        let backend_pid: i32 = revoker
            .query_one("SELECT pg_backend_pid()", &[])
            .unwrap()
            .get(0);
        pid_tx.send(backend_pid).unwrap();
        let result = revoker
            .query_one(
                "SELECT pulso_revoke_memory_snapshot($1,$2::text::uuid,$3,$4,$5)",
                &[&TENANT, &artifact_id, &1_i64, &artifact_digest, &"revoked"],
            )
            .map(|_| ())
            .map_err(|error| error.to_string());
        done_tx.send(result).unwrap();
    });
    let revoke_pid = pid_rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if let Ok(result) = done_rx.try_recv() {
            panic!(
                "revocation completed before the held memory-use transaction released its head lock: {result:?}"
            );
        }
        let wait_type: Option<String> = client
            .query_one(
                "SELECT wait_event_type FROM pg_stat_activity WHERE pid = $1",
                &[&revoke_pid],
            )
            .unwrap()
            .get(0);
        if wait_type.as_deref() == Some("Lock") {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "revocation did not block on the held head lock"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    client.batch_execute("COMMIT").unwrap();
    done_rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap()
        .expect("revocation commits after the preceding use");
    revoker.join().unwrap();
    assert!(
        admit(
            &mut client,
            "platform-event-after-revoke",
            "run-d",
            RECEIPT,
            Some(TEMPORAL),
            Some(150),
            Some(200),
            "world-a"
        )
        .is_err(),
        "revoked memory cannot be admitted"
    );
    let count: i64 = client
        .query_one("SELECT count(*) FROM pulso_memory_use_receipts", &[])
        .unwrap()
        .get(0);
    assert_eq!(count, 3, "failed fences must leave no receipts");
}
