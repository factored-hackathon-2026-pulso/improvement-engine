//! U08-E adapter for read-only E0 query results.
//!
//! It does not open source files, execute arbitrary SQL, write the source,
//! join a later result, inspect labels, call a model, or create a Scout draft.
//! U04-B owns replay availability; U08 owns the governed local-lab receipt.

use crate::enriched_history::VerifiedE0QueryProjection;
use crate::local_lab::{GovernedE0QueryCandidate, QueryResult};

/// Public marker for the E0 receipt-verification boundary. Construction and
/// admission remain crate-private, so a transport caller cannot manufacture an
/// E0 result from untrusted rows or a caller-selected cutoff.
pub struct E0QueryLab {
    _private: (),
}

/// Opaque read-only result after its existing U08 receipt was bound to one
/// sealed U04-B table projection. It is not authority to write, evaluate,
/// execute Agent Core, publish a candidate or release anything.
pub struct VerifiedE0QueryResult {
    result: QueryResult,
    /// Opaque issuer-attested commitment. It is deliberately not written into
    /// the public, self-digesting U08 receipt: a digest is integrity evidence,
    /// not proof that an E0 authority issued it.
    _attestation: E0QueryAttestation,
}

struct E0QueryAttestation {
    #[allow(dead_code)] // Read by the future internal E0 orchestration ledger.
    commitment: String,
}

impl VerifiedE0QueryResult {
    #[must_use]
    pub fn result(&self) -> &QueryResult {
        &self.result
    }

    #[must_use]
    pub fn authorizes_source_write_or_release(&self) -> bool {
        false
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum E0QueryLabError {
    ReceiptInvalid,
    ProjectionMismatch,
    CrossTenantDenied,
    LabelOrUnknownFieldDenied,
    FutureOrCutoffDenied,
    LaterJoinDenied,
}

impl E0QueryLab {
    /// Trusted composition takes an opaque receipt retrieved from U08's own
    /// live ledger and binds it to an opaque U04-B projection. A public
    /// `QueryResult` or a self-consistent digest cannot enter this boundary.
    #[allow(dead_code)] // Invoked by the future E0 run composition root.
    pub(crate) fn admit(
        projection: &VerifiedE0QueryProjection,
        candidate: GovernedE0QueryCandidate,
    ) -> Result<VerifiedE0QueryResult, E0QueryLabError> {
        let (rows, receipt, source_table, u04_source_snapshot_binding) = candidate.into_parts();
        if !receipt.has_valid_digest()
            || !receipt.binds_rows(&rows)
            || receipt.row_count != rows.len()
        {
            return Err(E0QueryLabError::ReceiptInvalid);
        }
        if receipt.tenant_id != projection.tenant_id()
            || receipt.source_snapshot_ref.tenant_id != projection.tenant_id()
        {
            return Err(E0QueryLabError::CrossTenantDenied);
        }
        if receipt.cutoff_unix_seconds != projection.cutoff_at_unix_seconds() {
            return Err(E0QueryLabError::FutureOrCutoffDenied);
        }
        if receipt.queried_table != projection.table()
            || receipt.source_contract_digest != projection.source_contract_digest()
            || receipt.source_digest != projection.source_digest()
            || receipt.transform_digest != projection.transform_digest()
        {
            return Err(E0QueryLabError::ProjectionMismatch);
        }
        if receipt.depends_on.is_some() {
            return Err(E0QueryLabError::LaterJoinDenied);
        }
        if receipt
            .accessed_columns
            .iter()
            .any(|field| !projection.allows_field(field))
        {
            return Err(E0QueryLabError::LabelOrUnknownFieldDenied);
        }
        if !projection.matches_lab_source(
            &u04_source_snapshot_binding,
            &source_table.name,
            &source_table.columns,
            &source_table.rows,
        ) {
            return Err(E0QueryLabError::ProjectionMismatch);
        }
        let attestation = E0QueryAttestation {
            commitment: format!(
                "{}:{}:{}:{}:{}:{}",
                receipt.digest,
                projection.source_snapshot_digest(),
                projection.availability_profile_digest(),
                projection.field_commitment(),
                projection.replay_projection_digest(),
                projection.source_evidence_digest(),
            ),
        };
        Ok(VerifiedE0QueryResult {
            result: QueryResult::untrusted(rows, receipt),
            _attestation: attestation,
        })
    }
}

/// ```compile_fail
/// use improvement_engine_core::e0_query_lab::E0QueryLab;
/// let _ = E0QueryLab { _private: () };
/// ```
const _NO_PUBLIC_E0_LAB_CONSTRUCTOR: () = ();

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::ArtifactReference;
    use crate::enriched_history::{
        AvailabilityClockMode, AvailabilityProfile, EnrichedHistoryAdapter,
        EnrichedHistoryManifest, PackageFile, ProvenanceDigests, ReplayRowAvailability, TableInput,
        replay_projection_digest,
    };
    use crate::local_lab::{
        ApprovedLabSource, InMemoryLabGrantAuthority, InMemoryLabSourceAuthority, LabAccess,
        LabGrant, LabQuery, LabSource, LabSourceApprovalPort, LabSourceManifest, LabTable,
        LocalInvestigationLab,
    };
    use crate::source_validation::SourceSnapshot;
    use serde_json::json;

    fn digest(byte: char) -> String {
        format!("sha256:{}", byte.to_string().repeat(64))
    }

    fn projection(tenant: &str) -> VerifiedE0QueryProjection {
        projection_for_snapshot(tenant, 'e')
    }

    fn projection_for_snapshot(tenant: &str, snapshot: char) -> VerifiedE0QueryProjection {
        VerifiedE0QueryProjection::deterministic_for_e0_query_test(
            tenant,
            100,
            digest(snapshot),
            digest('f'),
            "case",
            digest('b'),
            digest('c'),
            digest('d'),
            digest('a'),
            digest('b'),
            BTreeMap::from([
                ("event_time".to_owned(), "1970-01-01T00:01:40Z".to_owned()),
                ("status".to_owned(), "1970-01-01T00:01:40Z".to_owned()),
            ]),
            vec![BTreeMap::from([
                ("event_time".to_owned(), "1970-01-01T00:01:40Z".to_owned()),
                ("status".to_owned(), "completed".to_owned()),
            ])],
        )
    }

    #[test]
    fn only_a_real_u04b_projection_can_bind_e0_table_field_and_replay_commitments() {
        let snapshot = SourceSnapshot::from_json(
            &json!({
                "contract_version": {"major": 1, "minor": 0},
                "tenant_id": "tenant_a", "source_namespace": "platform_history",
                "world_ref": "world_a", "observed_cutoff": "1970-01-01T00:01:40Z",
                "sources": [{"table":"case", "uri":"file://fixture.csv", "file_digest":digest('a'),
                    "header_digest":digest('b'), "row_count":1,
                    "source_contract_ref":{"id":"case", "version":"v1", "digest":digest('c')}}]
            })
            .to_string(),
        )
        .expect("fixed source snapshot");
        let row = json!({"event_time":"1970-01-01T00:01:40Z", "status":"completed"});
        let availability = vec![ReplayRowAvailability::new(BTreeMap::from([
            ("event_time".to_owned(), "1970-01-01T00:01:40Z".to_owned()),
            ("status".to_owned(), "1970-01-01T00:01:40Z".to_owned()),
        ]))];
        let profile = AvailabilityProfile::new(
            "e0_replay",
            1,
            AvailabilityClockMode::replay_at_event_time("e0_zero_lag"),
            "tenant_a",
            snapshot.binding_digest(),
        );
        let manifest = EnrichedHistoryManifest::new_replay(
            "platform_history",
            "world_a",
            "1970-01-01T00:01:40Z",
            profile,
            vec![
                PackageFile::new(
                    "case",
                    ProvenanceDigests::new(digest('a'), digest('b'), digest('c'), digest('d')),
                    "1970-01-01T00:01:40Z",
                )
                .with_field_availability(BTreeMap::from([
                    ("event_time".to_owned(), "1970-01-01T00:01:40Z".to_owned()),
                    ("status".to_owned(), "1970-01-01T00:01:40Z".to_owned()),
                ]))
                .with_replay_projection_digest(replay_projection_digest(
                    std::slice::from_ref(&row),
                    &availability,
                ))
                .with_source_file_seal(snapshot.source_file_seal("case").expect("case seal")),
            ],
        );
        let adapter = EnrichedHistoryAdapter::from_snapshot(manifest, &snapshot)
            .expect("bound U04-B adapter");
        let replay = adapter
            .verified_replay_availability(&snapshot)
            .expect("verified replay availability");
        let projection = adapter
            .verified_e0_query_projection(
                &snapshot,
                &replay,
                "case",
                TableInput::new(
                    ProvenanceDigests::new(digest('a'), digest('b'), digest('c'), digest('d')),
                    vec![row],
                )
                .with_replay_row_availability(availability),
            )
            .expect("verified E0 query projection");
        assert_eq!(projection.table(), "case");
        assert!(projection.allows_field("status"));
        assert!(!projection.allows_field("label"));
        assert_eq!(projection.cutoff_at_unix_seconds(), 100);

        // The U08 artifact digest names the content (`a`), whereas the U04
        // snapshot binding commits the entire persisted snapshot and therefore
        // differs. The explicit approved-source mapping bridges those domains.
        let artifact = ArtifactReference {
            tenant_id: "tenant_a".to_owned(),
            id: "018f50a1-7f00-7000-8000-000000000008".to_owned(),
            revision: 1,
            digest: digest('a'),
        };
        assert_ne!(artifact.digest, snapshot.binding_digest());
        let lab_rows = vec![BTreeMap::from([
            ("event_time".to_owned(), "1970-01-01T00:01:40Z".to_owned()),
            ("status".to_owned(), "completed".to_owned()),
        ])];
        assert!(projection.matches_lab_source(
            &snapshot.binding_digest(),
            "case",
            &["event_time".to_owned(), "status".to_owned()],
            &lab_rows,
        ));
        let approved = InMemoryLabSourceAuthority
            .approve(
                LabSource::new(
                    LabSourceManifest {
                        tenant_id: "tenant_a".to_owned(),
                        snapshot_ref: artifact.clone(),
                        source_contract_digest: digest('b'),
                        source_digest: digest('a'),
                        transform_digest: digest('c'),
                        cutoff_unix_seconds: 100,
                        classification: crate::local_lab::LabDataClassification::Treated,
                        safe_for_discovery: true,
                    },
                    vec![LabTable::new(
                        "case",
                        vec!["event_time", "status"],
                        lab_rows,
                    )],
                )
                .unwrap(),
            )
            .unwrap();
        let approved = projection.bind_approved_lab_source(approved);
        let access = LabAccess::new(
            "real_u04",
            "tenant_a",
            "investigation",
            "grant",
            "authority",
            artifact,
            1_000,
        );
        let mut authority = InMemoryLabGrantAuthority::default();
        authority.issue(LabGrant::from_access(&access));
        let mut lab = LocalInvestigationLab::new(authority);
        let session = lab.open(access.clone(), approved, 100).unwrap();
        let result = lab
            .query(
                session.session_id(),
                &access,
                LabQuery::select("case", vec!["event_time", "status"], None),
                100,
            )
            .unwrap();
        let candidate = lab
            .governed_e0_candidate(session.session_id(), &access, &result.receipt().digest, 100)
            .unwrap();
        if let Err(error) = E0QueryLab::admit(&projection, candidate) {
            panic!("{error:?}");
        }
    }

    fn source(
        tenant: &str,
        cutoff: u64,
        label_column: bool,
    ) -> (ApprovedLabSource, ArtifactReference) {
        source_with_snapshot(tenant, cutoff, label_column, 'e')
    }

    fn source_with_snapshot(
        tenant: &str,
        cutoff: u64,
        label_column: bool,
        u04_binding_byte: char,
    ) -> (ApprovedLabSource, ArtifactReference) {
        let snapshot = ArtifactReference {
            tenant_id: tenant.to_owned(),
            id: "018f50a1-7f00-7000-8000-000000000008".to_owned(),
            revision: 1,
            // Intentionally distinct from the U04 SourceSnapshot binding.
            digest: digest('a'),
        };
        let mut row = BTreeMap::from([
            ("event_time".to_owned(), "1970-01-01T00:01:40Z".to_owned()),
            ("status".to_owned(), "completed".to_owned()),
        ]);
        let columns = if label_column {
            row.insert("label".to_owned(), "future_answer".to_owned());
            vec!["event_time", "status", "label"]
        } else {
            vec!["event_time", "status"]
        };
        let source = LabSource::new(
            LabSourceManifest {
                tenant_id: tenant.to_owned(),
                snapshot_ref: snapshot.clone(),
                source_contract_digest: digest('b'),
                source_digest: digest('c'),
                transform_digest: digest('d'),
                cutoff_unix_seconds: cutoff,
                classification: crate::local_lab::LabDataClassification::Treated,
                safe_for_discovery: true,
            },
            vec![LabTable::new("case", columns, vec![row])],
        )
        .expect("fixed treated source");
        let approved = InMemoryLabSourceAuthority
            .approve(source)
            .expect("fixed source approval");
        let approved =
            projection_for_snapshot(tenant, u04_binding_byte).bind_approved_lab_source(approved);
        (approved, snapshot)
    }

    fn lab_for(
        tenant: &str,
        cutoff: u64,
        label_column: bool,
    ) -> (
        LocalInvestigationLab,
        LabAccess,
        crate::local_lab::LabSession,
    ) {
        let (approved, snapshot) = source(tenant, cutoff, label_column);
        let access = LabAccess::new(
            "run",
            tenant,
            "investigation",
            "grant",
            "authority",
            snapshot,
            1_000,
        );
        let mut authority = InMemoryLabGrantAuthority::default();
        authority.issue(LabGrant::from_access(&access));
        let mut lab = LocalInvestigationLab::new(authority);
        let session = lab
            .open(access.clone(), approved, cutoff)
            .expect("fixed E0 session");
        (lab, access, session)
    }

    fn lab_for_other_snapshot() -> (
        LocalInvestigationLab,
        LabAccess,
        crate::local_lab::LabSession,
    ) {
        let (approved, snapshot) = source_with_snapshot("tenant_a", 100, false, 'f');
        let access = LabAccess::new(
            "run",
            "tenant_a",
            "investigation",
            "grant",
            "authority",
            snapshot,
            1_000,
        );
        let mut authority = InMemoryLabGrantAuthority::default();
        authority.issue(LabGrant::from_access(&access));
        let mut lab = LocalInvestigationLab::new(authority);
        let session = lab.open(access.clone(), approved, 100).unwrap();
        (lab, access, session)
    }

    #[test]
    fn binds_a_completed_u08_read_only_receipt_to_exact_e0_commitments() {
        let (mut lab, access, session) = lab_for("tenant_a", 100, false);
        let result = lab
            .query(
                session.session_id(),
                &access,
                LabQuery::select("case", vec!["event_time", "status"], None),
                100,
            )
            .expect("governed U08 read");
        let candidate = lab
            .governed_e0_candidate(session.session_id(), &access, &result.receipt().digest, 100)
            .expect("ledger-attested U08 result");
        let verified =
            E0QueryLab::admit(&projection("tenant_a"), candidate).expect("bound E0 result");
        assert!(verified.result().receipt().has_valid_digest());
        assert!(!verified.authorizes_source_write_or_release());
    }

    #[test]
    fn tampered_rows_cross_tenant_future_cutoff_labels_and_later_joins_fail_closed() {
        let (mut lab, access, session) = lab_for("tenant_a", 100, false);
        let (mut future_lab, future_access, future_session) = lab_for("tenant_a", 101, false);
        let future = future_lab
            .query(
                future_session.session_id(),
                &future_access,
                LabQuery::select("case", vec!["event_time"], None),
                101,
            )
            .unwrap();
        let future_candidate = future_lab
            .governed_e0_candidate(
                future_session.session_id(),
                &future_access,
                &future.receipt().digest,
                101,
            )
            .unwrap();
        assert!(matches!(
            E0QueryLab::admit(&projection("tenant_a"), future_candidate),
            Err(E0QueryLabError::FutureOrCutoffDenied)
        ));

        let (mut label_lab, label_access, label_session) = lab_for("tenant_a", 100, true);
        let label = label_lab
            .query(
                label_session.session_id(),
                &label_access,
                LabQuery::select("case", vec!["label"], None),
                100,
            )
            .unwrap();
        let label_candidate = label_lab
            .governed_e0_candidate(
                label_session.session_id(),
                &label_access,
                &label.receipt().digest,
                100,
            )
            .unwrap();
        assert!(matches!(
            E0QueryLab::admit(&projection("tenant_a"), label_candidate),
            Err(E0QueryLabError::LabelOrUnknownFieldDenied)
        ));

        let label_filter = label_lab
            .query(
                label_session.session_id(),
                &label_access,
                LabQuery::select(
                    "case",
                    vec!["event_time"],
                    Some(crate::local_lab::QueryFilter::equals(
                        "label",
                        "future_answer",
                    )),
                ),
                100,
            )
            .unwrap();
        let label_filter_candidate = label_lab
            .governed_e0_candidate(
                label_session.session_id(),
                &label_access,
                &label_filter.receipt().digest,
                100,
            )
            .unwrap();
        assert!(matches!(
            E0QueryLab::admit(&projection("tenant_a"), label_filter_candidate),
            Err(E0QueryLabError::LabelOrUnknownFieldDenied)
        ));

        let divergent_source_read = label_lab
            .query(
                label_session.session_id(),
                &label_access,
                LabQuery::select("case", vec!["event_time"], None),
                100,
            )
            .unwrap();
        let divergent_candidate = label_lab
            .governed_e0_candidate(
                label_session.session_id(),
                &label_access,
                &divergent_source_read.receipt().digest,
                100,
            )
            .unwrap();
        assert!(matches!(
            E0QueryLab::admit(&projection("tenant_a"), divergent_candidate),
            Err(E0QueryLabError::ProjectionMismatch)
        ));

        let (mut other_lab, other_access, other_session) = lab_for("tenant_b", 100, false);
        let other = other_lab
            .query(
                other_session.session_id(),
                &other_access,
                LabQuery::select("case", vec!["event_time"], None),
                100,
            )
            .unwrap();
        let other_candidate = other_lab
            .governed_e0_candidate(
                other_session.session_id(),
                &other_access,
                &other.receipt().digest,
                100,
            )
            .unwrap();
        assert!(matches!(
            E0QueryLab::admit(&projection("tenant_a"), other_candidate),
            Err(E0QueryLabError::CrossTenantDenied)
        ));

        let (mut snapshot_lab, snapshot_access, snapshot_session) = lab_for_other_snapshot();
        let snapshot_result = snapshot_lab
            .query(
                snapshot_session.session_id(),
                &snapshot_access,
                LabQuery::select("case", vec!["event_time"], None),
                100,
            )
            .unwrap();
        let snapshot_candidate = snapshot_lab
            .governed_e0_candidate(
                snapshot_session.session_id(),
                &snapshot_access,
                &snapshot_result.receipt().digest,
                100,
            )
            .unwrap();
        assert!(matches!(
            E0QueryLab::admit(&projection("tenant_a"), snapshot_candidate),
            Err(E0QueryLabError::ProjectionMismatch)
        ));

        let first = lab
            .query(
                session.session_id(),
                &access,
                LabQuery::select("case", vec!["status"], None),
                100,
            )
            .unwrap();
        let joined = lab
            .query(
                session.session_id(),
                &access,
                LabQuery::dependent_select(
                    "case",
                    vec!["event_time"],
                    "status",
                    first.receipt().digest.clone(),
                    "status",
                ),
                100,
            )
            .unwrap();
        let joined_candidate = lab
            .governed_e0_candidate(session.session_id(), &access, &joined.receipt().digest, 100)
            .unwrap();
        assert!(matches!(
            E0QueryLab::admit(&projection("tenant_a"), joined_candidate),
            Err(E0QueryLabError::LaterJoinDenied)
        ));
        assert_eq!(
            lab.query(
                session.session_id(),
                &access,
                LabQuery::source_write("case"),
                100
            ),
            Err(crate::local_lab::LabError::SourceWriteDenied)
        );
    }
}
