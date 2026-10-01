//! Explicitly gated durable-boundary checks; never runs against an unspecified DB.

use improvement_engine_core::{
    ArtifactDraft, ArtifactKind, ArtifactRepository, PostgresArtifactRepository,
};
use postgres::{Client, NoTls};
use serde_json::json;

const SOURCE_ID: &str = "018f0f4e-7bbd-7000-8000-000000000001";
const SIGNAL_ID: &str = "018f0f4e-7bbd-7000-8000-000000000002";
const DIGEST_A: &str = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const DIGEST_B: &str = "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

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
fn migration_enforces_cas_immutability_and_source_snapshot_kind() {
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
        .batch_execute("TRUNCATE pulso_artifact_heads, pulso_artifact_revisions")
        .expect("clear isolated U02 tables");
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
}
