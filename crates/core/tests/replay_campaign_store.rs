use improvement_engine_core::replay_clock::{
    ReplayAvailabilityProfile, ReplayCase, ReplayEvent, ReplayProtocol, plan_replay,
};
use improvement_engine_core::replay_protocol::store::{
    CheckpointConflict, PostgresReplayCampaignStore,
};
use improvement_engine_core::replay_protocol::{ReplayCampaign, ReplayCampaignError};
use std::sync::{Arc, Barrier, Mutex};

const SNAPSHOT: &str = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const CONFIG: &str = "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const RECEIPT_A: &str = "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
const RECEIPT_B: &str = "sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";
static TEST_LOCK: Mutex<()> = Mutex::new(());

fn plan(protocol: ReplayProtocol) -> improvement_engine_core::replay_clock::ReplayPlan {
    plan_replay(
        vec![
            ReplayCase::new("sensitive-case-a", 10),
            ReplayCase::new("sensitive-case-b", 20),
            ReplayCase::new("sensitive-case-c", 30),
        ],
        vec![ReplayEvent::new(
            "sensitive-event-a",
            "sensitive-case-a",
            9,
            9,
            None,
        )],
        ReplayAvailabilityProfile::EventTimeZeroLagAssumption,
        protocol,
    )
    .unwrap()
}

fn connect() -> postgres::Client {
    assert_eq!(
        std::env::var("PULSO_ALLOW_DESTRUCTIVE_TEST_DB").as_deref(),
        Ok("1")
    );
    let url = std::env::var("PULSO_TEST_POSTGRES_URL")
        .expect("isolated PULSO_TEST_POSTGRES_URL is required");
    postgres::Client::connect(&url, postgres::NoTls).expect("connect isolated PostgreSQL")
}

fn prepare(client: &mut postgres::Client) {
    client
        .batch_execute(include_str!(
            "../../../migrations/0006_pulso_replay_campaign_checkpoint.sql"
        ))
        .unwrap();
    client
        .batch_execute(
            "DROP TRIGGER IF EXISTS pulso_test_fail_checkpoint_head_update ON pulso_replay_campaign_checkpoint_heads; \
             DROP FUNCTION IF EXISTS pulso_test_fail_checkpoint_head_update(); \
             TRUNCATE pulso_replay_campaign_checkpoint_revisions, pulso_replay_campaign_checkpoint_heads",
        )
        .unwrap();
}

#[test]
fn checkpoint_wire_is_canonical_versioned_and_contains_no_raw_identifiers() {
    let mut campaign = ReplayCampaign::new(
        plan(ReplayProtocol::Prequential),
        SNAPSHOT.into(),
        CONFIG.into(),
        3,
    )
    .unwrap();
    let first = campaign.next_work().unwrap().unwrap();
    campaign.seal(&first, RECEIPT_A).unwrap();
    campaign
        .propose_revision("private-candidate-id", 4)
        .unwrap();

    let encoded = PostgresReplayCampaignStore::encode(&campaign).unwrap();
    let text = std::str::from_utf8(&encoded).unwrap();
    assert!(text.contains("\"wire_version\":1"));
    for sensitive in [
        "sensitive-case-a",
        "sensitive-case-b",
        "sensitive-event-a",
        "private-candidate-id",
    ] {
        assert!(!text.contains(sensitive), "wire leaked {sensitive}");
    }
    assert_eq!(
        PostgresReplayCampaignStore::encode(&campaign).unwrap(),
        encoded
    );
    assert_eq!(
        PostgresReplayCampaignStore::decode(
            &encoded,
            plan(ReplayProtocol::Prequential),
            SNAPSHOT,
            CONFIG
        )
        .unwrap()
        .summary(),
        campaign.summary()
    );
}

#[test]
fn update_identity_retry_and_conflicting_reuse_survive_durable_restore() {
    let mut campaign = ReplayCampaign::new(
        plan(ReplayProtocol::Prequential),
        SNAPSHOT.into(),
        CONFIG.into(),
        3,
    )
    .unwrap();
    let first = campaign.next_work().unwrap().unwrap();
    campaign.seal(&first, RECEIPT_A).unwrap();
    campaign
        .propose_revision("private-candidate-id", 4)
        .unwrap();

    let encoded = PostgresReplayCampaignStore::encode(&campaign).unwrap();
    let mut restored = PostgresReplayCampaignStore::decode(
        &encoded,
        plan(ReplayProtocol::Prequential),
        SNAPSHOT,
        CONFIG,
    )
    .unwrap();
    assert_eq!(
        restored.propose_revision("private-candidate-id", 4),
        Ok(()),
        "retry of the same source update ID must remain idempotent after restart"
    );
    assert!(matches!(
        restored.propose_revision("private-candidate-id", 5),
        Err(ReplayCampaignError::Protocol(
            improvement_engine_core::replay_protocol::ReplayProtocolError::ConflictingUpdate(_)
        ))
    ));
}

#[test]
fn raw_update_id_cannot_alias_the_private_persisted_identity_namespace() {
    let mut campaign = ReplayCampaign::new(
        plan(ReplayProtocol::Prequential),
        SNAPSHOT.into(),
        CONFIG.into(),
        3,
    )
    .unwrap();
    let first = campaign.next_work().unwrap().unwrap();
    campaign.seal(&first, RECEIPT_A).unwrap();
    campaign.propose_revision("candidate-alpha", 4).unwrap();
    let first_wire: serde_json::Value =
        serde_json::from_slice(&PostgresReplayCampaignStore::encode(&campaign).unwrap()).unwrap();
    let marker = format!(
        "persisted:{}",
        first_wire["updates"][0]["update_id_digest"]
            .as_str()
            .unwrap()
    );

    let second = campaign.next_work().unwrap().unwrap();
    campaign.seal(&second, RECEIPT_B).unwrap();
    campaign.propose_revision(&marker, 5).unwrap();
    let encoded = PostgresReplayCampaignStore::encode(&campaign).unwrap();
    let wire: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(wire["updates"].as_array().unwrap().len(), 2);

    let restored = PostgresReplayCampaignStore::decode(
        &encoded,
        plan(ReplayProtocol::Prequential),
        SNAPSHOT,
        CONFIG,
    )
    .unwrap();
    assert_eq!(
        PostgresReplayCampaignStore::encode(&restored).unwrap(),
        encoded
    );
}

#[test]
fn restore_replays_seals_and_update_journal_and_rejects_changed_bindings() {
    let mut campaign = ReplayCampaign::new(
        plan(ReplayProtocol::Prequential),
        SNAPSHOT.into(),
        CONFIG.into(),
        3,
    )
    .unwrap();
    let first = campaign.next_work().unwrap().unwrap();
    campaign.seal(&first, RECEIPT_A).unwrap();
    campaign
        .propose_revision("private-candidate-id", 4)
        .unwrap();
    let second = campaign.next_work().unwrap().unwrap();
    campaign.seal(&second, RECEIPT_B).unwrap();

    let encoded = PostgresReplayCampaignStore::encode(&campaign).unwrap();
    let mut resumed = PostgresReplayCampaignStore::decode(
        &encoded,
        plan(ReplayProtocol::Prequential),
        SNAPSHOT,
        CONFIG,
    )
    .unwrap();
    assert_eq!(resumed.summary(), campaign.summary());
    assert_eq!(resumed.next_work().unwrap(), campaign.next_work().unwrap());
    assert!(matches!(
        PostgresReplayCampaignStore::decode(
            &encoded,
            plan(ReplayProtocol::Frozen),
            SNAPSHOT,
            CONFIG
        ),
        Err(CheckpointConflict::Campaign(
            ReplayCampaignError::CheckpointBindingMismatch
        ))
    ));

    let mut tampered = serde_json::from_slice::<serde_json::Value>(&encoded).unwrap();
    tampered["next_cohort_index"] = serde_json::json!(1);
    let tampered = serde_json::to_vec(&tampered).unwrap();
    assert!(
        PostgresReplayCampaignStore::decode(
            &tampered,
            plan(ReplayProtocol::Prequential),
            SNAPSHOT,
            CONFIG
        )
        .is_err()
    );
}

#[test]
#[ignore = "requires isolated PULSO_TEST_POSTGRES_URL and explicit destructive-test consent"]
fn postgres_checkpoint_commit_is_atomic_cas_idempotent_and_rejects_conflicting_seals() {
    let _guard = TEST_LOCK.lock().unwrap();
    let mut client = connect();
    prepare(&mut client);

    let mut first = ReplayCampaign::new(
        plan(ReplayProtocol::Frozen),
        SNAPSHOT.into(),
        CONFIG.into(),
        3,
    )
    .unwrap();
    let first_work = first.next_work().unwrap().unwrap();
    first.seal(&first_work, RECEIPT_A).unwrap();
    let version = PostgresReplayCampaignStore::commit(&mut client, &first, 0, 0).unwrap();
    assert_eq!(version, 1);
    assert_eq!(
        PostgresReplayCampaignStore::commit(&mut client, &first, 0, 0).unwrap(),
        1,
        "exact retry must be idempotent"
    );
    assert_eq!(
        PostgresReplayCampaignStore::load(
            &mut client,
            plan(ReplayProtocol::Frozen),
            SNAPSHOT,
            CONFIG,
            3
        )
        .unwrap()
        .unwrap()
        .summary(),
        first.summary()
    );

    let mut conflicting = ReplayCampaign::new(
        plan(ReplayProtocol::Frozen),
        SNAPSHOT.into(),
        CONFIG.into(),
        3,
    )
    .unwrap();
    let conflicting_work = conflicting.next_work().unwrap().unwrap();
    conflicting.seal(&conflicting_work, RECEIPT_B).unwrap();
    assert_eq!(
        PostgresReplayCampaignStore::commit(&mut client, &conflicting, 0, 0),
        Err(CheckpointConflict::ConflictingCommit)
    );
    assert_eq!(
        PostgresReplayCampaignStore::commit(&mut client, &conflicting, 1, 1),
        Err(CheckpointConflict::ConflictingCommit)
    );
    client
        .execute(
            "UPDATE pulso_replay_campaign_checkpoint_heads SET cursor=cursor+1 WHERE campaign_digest=$1",
            &[&first.summary().campaign_digest()],
        )
        .unwrap();
    assert_eq!(
        PostgresReplayCampaignStore::load(
            &mut client,
            plan(ReplayProtocol::Frozen),
            SNAPSHOT,
            CONFIG,
            3
        ),
        Err(CheckpointConflict::InvalidCheckpoint)
    );
}

#[test]
#[ignore = "requires isolated PULSO_TEST_POSTGRES_URL and explicit destructive-test consent"]
fn postgres_two_writers_cas_race_and_failed_head_update_roll_back_revision() {
    let _guard = TEST_LOCK.lock().unwrap();
    let mut setup = connect();
    prepare(&mut setup);
    let database_url = std::env::var("PULSO_TEST_POSTGRES_URL").unwrap();
    let barrier = Arc::new(Barrier::new(3));
    let mut handles = Vec::new();
    for receipt in [RECEIPT_A.to_owned(), RECEIPT_B.to_owned()] {
        let barrier = Arc::clone(&barrier);
        let database_url = database_url.clone();
        handles.push(std::thread::spawn(move || {
            let mut campaign = ReplayCampaign::new(
                plan(ReplayProtocol::Frozen),
                "sha256:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee".into(),
                CONFIG.into(),
                3,
            )
            .unwrap();
            let work = campaign.next_work().unwrap().unwrap();
            campaign.seal(&work, &receipt).unwrap();
            let mut client = postgres::Client::connect(&database_url, postgres::NoTls).unwrap();
            barrier.wait();
            PostgresReplayCampaignStore::commit(&mut client, &campaign, 0, 0)
        }));
    }
    barrier.wait();
    let results = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        results
            .iter()
            .filter(|result| result.as_ref().ok() == Some(&1))
            .count(),
        1
    );
    assert_eq!(
        results
            .iter()
            .filter(|result| {
                result.as_ref().err() == Some(&CheckpointConflict::ConflictingCommit)
            })
            .count(),
        1
    );
    let committed: i64 = setup
        .query_one(
            "SELECT count(*) FROM pulso_replay_campaign_checkpoint_revisions",
            &[],
        )
        .unwrap()
        .get(0);
    assert_eq!(committed, 1, "exactly one concurrent writer must commit");

    setup
        .batch_execute(
            "CREATE FUNCTION pulso_test_fail_checkpoint_head_update() RETURNS trigger \
             LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'injected checkpoint CAS failure'; END $$; \
             CREATE TRIGGER pulso_test_fail_checkpoint_head_update \
             BEFORE UPDATE ON pulso_replay_campaign_checkpoint_heads \
             FOR EACH ROW EXECUTE FUNCTION pulso_test_fail_checkpoint_head_update();",
        )
        .unwrap();
    let mut rollback_campaign = ReplayCampaign::new(
        plan(ReplayProtocol::Frozen),
        "sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff".into(),
        CONFIG.into(),
        3,
    )
    .unwrap();
    let rollback_work = rollback_campaign.next_work().unwrap().unwrap();
    rollback_campaign.seal(&rollback_work, RECEIPT_A).unwrap();
    assert_eq!(
        PostgresReplayCampaignStore::commit(&mut setup, &rollback_campaign, 0, 0),
        Err(CheckpointConflict::Storage)
    );
    setup
        .batch_execute(
            "DROP TRIGGER pulso_test_fail_checkpoint_head_update ON pulso_replay_campaign_checkpoint_heads; \
             DROP FUNCTION pulso_test_fail_checkpoint_head_update();",
        )
        .unwrap();
    let rolled_back: i64 = setup
        .query_one(
            "SELECT count(*) FROM pulso_replay_campaign_checkpoint_revisions \
             WHERE campaign_digest=$1",
            &[&"sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"],
        )
        .unwrap()
        .get(0);
    assert_eq!(
        rolled_back, 0,
        "failed head CAS must roll back its inserted revision"
    );
}
