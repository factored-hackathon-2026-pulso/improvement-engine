//! Durable temporal/event identity checks for U33 memory-use receipts.

use improvement_engine_core::{
    ArtifactDraft, ArtifactKind, ArtifactRepository, PostgresArtifactRepository,
};
use postgres::{Client, NoTls};
use serde_json::json;

const TENANT: &str = "tenant-p4";
const TENANT_B: &str = "tenant-p4-b";
const MEMORY: &str = "018f50a1-7f00-7000-8000-000000000071";
const MEMORY_B: &str = "018f50a1-7f00-7000-8000-000000000072";
const MEMORY_C: &str = "018f50a1-7f00-7000-8000-000000000073";
const EVENT: &str = concat!(
    "sha256_",
    "aaaaaaaaaaaaaaaa",
    "aaaaaaaaaaaaaaaa",
    "aaaaaaaaaaaaaaaa",
    "aaaaaaaa"
);
const CROSS_SCOPE_EVENT: &str = concat!(
    "sha256_",
    "bbbbbbbbbbbbbbbb",
    "bbbbbbbbbbbbbbbb",
    "bbbbbbbbbbbbbbbb",
    "bbbbbbbb"
);
const EVENT_ATOMIC: &str = concat!(
    "sha256_",
    "cccccccccccccccc",
    "cccccccccccccccc",
    "cccccccccccccccc",
    "cccccccc"
);
const EVENT_ROLLBACK: &str = concat!(
    "sha256_",
    "dddddddddddddddd",
    "dddddddddddddddd",
    "dddddddddddddddd",
    "dddddddd"
);
const EVENT_PARTIAL: &str = concat!(
    "sha256_",
    "eeeeeeeeeeeeeeee",
    "eeeeeeeeeeeeeeee",
    "eeeeeeeeeeeeeeee",
    "eeeeeeee"
);
const EVENT_AFTER_REVOKE: &str = concat!(
    "sha256_",
    "ffffffffffffffff",
    "ffffffffffffffff",
    "ffffffffffffffff",
    "ffffffff"
);
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
fn temporal_successor_receipt_and_request_commit_atomically_and_replay_exactly() {
    let mut client = connect();
    for migration in [
        include_str!("../../../migrations/0001_pulso_artifact_revisions.sql"),
        include_str!("../../../migrations/0002_pulso_memory_control.sql"),
        include_str!("../../../migrations/0003_pulso_run_events.sql"),
        include_str!("../../../migrations/0004_pulso_memory_temporal_receipts.sql"),
        include_str!("../../../migrations/0005_p4_temporal_successor_outbox.sql"),
    ] {
        client.batch_execute(migration).unwrap();
    }
    client.batch_execute("TRUNCATE pulso_memory_use_receipts, pulso_memory_tombstones, pulso_memory_lineage, pulso_memory_heads, pulso_jobs, pulso_artifact_heads, pulso_artifact_revisions CASCADE").unwrap();

    let mut artifacts = PostgresArtifactRepository::new(client);
    let artifact = artifacts
        .append(
            None,
            ArtifactDraft::new(
                TENANT,
                MEMORY,
                1,
                ArtifactKind::MemoryWiki,
                json!({"available_at_unix_seconds": 100, "purpose": "investigation", "pages": {"index.md": "verified"}}),
                None,
            ),
        )
        .unwrap();
    let reference = artifact.reference();
    let artifact_b = artifacts
        .append(
            None,
            ArtifactDraft::new(
                TENANT_B,
                MEMORY_C,
                1,
                ArtifactKind::MemoryWiki,
                json!({"available_at_unix_seconds": 100, "purpose": "investigation", "pages": {"index.md": "other tenant"}}),
                None,
            ),
        )
        .unwrap();
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
                &TENANT_B,
                &"investigation",
                &"world-a",
                &"campaign-a",
                &"continuous",
                &"train",
                &reference_b.id,
                &1_i64,
                &reference_b.digest,
            ],
        )
        .unwrap();

    let enqueue = |client: &mut Client,
                   tenant: &str,
                   world: &str,
                   artifact_id: &str,
                   artifact_digest: &str,
                   event: &str,
                   receipt: &str,
                   request_digest: &str,
                   successor_id: &str| {
        client.query_one(
        "SELECT receipt_digest, successor_job_id::text, successor_status FROM pulso_record_temporal_memory_successor($1,$2,$3,$4,$5,$6,$7,$8,$9,$10::text::uuid,$11,$12,$13,$14,$15,$16,$17,$18::text::uuid,$19,$20)",
        &[&tenant, &receipt, &TEMPORAL, &event, &"investigation", &world, &"campaign-a", &"continuous", &"train", &artifact_id, &1_i64, &artifact_digest, &1_i64, &"source-run-1", &"sealed-grant-1", &150_i64, &200_i64, &successor_id, &request_digest, &1_715_044_712_192_i64],
        )
    };
    let function_signature = "pulso_record_temporal_memory_successor(text,text,text,text,text,text,text,text,text,uuid,bigint,text,bigint,text,text,bigint,bigint,uuid,text,bigint)";
    let public_execute: bool = client
        .query_one(
            "SELECT EXISTS (SELECT 1 FROM pg_proc p CROSS JOIN LATERAL aclexplode(COALESCE(p.proacl, acldefault('f', p.proowner))) acl WHERE p.oid = to_regprocedure($1) AND acl.grantee = 0 AND acl.privilege_type = 'EXECUTE')",
            &[&function_signature],
        )
        .unwrap()
        .get(0);
    assert!(
        !public_execute,
        "atomic writer must not be executable by PUBLIC"
    );

    // Exercise the function as a least-privilege runtime role: it gets only
    // the explicit writer grant, not direct access to memory or job tables.
    client.batch_execute("DO $$ BEGIN IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname='p4_temporal_successor_runtime') THEN EXECUTE 'DROP OWNED BY p4_temporal_successor_runtime'; EXECUTE 'DROP ROLE p4_temporal_successor_runtime'; END IF; END $$; CREATE ROLE p4_temporal_successor_runtime NOLOGIN; GRANT EXECUTE ON FUNCTION pulso_record_temporal_memory_successor(text,text,text,text,text,text,text,text,text,uuid,bigint,text,bigint,text,text,bigint,bigint,uuid,text,bigint) TO p4_temporal_successor_runtime; SET ROLE p4_temporal_successor_runtime").unwrap();
    let direct_table_access: bool = client
        .query_one(
            "SELECT has_table_privilege(current_user, 'pulso_memory_use_receipts', 'SELECT') OR has_table_privilege(current_user, 'pulso_jobs', 'INSERT')",
            &[],
        )
        .unwrap()
        .get(0);
    assert!(
        !direct_table_access,
        "runtime role must not bypass the writer"
    );

    let raw_event_ref = enqueue(
        &mut client,
        TENANT,
        "world-a",
        &reference.id,
        &reference.digest,
        "customer@example.com",
        RECEIPT,
        "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
        "018f50a1-7f00-7000-8000-000000000081",
    )
    .expect_err("the SQL writer rejects non-opaque source event references");
    assert_eq!(
        raw_event_ref.code(),
        Some(&postgres::error::SqlState::INVALID_PARAMETER_VALUE)
    );

    let first = enqueue(
        &mut client,
        TENANT,
        "world-a",
        &reference.id,
        &reference.digest,
        EVENT_ATOMIC,
        "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
        "018f50a1-7f00-7000-8000-000000000081",
    )
    .expect("U33 receipt and successor request commit together");
    let first_identity: (String, String, String) = (first.get(0), first.get(1), first.get(2));
    assert_eq!(
        first_identity.2, "queued",
        "the successor is requested, not U06-admitted"
    );
    let changed_successor_id = enqueue(
        &mut client,
        TENANT,
        "world-a",
        &reference.id,
        &reference.digest,
        EVENT_ATOMIC,
        "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
        "018f50a1-7f00-7000-8000-000000000084",
    )
    .expect_err("an existing event cannot be rebound to another successor job");
    assert_eq!(
        changed_successor_id.code(),
        Some(&postgres::error::SqlState::UNIQUE_VIOLATION),
        "a complete prior pair with a different requested job is a conflict, not a partial pair"
    );
    client.batch_execute("RESET ROLE").unwrap();
    let first_receipts: i64 = client
        .query_one(
            "SELECT count(*) FROM pulso_memory_use_receipts WHERE tenant_id=$1 AND event_ref=$2",
            &[&TENANT, &EVENT_ATOMIC],
        )
        .unwrap()
        .get(0);
    let first_jobs: i64 = client
        .query_one(
            "SELECT count(*) FROM pulso_jobs WHERE tenant_id=$1 AND source_event_ref=$2",
            &[&TENANT, &EVENT_ATOMIC],
        )
        .unwrap()
        .get(0);
    let first_events: i64 = client
        .query_one(
            "SELECT count(*) FROM pulso_run_events WHERE tenant_id=$1 AND run_ref=$2::text::uuid AND event_code='job_queued'",
            &[&TENANT, &"018f50a1-7f00-7000-8000-000000000081"],
        )
        .unwrap()
        .get(0);
    assert_eq!((first_receipts, first_jobs, first_events), (1, 1, 1));
    client
        .batch_execute("SET ROLE p4_temporal_successor_runtime")
        .unwrap();

    let retry = enqueue(
        &mut client,
        TENANT,
        "world-a",
        &reference.id,
        &reference.digest,
        EVENT_ATOMIC,
        "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
        "018f50a1-7f00-7000-8000-000000000081",
    )
    .expect("exact event replay returns the same durable pair");
    let retry_identity: (String, String, String) = (retry.get(0), retry.get(1), retry.get(2));
    assert_eq!(retry_identity, first_identity);
    client.batch_execute("RESET ROLE").unwrap();
    let replay_events: i64 = client
        .query_one(
            "SELECT count(*) FROM pulso_run_events WHERE tenant_id=$1 AND run_ref=$2::text::uuid AND event_code='job_queued'",
            &[&TENANT, &"018f50a1-7f00-7000-8000-000000000081"],
        )
        .unwrap()
        .get(0);
    assert_eq!(
        replay_events, 1,
        "event replay does not duplicate timeline history"
    );
    client
        .batch_execute("SET ROLE p4_temporal_successor_runtime")
        .unwrap();

    // Event references are unique within a tenant, not globally. Another
    // tenant may independently receive the same upstream source-event key.
    let tenant_b_request = enqueue(
        &mut client,
        TENANT_B,
        "world-a",
        &reference_b.id,
        &reference_b.digest,
        EVENT_ATOMIC,
        "sha256:1212121212121212121212121212121212121212121212121212121212121212",
        "sha256:3434343434343434343434343434343434343434343434343434343434343434",
        "018f50a1-7f00-7000-8000-000000000084",
    )
    .expect("same source event key is isolated by tenant");
    assert_eq!(tenant_b_request.get::<_, String>(2), "queued");
    client.batch_execute("RESET ROLE").unwrap();
    let tenant_b_pair: (i64, i64) = {
        let row = client
            .query_one(
                "SELECT (SELECT count(*) FROM pulso_memory_use_receipts WHERE tenant_id=$1 AND event_ref=$2), (SELECT count(*) FROM pulso_jobs WHERE tenant_id=$1 AND source_event_ref=$2)",
                &[&TENANT_B, &EVENT_ATOMIC],
            )
            .unwrap();
        (row.get(0), row.get(1))
    };
    assert_eq!(tenant_b_pair, (1, 1));

    // Advance tenant B's real scoped memory head through the governed
    // transform/publication path, then prove that replaying the old exact
    // event cannot bypass U33's current-head check.
    let next_pages = json!({"index.md": "tenant B published successor"});
    let next_payload = json!({
        "available_at_unix_seconds": 150,
        "purpose": "investigation",
        "pages": next_pages
    });
    client
        .query_one(
            "SELECT pulso_record_memory_transform_receipt(
                $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11,
                $12::text::uuid, $13, $14, $15::jsonb, $16
            )",
            &[
                &TENANT_B,
                &"sha256:5656565656565656565656565656565656565656565656565656565656565656",
                &"investigation",
                &"world-a",
                &"campaign-a",
                &"continuous",
                &"train",
                &"workspace-p4-head-advance",
                &"run-p4-head-advance",
                &"grant-p4-head-advance",
                &150_i64,
                &reference_b.id,
                &1_i64,
                &reference_b.digest,
                &next_pages,
                &"sha256:6767676767676767676767676767676767676767676767676767676767676767",
            ],
        )
        .expect("record a valid transform for the tenant-B head");
    client
        .query_one(
            "SELECT pulso_publish_memory_revision(
                $1, $2, $3, $4, $5, $6, $7, $8::text::uuid, $9, $10,
                $11, $12, $13, $14::jsonb
            )",
            &[
                &TENANT_B,
                &"investigation",
                &"world-a",
                &"campaign-a",
                &"continuous",
                &"train",
                &1_i64,
                &reference_b.id,
                &1_i64,
                &reference_b.digest,
                &"sha256:5656565656565656565656565656565656565656565656565656565656565656",
                &2_i64,
                &"sha256:7878787878787878787878787878787878787878787878787878787878787878",
                &next_payload,
            ],
        )
        .expect("advance the scoped head through its authorized publication");
    client
        .batch_execute("SET ROLE p4_temporal_successor_runtime")
        .unwrap();
    let stale_head_retry = enqueue(
        &mut client,
        TENANT_B,
        "world-a",
        &reference_b.id,
        &reference_b.digest,
        EVENT_ATOMIC,
        "sha256:1212121212121212121212121212121212121212121212121212121212121212",
        "sha256:3434343434343434343434343434343434343434343434343434343434343434",
        "018f50a1-7f00-7000-8000-000000000084",
    );
    assert!(
        stale_head_retry.is_err(),
        "exact replay cannot bypass U33 after the scoped head advances"
    );
    client.batch_execute("RESET ROLE").unwrap();
    let tenant_b_after_stale_retry: (i64, i64, i64) = {
        let row = client
            .query_one(
                "SELECT (SELECT count(*) FROM pulso_memory_use_receipts WHERE tenant_id=$1 AND event_ref=$2), (SELECT count(*) FROM pulso_jobs WHERE tenant_id=$1 AND source_event_ref=$2), (SELECT count(*) FROM pulso_run_events WHERE tenant_id=$1 AND run_ref=$3::text::uuid AND event_code='job_queued')",
                &[&TENANT_B, &EVENT_ATOMIC, &"018f50a1-7f00-7000-8000-000000000084"],
            )
            .unwrap();
        (row.get(0), row.get(1), row.get(2))
    };
    assert_eq!(tenant_b_after_stale_retry, (1, 1, 1));
    client
        .batch_execute("SET ROLE p4_temporal_successor_runtime")
        .unwrap();

    let changed_request = enqueue(
        &mut client,
        TENANT,
        "world-a",
        &reference.id,
        &reference.digest,
        EVENT_ATOMIC,
        "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "sha256:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
        "018f50a1-7f00-7000-8000-000000000081",
    );
    assert!(
        changed_request.is_err(),
        "changed immutable request must conflict"
    );

    // Force a late outbox insert failure with a duplicate job id. PostgreSQL
    // must roll the preceding U33 receipt insert back with the same transaction.
    let late_failure = enqueue(
        &mut client,
        TENANT,
        "world-a",
        &reference.id,
        &reference.digest,
        EVENT_ROLLBACK,
        "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        "sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
        "018f50a1-7f00-7000-8000-000000000081",
    );
    assert!(late_failure.is_err());
    client.batch_execute("RESET ROLE").unwrap();
    let rolled_back_receipts: i64 = client
        .query_one(
            "SELECT count(*) FROM pulso_memory_use_receipts WHERE tenant_id=$1 AND event_ref=$2",
            &[&TENANT, &EVENT_ROLLBACK],
        )
        .unwrap()
        .get(0);
    assert_eq!(
        rolled_back_receipts, 0,
        "late job failure rolls back U33 receipt"
    );

    // An old receipt with no paired successor is not silently repaired; the
    // mismatch is visible and no second job is invented.
    client
        .query_one(
            "SELECT pulso_record_memory_use_temporal($1,$2,$3,$4,$5,$6,$7,$8,$9,$10::text::uuid,$11,$12,$13,$14,$15,$16,$17)",
            &[&TENANT, &RACE_RECEIPT, &TEMPORAL, &EVENT_PARTIAL, &"investigation", &"world-a", &"campaign-a", &"continuous", &"train", &reference.id, &1_i64, &reference.digest, &1_i64, &"source-run-1", &"sealed-grant-1", &150_i64, &200_i64],
        )
        .unwrap();
    client
        .batch_execute("SET ROLE p4_temporal_successor_runtime")
        .unwrap();
    let partial_pair = enqueue(
        &mut client,
        TENANT,
        "world-a",
        &reference.id,
        &reference.digest,
        EVENT_PARTIAL,
        RACE_RECEIPT,
        "sha256:abababababababababababababababababababababababababababababababab",
        "018f50a1-7f00-7000-8000-000000000083",
    )
    .expect_err("one-sided persisted state requires explicit reconciliation");
    assert_eq!(
        partial_pair.code(),
        Some(&postgres::error::SqlState::OBJECT_NOT_IN_PREREQUISITE_STATE)
    );
    client.batch_execute("RESET ROLE").unwrap();
    let partial_jobs: i64 = client
        .query_one(
            "SELECT count(*) FROM pulso_jobs WHERE tenant_id=$1 AND source_event_ref=$2",
            &[&TENANT, &EVENT_PARTIAL],
        )
        .unwrap()
        .get(0);
    assert_eq!(partial_jobs, 0, "partial receipt is not auto-repaired");

    client
        .query_one(
            "SELECT pulso_revoke_memory_snapshot($1,$2::text::uuid,$3,$4,$5)",
            &[
                &TENANT,
                &reference.id,
                &1_i64,
                &reference.digest,
                &"revoked",
            ],
        )
        .unwrap();
    // Even an exact event replay must pass the U33 live-head/tombstone check
    // before the already-stored receipt/job/event can be returned.
    client
        .batch_execute("SET ROLE p4_temporal_successor_runtime")
        .unwrap();
    let revoked_exact_retry = enqueue(
        &mut client,
        TENANT,
        "world-a",
        &reference.id,
        &reference.digest,
        EVENT_ATOMIC,
        "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
        "018f50a1-7f00-7000-8000-000000000081",
    );
    assert!(
        revoked_exact_retry.is_err(),
        "revoked snapshot cannot turn an exact replay into success"
    );
    client.batch_execute("RESET ROLE").unwrap();
    let after_revoked_retry: (i64, i64, i64) = {
        let row = client
            .query_one(
                "SELECT (SELECT count(*) FROM pulso_memory_use_receipts WHERE tenant_id=$1 AND event_ref=$2), (SELECT count(*) FROM pulso_jobs WHERE tenant_id=$1 AND source_event_ref=$2), (SELECT count(*) FROM pulso_run_events WHERE tenant_id=$1 AND run_ref=$3::text::uuid AND event_code='job_queued')",
                &[&TENANT, &EVENT_ATOMIC, &"018f50a1-7f00-7000-8000-000000000081"],
            )
            .unwrap();
        (row.get(0), row.get(1), row.get(2))
    };
    assert_eq!(after_revoked_retry, (1, 1, 1));

    let revoked = enqueue(
        &mut client,
        TENANT,
        "world-a",
        &reference.id,
        &reference.digest,
        EVENT_AFTER_REVOKE,
        "sha256:9999999999999999999999999999999999999999999999999999999999999999",
        "sha256:8888888888888888888888888888888888888888888888888888888888888888",
        "018f50a1-7f00-7000-8000-000000000082",
    );
    assert!(
        revoked.is_err(),
        "revoked snapshot cannot enqueue a receipt or successor"
    );
    let revoked_receipts: i64 = client
        .query_one(
            "SELECT count(*) FROM pulso_memory_use_receipts WHERE tenant_id=$1 AND event_ref=$2",
            &[&TENANT, &EVENT_AFTER_REVOKE],
        )
        .unwrap()
        .get(0);
    assert_eq!(revoked_receipts, 0);
    client.batch_execute("REVOKE ALL ON FUNCTION pulso_record_temporal_memory_successor(text,text,text,text,text,text,text,text,text,uuid,bigint,text,bigint,text,text,bigint,bigint,uuid,text,bigint) FROM p4_temporal_successor_runtime; DROP ROLE p4_temporal_successor_runtime").unwrap();
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
