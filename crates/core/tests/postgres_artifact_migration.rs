//! Explicitly gated durable-boundary checks; never runs against an unspecified DB.

use improvement_engine_core::{
    ArtifactDraft, ArtifactKind, ArtifactRepository, PostgresArtifactRepository,
};
use postgres::{Client, NoTls};
use serde_json::json;
use std::sync::{Arc, Barrier};
use std::thread;

const SOURCE_ID: &str = "018f0f4e-7bbd-7000-8000-000000000001";
const SIGNAL_ID: &str = "018f0f4e-7bbd-7000-8000-000000000002";
const MEMORY_ID: &str = "018f0f4e-7bbd-7000-8000-000000000007";
const DIGEST_A: &str = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const DIGEST_B: &str = "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const DIGEST_C: &str = "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
const DIGEST_D: &str = "sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";

#[allow(clippy::too_many_arguments)]
fn append(
    client: &mut Client,
    tenant: &str,
    id: &str,
    expected_head: Option<i64>,
    revision: i64,
    kind: &str,
    digest: &str,
    source: Option<(&str, &str, i64, &str)>,
) -> Result<(), postgres::Error> {
    let (source_tenant, source_id, source_revision, source_digest) = source
        .map(|(tenant, id, revision, digest)| {
            (Some(tenant), Some(id), Some(revision), Some(digest))
        })
        .unwrap_or((None, None, None, None));
    // `postgres` implements ToSql for `serde_json::Value` (not `&str`) when
    // the SQL parameter is JSONB. Keep this direct migration seam typed so
    // the real PostgreSQL CI regression covers both UUID and JSONB bindings.
    let payload = json!({});
    client.query_one(
        "SELECT pulso_append_artifact_revision(
            $1, $2::text::uuid, $3, $4, $5, $6, $7::jsonb, $8, $9::text::uuid, $10, $11
        )",
        &[
            &tenant,
            &id,
            &expected_head,
            &revision,
            &kind,
            &digest,
            &payload,
            &source_tenant,
            &source_id,
            &source_revision,
            &source_digest,
        ],
    )?;
    Ok(())
}

#[test]
#[ignore = "requires an isolated PULSO_TEST_POSTGRES_URL and explicit destructive-test consent"]
fn migration_enforces_cas_immutability_and_memory_tombstones() {
    assert_eq!(
        std::env::var("PULSO_ALLOW_DESTRUCTIVE_TEST_DB").as_deref(),
        Ok("1"),
        "set PULSO_ALLOW_DESTRUCTIVE_TEST_DB=1 only for an isolated test database"
    );
    let database_url = std::env::var("PULSO_TEST_POSTGRES_URL")
        .expect("PULSO_TEST_POSTGRES_URL must point at an isolated test database");
    let mut client = Client::connect(&database_url, NoTls).expect("isolated PostgreSQL connection");
    client
        .batch_execute(include_str!(
            "../../../migrations/0001_pulso_artifact_revisions.sql"
        ))
        .expect("apply U02 migration");
    client
        .batch_execute(include_str!(
            "../../../migrations/0002_pulso_memory_control.sql"
        ))
        .expect("apply U33 migration");
    client
        .batch_execute(
            "TRUNCATE pulso_memory_use_receipts, pulso_memory_transform_receipts, pulso_memory_tombstones, pulso_memory_lineage,
                      pulso_memory_heads, pulso_artifact_heads, pulso_artifact_revisions",
        )
        .expect("clear isolated Pulso tables");
    client
        .batch_execute(
            "DROP ROLE IF EXISTS pulso_u02_runtime_test;
             CREATE ROLE pulso_u02_runtime_test NOLOGIN NOINHERIT;
             GRANT pulso_u02_runtime_test TO CURRENT_USER;
             GRANT EXECUTE ON FUNCTION pulso_append_artifact_revision(
                 TEXT, UUID, BIGINT, BIGINT, TEXT, TEXT, JSONB, TEXT, UUID, BIGINT, TEXT
             ) TO pulso_u02_runtime_test;
             GRANT EXECUTE ON FUNCTION pulso_get_artifact_revision(TEXT, UUID, BIGINT)
                 TO pulso_u02_runtime_test;
             SET ROLE pulso_u02_runtime_test;",
        )
        .expect("configure isolated least-privilege runtime role");

    append(
        &mut client,
        "tenant-a",
        SOURCE_ID,
        None,
        1,
        "source_snapshot",
        DIGEST_A,
        None,
    )
    .unwrap();
    append(
        &mut client,
        "tenant-a",
        SIGNAL_ID,
        None,
        1,
        "signal",
        DIGEST_B,
        Some(("tenant-a", SOURCE_ID, 1, DIGEST_A)),
    )
    .unwrap();

    assert!(
        append(
            &mut client,
            "tenant-a",
            SOURCE_ID,
            None,
            2,
            "source_snapshot",
            DIGEST_B,
            None,
        )
        .is_err()
    );
    assert!(
        append(
            &mut client,
            "tenant-a",
            "018f0f4e-7bbd-7000-8000-000000000003",
            None,
            1,
            "signal",
            DIGEST_B,
            Some(("tenant-a", SIGNAL_ID, 1, DIGEST_B)),
        )
        .is_err()
    );
    assert!(
        client
            .execute(
                "UPDATE pulso_artifact_revisions SET payload = '{\"changed\": true}'::jsonb",
                &[]
            )
            .is_err()
    );
    assert!(client
        .execute(
            "INSERT INTO pulso_artifact_revisions (
                tenant_id, artifact_id, revision, kind, digest, payload
             ) VALUES (
                'tenant-a', '018f0f4e-7bbd-7000-8000-000000000006', 1,
                'signal', 'sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc', '{}'::jsonb
             )",
            &[],
        )
        .is_err());

    let mut repository = PostgresArtifactRepository::new(client);
    let source = repository
        .append(
            None,
            ArtifactDraft::new(
                "tenant-a",
                "018f0f4e-7bbd-7000-8000-000000000004",
                1,
                ArtifactKind::SourceSnapshot,
                json!({"table": "contacts"}),
                None,
            ),
        )
        .expect("adapter appends source snapshot via function");
    let signal = repository
        .append(
            None,
            ArtifactDraft::new(
                "tenant-a",
                "018f0f4e-7bbd-7000-8000-000000000005",
                1,
                ArtifactKind::Signal,
                json!({"metric": "repeat_contact_rate"}),
                Some(source.reference()),
            ),
        )
        .expect("adapter reads back a verified source snapshot reference");
    assert_eq!(
        repository
            .get("tenant-a", &signal.id, signal.revision)
            .expect("adapter readback")
            .expect("stored artifact"),
        signal
    );

    let mut client = repository.into_inner();

    let initial_payload = json!({
        "available_at_unix_seconds": 100,
        "purpose": "investigation",
        "pages": {"index.md": "initial"}
    });
    client
        .query_one(
            "SELECT pulso_append_artifact_revision(
                $1, $2::text::uuid, $3, $4, $5, $6, $7::jsonb, $8, $9::text::uuid, $10, $11
            )",
            &[
                &"tenant-a",
                &MEMORY_ID,
                &Option::<i64>::None,
                &1_i64,
                &"memory_wiki",
                &DIGEST_A,
                &initial_payload,
                &Option::<String>::None,
                &Option::<String>::None,
                &Option::<i64>::None,
                &Option::<String>::None,
            ],
        )
        .expect("append schema-valid immutable memory snapshot");
    assert!(
        client
            .query_one(
                "SELECT pulso_seed_memory_head($1, $2, $3, $4, $5, $6, $7::text::uuid, $8, $9)",
                &[
                    &"tenant-a",
                    &"investigation",
                    &"world-a",
                    &"campaign-a",
                    &"continuous",
                    &"train",
                    &MEMORY_ID,
                    &1_i64,
                    &DIGEST_A
                ],
            )
            .is_err(),
        "runtime role cannot fabricate privileged memory state"
    );
    client
        .batch_execute("RESET ROLE")
        .expect("return to privileged memory boundary");
    assert!(
        client
            .query_one(
                "SELECT pulso_seed_memory_head($1, $2, $3, $4, $5, $6, $7::text::uuid, $8, $9)",
                &[
                    &"tenant-a",
                    &"investigation",
                    &"bad-world",
                    &"campaign-a",
                    &"continuous",
                    &"train",
                    &SIGNAL_ID,
                    &1_i64,
                    &DIGEST_B
                ],
            )
            .is_err(),
        "seed rejects a non-memory/schema-invalid artifact"
    );
    client
        .query_one(
            "SELECT pulso_seed_memory_head(
                $1, $2, $3, $4, $5, $6, $7::text::uuid, $8, $9
            )",
            &[
                &"tenant-a",
                &"investigation",
                &"world-a",
                &"campaign-a",
                &"continuous",
                &"train",
                &MEMORY_ID,
                &1_i64,
                &DIGEST_A,
            ],
        )
        .expect("seed scope-specific memory head");
    assert!(
        client
            .query_one(
                "SELECT pulso_record_memory_transform_receipt(
                $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11,
                $12::text::uuid, $13, $14, $15::jsonb, $16
            )",
                &[
                    &"tenant-a",
                    &DIGEST_A,
                    &"investigation",
                    &"world-a",
                    &"campaign-a",
                    &"continuous",
                    &"train",
                    &"workspace-nested",
                    &"run-nested",
                    &"grant-nested",
                    &100_i64,
                    &MEMORY_ID,
                    &1_i64,
                    &DIGEST_A,
                    &json!({"nested.md": {"not": "a string"}}),
                    &DIGEST_B,
                ],
            )
            .is_err(),
        "nested JSON page values are not memory text pages"
    );
    let changed_pages = json!({"index.md": "published"});
    let published_payload = json!({
        "available_at_unix_seconds": 100,
        "purpose": "investigation",
        "pages": changed_pages
    });
    // DIGEST_C is not persisted before this barrier. The isolated clients must
    // therefore take the INSERT .. ON CONFLICT path, rather than only testing
    // a pre-existing idempotent receipt.
    let transform_barrier = Arc::new(Barrier::new(3));
    let transform_workers = (0..2)
        .map(|_| {
            let barrier = Arc::clone(&transform_barrier);
            let database_url = database_url.clone();
            thread::spawn(move || {
                let mut connection =
                    Client::connect(&database_url, NoTls).expect("isolated transform client");
                barrier.wait();
                connection
                    .query_one(
                        "SELECT pulso_record_memory_transform_receipt(
                            $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11,
                            $12::text::uuid, $13, $14, $15::jsonb, $16
                        )",
                        &[
                            &"tenant-a",
                            &DIGEST_C,
                            &"investigation",
                            &"world-a",
                            &"campaign-a",
                            &"continuous",
                            &"train",
                            &"workspace-concurrent",
                            &"run-concurrent",
                            &"grant-concurrent",
                            &100_i64,
                            &MEMORY_ID,
                            &1_i64,
                            &DIGEST_A,
                            &json!({"index.md": "published"}),
                            &DIGEST_B,
                        ],
                    )
                    .is_ok()
            })
        })
        .collect::<Vec<_>>();
    transform_barrier.wait();
    assert!(
        transform_workers
            .into_iter()
            .all(|worker| worker.join().unwrap()),
        "both concurrent first transform receipts succeed"
    );
    let transform_count: i64 = client
        .query_one(
            "SELECT count(*) FROM pulso_memory_transform_receipts
             WHERE tenant_id = $1 AND receipt_digest = $2",
            &[&"tenant-a", &DIGEST_C],
        )
        .unwrap()
        .get(0);
    assert_eq!(
        transform_count, 1,
        "one transform receipt survives the race"
    );
    assert!(
        client
            .query_one(
                "SELECT pulso_record_memory_transform_receipt(
                    $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11,
                    $12::text::uuid, $13, $14, $15::jsonb, $16
                )",
                &[
                    &"tenant-a",
                    &DIGEST_C,
                    &"investigation",
                    &"world-a",
                    &"campaign-a",
                    &"continuous",
                    &"train",
                    &"workspace-transform-conflict",
                    &"run-concurrent",
                    &"grant-concurrent",
                    &100_i64,
                    &MEMORY_ID,
                    &1_i64,
                    &DIGEST_A,
                    &changed_pages,
                    &DIGEST_B,
                ],
            )
            .is_err(),
        "a divergent duplicate transform receipt conflicts"
    );
    client
        .query_one(
            "SELECT pulso_publish_memory_revision(
                $1, $2, $3, $4, $5, $6, $7, $8::text::uuid, $9, $10, $11, $12, $13, $14::jsonb
            )",
            &[
                &"tenant-a",
                &"investigation",
                &"world-a",
                &"campaign-a",
                &"continuous",
                &"train",
                &1_i64,
                &MEMORY_ID,
                &1_i64,
                &DIGEST_A,
                &DIGEST_C,
                &2_i64,
                &DIGEST_B,
                &published_payload,
            ],
        )
        .expect("publish memory by scoped CAS");
    // DIGEST_D is likewise fresh until these two connections issue the
    // INSERT concurrently. A sequential call here would not prove race safety.
    let use_barrier = Arc::new(Barrier::new(3));
    let use_workers = (0..2)
        .map(|_| {
            let barrier = Arc::clone(&use_barrier);
            let database_url = database_url.clone();
            thread::spawn(move || {
                let mut connection =
                    Client::connect(&database_url, NoTls).expect("isolated use client");
                barrier.wait();
                connection
                    .query_one(
                        "SELECT pulso_record_memory_use(
                            $1, $2, $3, $4, $5, $6, $7, $8::text::uuid, $9, $10, $11, $12, $13, $14
                        )",
                        &[
                            &"tenant-a",
                            &DIGEST_D,
                            &"investigation",
                            &"world-a",
                            &"campaign-a",
                            &"continuous",
                            &"train",
                            &MEMORY_ID,
                            &2_i64,
                            &DIGEST_B,
                            &2_i64,
                            &"run-use-concurrent",
                            &"grant-use-concurrent",
                            &100_i64,
                        ],
                    )
                    .is_ok()
            })
        })
        .collect::<Vec<_>>();
    use_barrier.wait();
    assert!(
        use_workers.into_iter().all(|worker| worker.join().unwrap()),
        "both concurrent first allowed-use receipts succeed"
    );
    let use_count: i64 = client
        .query_one(
            "SELECT count(*) FROM pulso_memory_use_receipts WHERE tenant_id = $1 AND receipt_digest = $2",
            &[&"tenant-a", &DIGEST_D],
        )
        .unwrap()
        .get(0);
    assert_eq!(use_count, 1, "one allowed-use receipt survives the race");
    assert!(
        client
            .query_one(
                "SELECT pulso_record_memory_use(
                $1, $2, $3, $4, $5, $6, $7, $8::text::uuid, $9, $10, $11, $12, $13, $14
            )",
                &[
                    &"tenant-a",
                    &DIGEST_D,
                    &"investigation",
                    &"world-a",
                    &"campaign-a",
                    &"continuous",
                    &"train",
                    &MEMORY_ID,
                    &2_i64,
                    &DIGEST_B,
                    &2_i64,
                    &"run-use-concurrent",
                    &"grant-conflict",
                    &100_i64,
                ],
            )
            .is_err(),
        "same use receipt key with different semantics conflicts"
    );
    assert!(
        client
            .query_one(
                "SELECT pulso_publish_memory_revision(
                $1, $2, $3, $4, $5, $6, $7, $8::text::uuid, $9, $10, $11, $12, $13, $14::jsonb
            )",
                &[
                    &"tenant-a",
                    &"investigation",
                    &"world-a",
                    &"campaign-a",
                    &"continuous",
                    &"train",
                    &1_i64,
                    &MEMORY_ID,
                    &1_i64,
                    &DIGEST_A,
                    &DIGEST_A,
                    &2_i64,
                    &DIGEST_B,
                    &published_payload,
                ],
            )
            .is_err()
    );
    client
        .query_one(
            "SELECT pulso_revoke_memory_snapshot($1, $2::text::uuid, $3, $4, $5)",
            &[
                &"tenant-a",
                &MEMORY_ID,
                &1_i64,
                &DIGEST_A,
                &"source_permission_revoked",
            ],
        )
        .expect("append immutable tombstone");
    assert!(
        client
            .query_one(
                "SELECT pulso_publish_memory_revision(
                $1, $2, $3, $4, $5, $6, $7, $8::text::uuid, $9, $10, $11, $12, $13, $14::jsonb
            )",
                &[
                    &"tenant-a",
                    &"investigation",
                    &"world-a",
                    &"campaign-a",
                    &"continuous",
                    &"train",
                    &2_i64,
                    &MEMORY_ID,
                    &2_i64,
                    &DIGEST_B,
                    &DIGEST_A,
                    &3_i64,
                    &DIGEST_A,
                    &initial_payload,
                ],
            )
            .is_err()
    );
}
