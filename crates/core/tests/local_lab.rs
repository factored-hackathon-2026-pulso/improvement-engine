use std::collections::BTreeMap;

use improvement_engine_core::ArtifactReference;
use improvement_engine_core::local_lab::{
    InMemoryLabGrantAuthority, InMemoryLabSourceAuthority, LabAccess, LabDataClassification,
    LabError, LabGrant, LabQuery, LabSource, LabSourceApprovalPort, LabSourceManifest, LabTable,
    LocalInvestigationLab, QueryFilter,
};

const TENANT: &str = "bank_demo";

fn digest(seed: char) -> String {
    format!("sha256:{}", seed.to_string().repeat(64))
}

fn snapshot_ref() -> ArtifactReference {
    ArtifactReference {
        tenant_id: TENANT.to_owned(),
        id: "018f3a54-7eaf-7c83-8a04-5bf4ec1a9d26".to_owned(),
        revision: 1,
        digest: digest('a'),
    }
}

fn source() -> LabSource {
    LabSource::new(
        LabSourceManifest {
            tenant_id: TENANT.to_owned(),
            snapshot_ref: snapshot_ref(),
            source_contract_digest: digest('b'),
            source_digest: digest('c'),
            transform_digest: digest('d'),
            cutoff_unix_seconds: 100,
            classification: LabDataClassification::Treated,
            safe_for_discovery: true,
        },
        vec![LabTable::new(
            "contacts",
            vec!["customer_id", "reason", "resolved"],
            vec![
                row(&[
                    ("customer_id", "c-1"),
                    ("reason", "payment"),
                    ("resolved", "false"),
                ]),
                row(&[
                    ("customer_id", "c-2"),
                    ("reason", "card"),
                    ("resolved", "true"),
                ]),
                row(&[
                    ("customer_id", "c-3"),
                    ("reason", "payment"),
                    ("resolved", "false"),
                ]),
            ],
        )],
    )
    .expect("valid approved local source")
}

fn approved_source(source: LabSource) -> improvement_engine_core::local_lab::ApprovedLabSource {
    let mut authority = InMemoryLabSourceAuthority;
    authority.approve(source).unwrap()
}

fn row(values: &[(&str, &str)]) -> BTreeMap<String, String> {
    values
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect()
}

fn access(expires_at: u64) -> LabAccess {
    LabAccess::new(
        "run-001",
        TENANT,
        "investigation",
        "grant-001",
        "authority-001",
        snapshot_ref(),
        expires_at,
    )
}

fn authorized_lab(expires_at: u64) -> (LocalInvestigationLab, LabAccess) {
    let access = access(expires_at);
    let mut authority = InMemoryLabGrantAuthority::default();
    authority.issue(LabGrant::from_access(&access));
    (LocalInvestigationLab::new(authority), access)
}

#[test]
fn two_dependent_read_only_queries_produce_ordered_sealed_receipts_without_mutating_source() {
    let (mut lab, access) = authorized_lab(200);
    let original = source();
    let approved = approved_source(original.clone());
    let session = lab.open(access.clone(), approved, 100).unwrap();

    let first = lab
        .query(
            session.session_id(),
            &access,
            LabQuery::select(
                "contacts",
                vec!["customer_id"],
                Some(QueryFilter::equals("reason", "payment")),
            ),
            101,
        )
        .unwrap();
    assert_eq!(
        first.rows(),
        &[
            row(&[("customer_id", "c-1")]),
            row(&[("customer_id", "c-3")])
        ]
    );

    let second = lab
        .query(
            session.session_id(),
            &access,
            LabQuery::dependent_select(
                "contacts",
                vec!["customer_id", "resolved"],
                "customer_id",
                first.receipt().digest.clone(),
                "customer_id",
            ),
            102,
        )
        .unwrap();

    assert_eq!(second.receipt().sequence, 2);
    assert_eq!(
        second.receipt().depends_on.as_deref(),
        Some(first.receipt().digest.as_str())
    );
    assert_eq!(second.receipt().source_snapshot_ref, snapshot_ref());
    assert_eq!(second.receipt().source_contract_digest, digest('b'));
    assert_eq!(second.receipt().transform_digest, digest('d'));
    assert_eq!(second.receipt().cutoff_unix_seconds, 100);
    assert_eq!(original, source(), "the approved source remains immutable");
}

#[test]
fn rejects_cross_tenant_snapshot_or_grant_before_query_execution() {
    let (mut lab, access) = authorized_lab(200);
    let approved = approved_source(source());
    let session = lab.open(access.clone(), approved, 100).unwrap();
    let query = LabQuery::select("contacts", vec!["customer_id"], None);

    let mut cross_tenant = access.clone();
    cross_tenant.tenant_id = "other_bank".to_owned();
    assert_eq!(
        lab.query(session.session_id(), &cross_tenant, query.clone(), 101),
        Err(LabError::AccessDenied)
    );

    let mut cross_snapshot = access.clone();
    cross_snapshot.snapshot_ref.digest = digest('e');
    assert_eq!(
        lab.query(session.session_id(), &cross_snapshot, query.clone(), 101),
        Err(LabError::AccessDenied)
    );

    let mut cross_grant = access;
    cross_grant.grant_id = "grant-other".to_owned();
    assert_eq!(
        lab.query(session.session_id(), &cross_grant, query, 101),
        Err(LabError::AccessDenied)
    );
}

#[test]
fn denies_unbounded_disallowed_write_and_external_io_shapes_before_execution() {
    let (mut lab, access) = authorized_lab(200);
    let approved = approved_source(source());
    let session = lab.open(access.clone(), approved, 100).unwrap();

    assert_eq!(
        lab.query(
            session.session_id(),
            &access,
            LabQuery::select("contacts", Vec::<String>::new(), None),
            101
        ),
        Err(LabError::QueryShapeDenied {
            reason: "projection cannot be empty"
        })
    );
    assert_eq!(
        lab.query(
            session.session_id(),
            &access,
            LabQuery::source_write("contacts"),
            101
        ),
        Err(LabError::SourceWriteDenied)
    );
    assert_eq!(
        lab.query(
            session.session_id(),
            &access,
            LabQuery::external_io("https://example.test"),
            101
        ),
        Err(LabError::ExternalIoDenied)
    );
    assert_eq!(
        lab.query(
            session.session_id(),
            &access,
            LabQuery::select(
                "contacts",
                vec!["customer_id"],
                Some(QueryFilter::equals("reason", "x".repeat(4_097)))
            ),
            101
        ),
        Err(LabError::QueryShapeDenied {
            reason: "filter value exceeds bound"
        })
    );
    assert_eq!(
        lab.receipts(session.session_id(), &access, 101)
            .unwrap()
            .len(),
        0
    );
}

#[test]
fn source_constructor_rejects_oversized_values_or_cross_tenant_provenance_before_sqlite() {
    let oversized = LabSource::new(
        LabSourceManifest {
            tenant_id: TENANT.to_owned(),
            snapshot_ref: snapshot_ref(),
            source_contract_digest: digest('b'),
            source_digest: digest('c'),
            transform_digest: digest('d'),
            cutoff_unix_seconds: 100,
            classification: LabDataClassification::Treated,
            safe_for_discovery: true,
        },
        vec![LabTable::new(
            "contacts",
            vec!["customer_id"],
            vec![row(&[("customer_id", &"x".repeat(4_097))])],
        )],
    );
    assert_eq!(oversized, Err(LabError::InvalidSource));

    let cross_tenant = LabSource::new(
        LabSourceManifest {
            tenant_id: "other_bank".to_owned(),
            snapshot_ref: snapshot_ref(),
            source_contract_digest: digest('b'),
            source_digest: digest('c'),
            transform_digest: digest('d'),
            cutoff_unix_seconds: 100,
            classification: LabDataClassification::Treated,
            safe_for_discovery: true,
        },
        vec![LabTable::new(
            "contacts",
            vec!["customer_id"],
            vec![row(&[("customer_id", "c")])],
        )],
    );
    assert_eq!(cross_tenant, Err(LabError::InvalidSource));
}

#[test]
fn expiry_and_failed_dependent_query_do_not_publish_partial_state() {
    let (mut lab, access) = authorized_lab(105);
    let approved = approved_source(source());
    let session = lab.open(access.clone(), approved, 100).unwrap();
    let first = lab
        .query(
            session.session_id(),
            &access,
            LabQuery::select("contacts", vec!["customer_id"], None),
            101,
        )
        .unwrap();

    assert_eq!(
        lab.query(
            session.session_id(),
            &access,
            LabQuery::dependent_select(
                "contacts",
                vec!["customer_id"],
                "customer_id",
                digest('f'),
                "customer_id"
            ),
            102,
        ),
        Err(LabError::DependencyDenied)
    );
    assert_eq!(
        lab.receipts(session.session_id(), &access, 102)
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        lab.query(
            session.session_id(),
            &access,
            LabQuery::select("contacts", vec!["customer_id"], None),
            105
        ),
        Err(LabError::SessionExpired)
    );
    assert!(!first.receipt().digest.is_empty());
}

#[test]
fn receipt_reads_are_ttl_bound_and_expired_sessions_are_destroyed() {
    let (mut lab, access) = authorized_lab(105);
    let approved = approved_source(source());
    let session = lab.open(access.clone(), approved, 100).unwrap();
    lab.query(
        session.session_id(),
        &access,
        LabQuery::select("contacts", vec!["customer_id"], None),
        101,
    )
    .unwrap();
    assert_eq!(
        lab.receipts(session.session_id(), &access, 105),
        Err(LabError::SessionExpired)
    );
    assert_eq!(lab.purge_expired(105), 1);
    assert_eq!(
        lab.query(
            session.session_id(),
            &access,
            LabQuery::select("contacts", vec!["customer_id"], None),
            105
        ),
        Err(LabError::SessionNotFound)
    );
}

#[test]
fn rejects_pii_or_unapproved_source_before_it_can_open_a_session() {
    let (mut lab, access) = authorized_lab(200);
    let pii = LabSource::new(
        LabSourceManifest {
            tenant_id: TENANT.to_owned(),
            snapshot_ref: snapshot_ref(),
            source_contract_digest: digest('b'),
            source_digest: digest('c'),
            transform_digest: digest('d'),
            cutoff_unix_seconds: 100,
            classification: LabDataClassification::Pii,
            safe_for_discovery: true,
        },
        vec![LabTable::new(
            "contacts",
            vec!["customer_id"],
            vec![row(&[("customer_id", "c")])],
        )],
    )
    .unwrap();
    let mut source_authority = InMemoryLabSourceAuthority;
    assert_eq!(
        source_authority.approve(pii),
        Err(LabError::SourceApprovalDenied)
    );

    let unsafe_source = LabSource::new(
        LabSourceManifest {
            tenant_id: TENANT.to_owned(),
            snapshot_ref: snapshot_ref(),
            source_contract_digest: digest('b'),
            source_digest: digest('c'),
            transform_digest: digest('d'),
            cutoff_unix_seconds: 100,
            classification: LabDataClassification::Treated,
            safe_for_discovery: false,
        },
        vec![LabTable::new(
            "contacts",
            vec!["customer_id"],
            vec![row(&[("customer_id", "c")])],
        )],
    )
    .unwrap();
    assert_eq!(
        source_authority.approve(unsafe_source),
        Err(LabError::SourceApprovalDenied)
    );
    let approved = approved_source(source());
    assert!(lab.open(access, approved, 100).is_ok());
}

#[test]
fn dependent_receipts_are_single_use_and_cannot_be_replayed() {
    let (mut lab, access) = authorized_lab(200);
    let approved = approved_source(source());
    let session = lab.open(access.clone(), approved, 100).unwrap();
    let first = lab
        .query(
            session.session_id(),
            &access,
            LabQuery::select("contacts", vec!["customer_id"], None),
            101,
        )
        .unwrap();
    let dependent = || {
        LabQuery::dependent_select(
            "contacts",
            vec!["customer_id"],
            "customer_id",
            first.receipt().digest.clone(),
            "customer_id",
        )
    };
    lab.query(session.session_id(), &access, dependent(), 102)
        .unwrap();
    assert_eq!(
        lab.query(session.session_id(), &access, dependent(), 103),
        Err(LabError::DependencyDenied)
    );
}

#[test]
fn same_scope_in_two_labs_has_distinct_session_instance_receipts() {
    let (mut first_lab, first_access) = authorized_lab(200);
    let (mut second_lab, second_access) = authorized_lab(200);
    let first_session = first_lab
        .open(first_access.clone(), approved_source(source()), 100)
        .unwrap();
    let second_session = second_lab
        .open(second_access.clone(), approved_source(source()), 100)
        .unwrap();
    let first = first_lab
        .query(
            first_session.session_id(),
            &first_access,
            LabQuery::select("contacts", vec!["customer_id"], None),
            101,
        )
        .unwrap();
    let second = second_lab
        .query(
            second_session.session_id(),
            &second_access,
            LabQuery::select("contacts", vec!["customer_id"], None),
            101,
        )
        .unwrap();
    assert_ne!(first.receipt().session_id, second.receipt().session_id);
}
