use improvement_engine_core::quota_grant::{
    AuthorizedGrant, QuotaGrantError, QuotaLedger, QuotaLimit, QuotaReservation, QuotaResource,
    QuotaWindow,
};
use improvement_engine_core::run_config::{Cadence, EligibleSource, RunConfig, ScanBudget};

fn source() -> EligibleSource {
    EligibleSource::new("latam_bank", "call_center_interactions").expect("valid source")
}

fn config(max_rows_per_source: u64) -> RunConfig {
    RunConfig::new(
        1,
        Cadence::scheduled_every_minutes(60).expect("valid cadence"),
        ScanBudget::new(1, max_rows_per_source, 16 * 1024 * 1024).expect("valid budget"),
        vec![source()],
    )
    .expect("valid config")
}

fn query_bytes() -> QuotaResource {
    QuotaResource::new("query_bytes").expect("valid resource")
}

fn window() -> QuotaWindow {
    window_for("demo").expect("valid quota window")
}

fn window_for(tenant_id: &str) -> Result<QuotaWindow, QuotaGrantError> {
    window_for_end(tenant_id, 1_700_086_400)
}

fn window_for_end(tenant_id: &str, end_unix_seconds: u64) -> Result<QuotaWindow, QuotaGrantError> {
    QuotaWindow::new(tenant_id, query_bytes(), 1_700_000_000, end_unix_seconds)
}

fn grant(grant_id: &str) -> AuthorizedGrant {
    grant_for("demo", "policy-revision-001", grant_id, 10)
}

fn grant_for(
    tenant_id: &str,
    authority_ref: &str,
    grant_id: &str,
    max_units: u64,
) -> AuthorizedGrant {
    AuthorizedGrant::issue(
        grant_id,
        authority_ref,
        tenant_id,
        query_bytes(),
        max_units,
        1_700_086_400,
    )
    .expect("valid authorized grant")
}

fn configured_ledger() -> QuotaLedger {
    let mut ledger = QuotaLedger::new();
    ledger
        .configure_limit(QuotaLimit::new(window(), 10).expect("valid global limit"))
        .expect("window is configured before reservation");
    ledger
}

fn reservation(
    key: &str,
    run_config: &RunConfig,
    authorized_grant: AuthorizedGrant,
    units: u64,
    at: u64,
) -> QuotaReservation {
    QuotaReservation::new(key, window(), run_config, authorized_grant, units, at)
        .expect("valid reservation")
}

#[test]
fn a_new_config_cannot_reset_a_tenants_global_window_budget() {
    let first_config = config(50_000);
    let replacement_config = config(50_001);
    let mut ledger = configured_ledger();

    let first = ledger
        .reserve(reservation(
            "request-001",
            &first_config,
            grant("grant-001"),
            7,
            1_700_000_001,
        ))
        .expect("first reservation fits");
    let second = ledger
        .reserve(reservation(
            "request-002",
            &replacement_config,
            grant("grant-002"),
            4,
            1_700_000_002,
        ))
        .expect("quota exhaustion is a handled outcome");

    assert!(first.is_reserved());
    assert!(second.is_deferred_for_quota());
    assert_ne!(first.config_identity(), second.config_identity());
    assert_eq!(ledger.reserved_units(&window()), 7);
}

#[test]
fn expired_or_revoked_grants_cannot_reserve_a_new_run() {
    let run_config = config(50_000);
    let mut ledger = configured_ledger();

    let expired = ledger.reserve(reservation(
        "expired-request",
        &run_config,
        AuthorizedGrant::issue(
            "expired-grant",
            "policy-revision-001",
            "demo",
            query_bytes(),
            10,
            1_700_000_001,
        )
        .expect("valid expired grant record"),
        1,
        1_700_000_001,
    ));
    assert_eq!(
        expired,
        Err(QuotaGrantError::GrantExpired {
            grant_id: "expired-grant".to_owned(),
        })
    );

    let active = grant("revocable-grant");
    let revocation = ledger
        .revoke_grant(
            "demo",
            "policy-revision-001",
            "revocable-grant",
            1_700_000_010,
            "policy_changed",
        )
        .expect("valid revocation")
        .receipt_id()
        .to_owned();
    let revoked = ledger.reserve(reservation(
        "revoked-request",
        &run_config,
        active,
        1,
        1_700_000_010,
    ));
    assert_eq!(
        revoked,
        Err(QuotaGrantError::GrantRevoked {
            grant_id: "revocable-grant".to_owned(),
            revocation_receipt_id: revocation,
        })
    );
    assert_eq!(ledger.reserved_units(&window()), 0);
}

#[test]
fn replaying_the_same_request_returns_the_same_receipt_without_double_reserving() {
    let run_config = config(50_000);
    let mut ledger = configured_ledger();

    let first = ledger
        .reserve(reservation(
            "retryable-request",
            &run_config,
            grant("retryable-grant"),
            3,
            1_700_000_001,
        ))
        .expect("first reservation");
    let retry = ledger
        .reserve(reservation(
            "retryable-request",
            &run_config,
            grant("retryable-grant"),
            3,
            1_700_000_001,
        ))
        .expect("idempotent retry");

    assert_eq!(first, retry);
    assert_eq!(ledger.reserved_units(&window()), 3);
    assert!(first.receipt_id().starts_with("quota-receipt:sha256:"));
}

#[test]
fn reusing_an_idempotency_key_for_a_different_request_is_rejected() {
    let run_config = config(50_000);
    let mut ledger = configured_ledger();
    ledger
        .reserve(reservation(
            "same-key",
            &run_config,
            grant("same-key-grant"),
            3,
            1_700_000_001,
        ))
        .expect("first reservation");

    assert_eq!(
        ledger.reserve(reservation(
            "same-key",
            &run_config,
            grant("same-key-grant"),
            4,
            1_700_000_001,
        )),
        Err(QuotaGrantError::IdempotencyConflict {
            idempotency_key: "same-key".to_owned(),
        })
    );
    assert_eq!(ledger.reserved_units(&window()), 3);
}

#[test]
fn tenants_can_reuse_an_idempotency_key_without_sharing_a_receipt_or_balance() {
    let run_config = config(50_000);
    let mut ledger = configured_ledger();
    let other_window = window_for("other_tenant").expect("valid second tenant window");
    ledger
        .configure_limit(QuotaLimit::new(other_window.clone(), 10).expect("valid global limit"))
        .expect("second tenant window is independent");

    let demo = ledger
        .reserve(reservation(
            "shared-key",
            &run_config,
            grant("shared-grant"),
            3,
            1_700_000_001,
        ))
        .expect("demo reservation");
    let other = ledger
        .reserve(
            QuotaReservation::new(
                "shared-key",
                other_window.clone(),
                &run_config,
                grant_for("other_tenant", "policy-revision-001", "shared-grant", 10),
                3,
                1_700_000_001,
            )
            .expect("other tenant reservation"),
        )
        .expect("other tenant reservation must not conflict");

    assert!(demo.is_reserved());
    assert!(other.is_reserved());
    assert_ne!(demo.receipt_id(), other.receipt_id());
    assert_eq!(ledger.reserved_units(&window()), 3);
    assert_eq!(ledger.reserved_units(&other_window), 3);
}

#[test]
fn same_grant_id_under_different_authorities_does_not_share_usage_or_revocation() {
    let run_config = config(50_000);
    let mut ledger = QuotaLedger::new();
    ledger
        .configure_limit(QuotaLimit::new(window(), 20).expect("valid global limit"))
        .expect("configured window");

    let authority_a = ledger
        .reserve(reservation(
            "authority-a",
            &run_config,
            grant_for("demo", "authority-a", "shared-grant", 10),
            7,
            1_700_000_001,
        ))
        .expect("first authority reservation");
    let authority_b = ledger
        .reserve(reservation(
            "authority-b",
            &run_config,
            grant_for("demo", "authority-b", "shared-grant", 10),
            4,
            1_700_000_002,
        ))
        .expect("independent authority grant budget");
    ledger
        .revoke_grant(
            "demo",
            "authority-a",
            "shared-grant",
            1_700_000_010,
            "policy_changed",
        )
        .expect("authority A revocation");
    let authority_b_after_other_revocation = ledger
        .reserve(reservation(
            "authority-b-after-revoke",
            &run_config,
            grant_for("demo", "authority-b", "shared-grant", 10),
            1,
            1_700_000_011,
        ))
        .expect("authority B remains valid");

    assert!(authority_a.is_reserved());
    assert!(authority_b.is_reserved());
    assert!(authority_b_after_other_revocation.is_reserved());
    assert_eq!(ledger.reserved_units(&window()), 12);
}

#[test]
fn changing_window_end_cannot_create_a_second_budget_for_the_same_window_start() {
    let run_config = config(50_000);
    let mut ledger = configured_ledger();
    ledger
        .reserve(reservation(
            "original-window",
            &run_config,
            grant("original-window-grant"),
            7,
            1_700_000_001,
        ))
        .expect("original window reservation");
    let changed_end = window_for_end("demo", 1_700_172_800).expect("valid changed-end window");

    assert_eq!(
        ledger.reserve(
            QuotaReservation::new(
                "changed-end-window",
                changed_end.clone(),
                &run_config,
                grant("changed-end-grant"),
                4,
                1_700_000_002,
            )
            .expect("syntactically valid reservation"),
        ),
        Err(QuotaGrantError::QuotaWindowEndMismatch {
            tenant_id: "demo".to_owned(),
            resource: "query_bytes".to_owned(),
            start_unix_seconds: 1_700_000_000,
            expected_end_unix_seconds: 1_700_086_400,
            actual_end_unix_seconds: 1_700_172_800,
        })
    );
    assert_eq!(ledger.reserved_units(&window()), 7);
    assert_eq!(ledger.reserved_units(&changed_end), 7);
}
