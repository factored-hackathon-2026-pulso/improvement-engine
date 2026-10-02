//! Explicitly gated U29 durable-boundary check on an isolated PostgreSQL DB.

use improvement_engine_core::platform_observations::{
    CoreChainVerification, CoreChainVerifierPort, Coverage, CoverageState, EvidenceKind,
    InteractionEventKind, ObservationAccess, ObservationAuthorizationPort, ObservationBatchContext,
    ObservationError, ObservationEvent, ObservationRepository, ObservationSourceContract,
    ObservationSourceRegistry, PlatformObservationBatch, PostgresObservationRepository,
    SourceSamplingMode, TargetSystem, TransportCursor, TransportSequenceMode,
};
use postgres::{Client, NoTls};

struct TrustedCoreVerifier;
struct TestAuthority;
impl ObservationAuthorizationPort for TestAuthority {
    fn authorize(&self, access: &ObservationAccess) -> bool {
        access.tenant_id() == "tenant-a"
            && access.grant_id() == "grant-a"
            && access.purpose() == "platform_observation"
    }
}
fn access() -> ObservationAccess {
    ObservationAccess::new("tenant-a", "grant-a", "platform_observation").unwrap()
}
fn authority() -> Box<dyn ObservationAuthorizationPort> {
    Box::new(TestAuthority)
}
fn restricted_client(url: &str) -> Client {
    let mut config: postgres::Config = url.parse().unwrap();
    config
        .user("pulso_u29_test_runtime")
        .password("pulso_u29_ephemeral_only");
    config.connect(NoTls).unwrap()
}
fn registry() -> ObservationSourceRegistry {
    ObservationSourceRegistry::from_trusted_configuration(vec![
        ObservationSourceContract::new(
            "attention-platform",
            "contract:platform-v1",
            TargetSystem::Attention,
            TransportSequenceMode::Contiguous,
            SourceSamplingMode::DurableAudit,
            true,
        )
        .unwrap(),
        ObservationSourceContract::new(
            "agent-core",
            "contract:core-v1",
            TargetSystem::Attention,
            TransportSequenceMode::Contiguous,
            SourceSamplingMode::DurableAudit,
            true,
        )
        .unwrap(),
    ])
    .unwrap()
}
impl CoreChainVerifierPort for TrustedCoreVerifier {
    fn verify(&self, _event: &ObservationEvent) -> CoreChainVerification {
        CoreChainVerification::Verified {
            receipt_digest:
                "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
        }
    }
}

fn batch() -> PlatformObservationBatch {
    PlatformObservationBatch::new(
        ObservationBatchContext::new(
            "tenant-a",
            "attention-platform",
            "partition-1",
            "contract:platform-v1",
            TransportCursor::sequence(0, 0),
        )
        .unwrap(),
        Coverage::new(
            CoverageState::Complete,
            "eligible-goals",
            1_759_320_000_000,
            1_759_406_400_000,
            Some(1),
            None,
        )
        .unwrap(),
        vec![
            ObservationEvent::new(
                "event-1",
                "run-1",
                TargetSystem::Attention,
                EvidenceKind::PlatformAudit,
                InteractionEventKind::ResponseRequested,
                1_759_320_000_000,
                1_759_320_002_000,
            )
            .unwrap(),
        ],
        1_759_320_002_000,
        1_800_000_000_000,
    )
    .unwrap()
}

#[test]
#[ignore = "requires isolated PULSO_TEST_POSTGRES_URL and explicit destructive-test consent"]
fn treated_batch_survives_reconnect_without_cross_tenant_read_or_double_insert() {
    assert_eq!(
        std::env::var("PULSO_ALLOW_DESTRUCTIVE_TEST_DB").as_deref(),
        Ok("1")
    );
    let url = std::env::var("PULSO_TEST_POSTGRES_URL").expect("isolated U29 database");
    let mut client = Client::connect(&url, NoTls).expect("connect");
    client
        .batch_execute(include_str!(
            "../../../migrations/0002_pulso_platform_observations.sql"
        ))
        .expect("apply U29 migration");
    client.batch_execute("TRUNCATE pulso_platform_observation_events, pulso_platform_observation_batches, pulso_platform_observation_blobs, pulso_platform_observation_cursors")
        .expect("clear isolated U29 tables");

    let expected = batch();
    let mut repository =
        PostgresObservationRepository::authorized(client, access(), authority(), registry())
            .unwrap();
    let first = repository.ingest(expected.clone()).expect("durable insert");
    drop(repository);

    let client = Client::connect(&url, NoTls).expect("reconnect");
    let mut repository =
        PostgresObservationRepository::authorized(client, access(), authority(), registry())
            .unwrap();
    assert_eq!(repository.list("tenant-a").expect("tenant read").len(), 1);
    assert_eq!(
        repository.list("tenant-b"),
        Err(ObservationError::TenantAccessDenied)
    );
    assert_eq!(repository.ingest(expected).expect("exact replay"), first);
    assert_eq!(repository.list("tenant-a").expect("no duplicate").len(), 1);

    let changed = PlatformObservationBatch::new(
        ObservationBatchContext::new(
            "tenant-a",
            "attention-platform",
            "partition-1",
            "contract:platform-v1",
            TransportCursor::sequence(1, 1),
        )
        .unwrap(),
        Coverage::new(
            CoverageState::Complete,
            "eligible-goals",
            1_759_320_000_000,
            1_759_406_400_000,
            Some(1),
            None,
        )
        .unwrap(),
        vec![
            ObservationEvent::new(
                "event-1",
                "run-1",
                TargetSystem::Attention,
                EvidenceKind::PlatformAudit,
                InteractionEventKind::ResponseReceived,
                1_759_320_000_000,
                1_759_320_002_000,
            )
            .unwrap(),
        ],
        1_759_320_002_000,
        1_800_000_000_000,
    )
    .unwrap();
    assert_eq!(
        repository.ingest(changed),
        Err(ObservationError::ConflictingEvent {
            event_id: "event-1".into()
        })
    );
    assert_eq!(repository.list("tenant-a").unwrap().len(), 1);

    let old_projection = repository
        .window_projection(
            "tenant-a",
            1_759_320_000_000,
            1_759_406_400_000,
            1_759_320_002_000,
        )
        .unwrap();
    let late = PlatformObservationBatch::new(
        ObservationBatchContext::new(
            "tenant-a",
            "attention-platform",
            "partition-1",
            "contract:platform-v1",
            TransportCursor::sequence(1, 1),
        )
        .unwrap(),
        Coverage::new(
            CoverageState::Partial,
            "eligible-goals",
            1_759_320_000_000,
            1_759_406_400_000,
            None,
            Some("late_delivery".into()),
        )
        .unwrap(),
        vec![
            ObservationEvent::new(
                "event-2",
                "run-2",
                TargetSystem::Attention,
                EvidenceKind::PlatformAudit,
                InteractionEventKind::HumanAction,
                1_759_320_001_000,
                1_759_320_010_000,
            )
            .unwrap(),
        ],
        1_759_320_010_000,
        1_800_000_000_000,
    )
    .unwrap();
    repository
        .ingest(late)
        .expect("failed conflict did not advance cursor");
    let still_old = repository
        .window_projection(
            "tenant-a",
            1_759_320_000_000,
            1_759_406_400_000,
            1_759_320_002_000,
        )
        .unwrap();
    let corrected = repository
        .window_projection(
            "tenant-a",
            1_759_320_000_000,
            1_759_406_400_000,
            1_759_320_010_000,
        )
        .unwrap();
    assert_eq!(old_projection, still_old);
    assert_eq!(corrected.events().len(), 2);
    assert_eq!(corrected.coverages().len(), 2);
    assert_eq!(
        corrected.coverages()[1].coverage().state(),
        CoverageState::Partial
    );
    assert_ne!(old_projection.digest(), corrected.digest());

    let outage = PlatformObservationBatch::new(
        ObservationBatchContext::new(
            "tenant-a",
            "attention-platform",
            "partition-1",
            "contract:platform-v1",
            TransportCursor::sequence(2, 2),
        )
        .unwrap(),
        Coverage::new(
            CoverageState::Degraded,
            "eligible-goals",
            1_759_320_000_000,
            1_759_406_400_000,
            None,
            Some("collector_unavailable".into()),
        )
        .unwrap(),
        Vec::new(),
        1_759_320_020_000,
        1_800_000_000_000,
    )
    .unwrap();
    repository
        .ingest(outage)
        .expect("outage heartbeat is durable without inventing events");
    let outage_view = repository
        .window_projection(
            "tenant-a",
            1_759_320_000_000,
            1_759_406_400_000,
            1_759_320_020_000,
        )
        .unwrap();
    assert_eq!(outage_view.events().len(), 2);
    assert_eq!(outage_view.coverages().len(), 3);
    assert_eq!(
        outage_view.coverages()[2].coverage().state(),
        CoverageState::Degraded
    );

    let inconsistent_retention = PlatformObservationBatch::new(
        ObservationBatchContext::new(
            "tenant-a",
            "attention-platform",
            "partition-1",
            "contract:platform-v1",
            TransportCursor::sequence(3, 3),
        )
        .unwrap(),
        Coverage::new(
            CoverageState::Degraded,
            "eligible-goals",
            1_759_320_000_000,
            1_759_406_400_000,
            None,
            Some("collector_unavailable".into()),
        )
        .unwrap(),
        Vec::new(),
        1_759_320_021_000,
        1_900_000_000_000,
    )
    .unwrap();
    assert_eq!(
        repository.ingest(inconsistent_retention),
        Err(ObservationError::RetentionConflict)
    );

    drop(repository);
    let client = Client::connect(&url, NoTls).unwrap();
    let mut repository = PostgresObservationRepository::authorized_with_core_verifier(
        client,
        access(),
        authority(),
        registry(),
        Box::new(TrustedCoreVerifier),
    )
    .unwrap();
    let core_event = ObservationEvent::new(
        "core-1",
        "core-run-1",
        TargetSystem::Attention,
        EvidenceKind::CoreAudit,
        InteractionEventKind::ModelDecision,
        1_759_320_003_000,
        1_759_320_004_000,
    )
    .unwrap()
    .with_source_event_digest(
        "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
    )
    .unwrap()
    .with_core_run_sequence(0);
    let core_batch = PlatformObservationBatch::new(
        ObservationBatchContext::new(
            "tenant-a",
            "agent-core",
            "core-partition",
            "contract:core-v1",
            TransportCursor::sequence(0, 0),
        )
        .unwrap(),
        Coverage::new(
            CoverageState::Complete,
            "core-runs",
            1_759_320_000_000,
            1_759_406_400_000,
            Some(1),
            None,
        )
        .unwrap(),
        vec![core_event],
        1_759_320_004_000,
        1_800_000_000_000,
    )
    .unwrap();
    let verified = repository
        .ingest(core_batch.clone())
        .expect("trusted chain receipt");
    assert_eq!(verified.core_verification_refs.len(), 1);
    drop(repository);

    let client = Client::connect(&url, NoTls).unwrap();
    let mut repository =
        PostgresObservationRepository::authorized(client, access(), authority(), registry())
            .unwrap();
    assert_eq!(
        repository
            .ingest(core_batch)
            .expect("replay uses stored verification receipt"),
        verified
    );

    let delayed = PlatformObservationBatch::new(
        ObservationBatchContext::new(
            "tenant-a",
            "attention-platform",
            "partition-1",
            "contract:platform-v1",
            TransportCursor::sequence(3, 3),
        )
        .unwrap(),
        Coverage::new(
            CoverageState::Complete,
            "eligible-goals",
            1_759_320_000_000,
            1_759_406_400_000,
            Some(1),
            None,
        )
        .unwrap(),
        vec![
            ObservationEvent::new(
                "delayed-1",
                "run-delayed",
                TargetSystem::Attention,
                EvidenceKind::PlatformAudit,
                InteractionEventKind::HumanAction,
                1_759_320_003_000,
                1_759_320_030_000,
            )
            .unwrap(),
        ],
        1_759_320_040_000,
        1_800_000_000_000,
    )
    .unwrap();
    repository.ingest(delayed).unwrap();
    let before_batch = repository
        .window_projection(
            "tenant-a",
            1_759_320_000_000,
            1_759_406_400_000,
            1_759_320_035_000,
        )
        .unwrap();
    let after_batch = repository
        .window_projection(
            "tenant-a",
            1_759_320_000_000,
            1_759_406_400_000,
            1_759_320_040_000,
        )
        .unwrap();
    assert_eq!(before_batch.events().len(), 3);
    assert_eq!(after_batch.events().len(), 4);

    drop(repository);
    let mut admin = Client::connect(&url, NoTls).unwrap();
    assert!(
        admin
            .execute(
                "DELETE FROM pulso_platform_observation_blobs WHERE tenant_id='tenant-a'",
                &[],
            )
            .is_err(),
        "purge is not implemented; immutable trigger blocks even owner delete"
    );
    admin
        .batch_execute(
            "DO $$ BEGIN
           IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname='pulso_u29_test_runtime') THEN
             CREATE ROLE pulso_u29_test_runtime;
           END IF;
           IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname='pulso_u29_test_foreign') THEN
             CREATE ROLE pulso_u29_test_foreign NOINHERIT NOBYPASSRLS;
           END IF;
         END $$;
         ALTER ROLE pulso_u29_test_runtime LOGIN NOINHERIT NOBYPASSRLS PASSWORD 'pulso_u29_ephemeral_only';
         GRANT USAGE ON SCHEMA public TO pulso_u29_test_runtime;
         GRANT SELECT, INSERT, UPDATE ON
           pulso_platform_observation_blobs,
           pulso_platform_observation_cursors,
           pulso_platform_observation_batches,
           pulso_platform_observation_events TO pulso_u29_test_runtime;
         INSERT INTO pulso_observation_role_entitlements
           (role_name,tenant_id,grant_id,purpose)
           VALUES ('pulso_u29_test_runtime','tenant-a','grant-a','platform_observation')
           ON CONFLICT DO NOTHING;",
        )
        .unwrap();
    let runtime_client = restricted_client(&url);
    let mut restricted = PostgresObservationRepository::authorized(
        runtime_client,
        access(),
        authority(),
        registry(),
    )
    .unwrap();
    assert_eq!(restricted.list("tenant-a").unwrap().len(), 4);
    assert_eq!(
        restricted.list("tenant-b"),
        Err(ObservationError::TenantAccessDenied)
    );
    let mut direct = restricted_client(&url);
    assert!(direct.batch_execute("SET ROLE postgres").is_err());
    assert!(
        direct
            .batch_execute("SET ROLE pulso_u29_test_foreign")
            .is_err()
    );
    assert!(
        direct
            .execute(
                "INSERT INTO pulso_observation_role_entitlements
         (role_name,tenant_id,grant_id,purpose) VALUES
         ('pulso_u29_test_runtime','tenant-b','grant-a','platform_observation')",
                &[],
            )
            .is_err()
    );
    let forbidden: i64 = direct
        .query_one(
            "SELECT count(*) FROM pulso_platform_observation_events WHERE tenant_id='tenant-a'",
            &[],
        )
        .unwrap()
        .get(0);
    assert_eq!(forbidden, 0, "no transaction-local grant means no rows");
    let mut scoped_tx = direct.transaction().unwrap();
    scoped_tx
        .query_one(
            "SELECT set_config('pulso.observation_grant','grant-a',true),
                set_config('pulso.observation_purpose','platform_observation',true)",
            &[],
        )
        .unwrap();
    let scoped_count: i64 = scoped_tx
        .query_one(
            "SELECT count(*) FROM pulso_platform_observation_events WHERE tenant_id='tenant-a'",
            &[],
        )
        .unwrap()
        .get(0);
    assert_eq!(scoped_count, 4);
    scoped_tx.commit().unwrap();
    let after_commit: i64 = direct
        .query_one(
            "SELECT count(*) FROM pulso_platform_observation_events WHERE tenant_id='tenant-a'",
            &[],
        )
        .unwrap()
        .get(0);
    assert_eq!(
        after_commit, 0,
        "grant context cannot leak into a reused connection"
    );
    direct
        .query_one(
            "SELECT set_config('pulso.observation_grant','grant-b',false),
      set_config('pulso.observation_purpose','platform_observation',false)",
            &[],
        )
        .unwrap();
    let wrong_grant: i64 = direct
        .query_one(
            "SELECT count(*) FROM pulso_platform_observation_events WHERE tenant_id='tenant-a'",
            &[],
        )
        .unwrap()
        .get(0);
    assert_eq!(wrong_grant, 0);
    direct
        .query_one(
            "SELECT set_config('pulso.observation_grant','grant-a',false),
      set_config('pulso.observation_purpose','wrong_purpose',false)",
            &[],
        )
        .unwrap();
    let wrong_purpose: i64 = direct
        .query_one(
            "SELECT count(*) FROM pulso_platform_observation_events WHERE tenant_id='tenant-a'",
            &[],
        )
        .unwrap()
        .get(0);
    assert_eq!(wrong_purpose, 0);
    direct
        .query_one(
            "SELECT set_config('pulso.observation_grant','grant-a',false),
      set_config('pulso.observation_purpose','platform_observation',false)",
            &[],
        )
        .unwrap();
    let cross_tenant: i64 = direct
        .query_one(
            "SELECT count(*) FROM pulso_platform_observation_events WHERE tenant_id='tenant-b'",
            &[],
        )
        .unwrap()
        .get(0);
    assert_eq!(cross_tenant, 0);
    assert!(direct.execute(
        "INSERT INTO pulso_platform_observation_events
         (tenant_id,source_id,source_event_id,event_digest,event_json,occurred_at_ms,received_at_ms,batch_digest)
         VALUES ('tenant-b','src','ev','sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
                 '{}'::jsonb,1,1,'sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa')", &[],
    ).is_err());
    let mut explain_client = Client::connect(&url, NoTls).unwrap();
    explain_client
        .batch_execute("SET enable_seqscan=off")
        .unwrap();
    let event_plan = explain_client
        .query(
            "EXPLAIN (COSTS OFF) SELECT event_json FROM pulso_platform_observation_events
         WHERE tenant_id=$1 AND occurred_at_ms >= $2 AND occurred_at_ms < $3
           AND received_at_ms <= $4",
            &[
                &"tenant-a",
                &1_759_320_000_000_i64,
                &1_759_406_400_000_i64,
                &1_759_320_040_000_i64,
            ],
        )
        .unwrap()
        .into_iter()
        .map(|row| row.get::<_, String>(0))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(event_plan.contains("Index Scan"), "{event_plan}");
    let coverage_plan = explain_client
        .query(
            "EXPLAIN (COSTS OFF) SELECT coverage FROM pulso_platform_observation_batches
         WHERE tenant_id=$1 AND (coverage->>'window_start_ms')::bigint=$2
           AND (coverage->>'window_end_ms')::bigint=$3 AND max_received_at_ms <= $4",
            &[
                &"tenant-a",
                &1_759_320_000_000_i64,
                &1_759_406_400_000_i64,
                &1_759_320_040_000_i64,
            ],
        )
        .unwrap()
        .into_iter()
        .map(|row| row.get::<_, String>(0))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(coverage_plan.contains("Index Scan"), "{coverage_plan}");
    let indexes = explain_client.query(
        "SELECT indexname,indexdef FROM pg_indexes
         WHERE tablename IN ('pulso_platform_observation_batches','pulso_platform_observation_events')",
        &[],
    ).unwrap().into_iter().map(|row| (row.get::<_, String>(0), row.get::<_, String>(1)))
      .collect::<std::collections::BTreeMap<_, _>>();
    let window_index = indexes
        .get("pulso_platform_observation_batches_window_idx")
        .unwrap();
    assert!(window_index.contains("window_start_ms") && window_index.contains("window_end_ms"));
    assert!(window_index.contains("max_received_at_ms"));
    let join_index = indexes
        .get("pulso_platform_observation_events_batch_idx")
        .unwrap();
    assert!(join_index.contains("source_id") && join_index.contains("batch_digest"));
    let time_index = indexes
        .get("pulso_platform_observation_events_time_idx")
        .unwrap();
    assert!(time_index.contains("occurred_at_ms") && time_index.contains("received_at_ms"));
}
