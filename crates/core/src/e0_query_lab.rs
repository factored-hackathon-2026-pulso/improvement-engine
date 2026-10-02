//! U08-E adapter for read-only E0 query results.
//!
//! It does not open source files, execute arbitrary SQL, write the source,
//! join a later result, inspect labels, call a model, or create a Scout draft.
//! U04-B owns replay availability; U08 owns the governed local-lab receipt.

use crate::enriched_history::VerifiedE0QueryProjection;
use crate::local_lab::{E0ReplayReceiptBinding, QueryResult};

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
    AlreadyBoundDenied,
}

impl E0QueryLab {
    /// Trusted composition takes a completed U08 query and binds it to an
    /// opaque U04-B E0 projection. Every check happens before re-signing the
    /// receipt, so a rejected result has no E0 receipt/capability.
    #[allow(dead_code)] // Invoked by the future E0 run composition root.
    pub(crate) fn admit(
        projection: &VerifiedE0QueryProjection,
        result: QueryResult,
    ) -> Result<VerifiedE0QueryResult, E0QueryLabError> {
        let (rows, receipt) = result.into_parts();
        if !receipt.has_valid_digest()
            || !receipt.binds_rows(&rows)
            || receipt.row_count != rows.len()
        {
            return Err(E0QueryLabError::ReceiptInvalid);
        }
        if receipt.e0_replay.is_some() {
            return Err(E0QueryLabError::AlreadyBoundDenied);
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
            .queried_columns
            .iter()
            .any(|field| !projection.allows_field(field))
        {
            return Err(E0QueryLabError::LabelOrUnknownFieldDenied);
        }
        let receipt = receipt.bind_e0_replay(E0ReplayReceiptBinding {
            source_snapshot_digest: projection.source_snapshot_digest().to_owned(),
            availability_profile_digest: projection.availability_profile_digest().to_owned(),
            field_commitment: projection.field_commitment().to_owned(),
            replay_projection_digest: projection.replay_projection_digest().to_owned(),
        });
        Ok(VerifiedE0QueryResult {
            result: QueryResult::untrusted(rows, receipt),
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
        VerifiedE0QueryProjection::deterministic_for_e0_query_test(
            tenant,
            100,
            digest('e'),
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
        assert_eq!(projection.cutoff_at_unix_seconds(), 100);
    }

    fn source(
        tenant: &str,
        cutoff: u64,
        label_column: bool,
    ) -> (ApprovedLabSource, ArtifactReference) {
        let snapshot = ArtifactReference {
            tenant_id: tenant.to_owned(),
            id: "018f50a1-7f00-7000-8000-000000000008".to_owned(),
            revision: 1,
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
        let verified = E0QueryLab::admit(&projection("tenant_a"), result).expect("bound E0 result");
        let binding = verified
            .result()
            .receipt()
            .e0_replay
            .as_ref()
            .expect("E0 binding");
        assert_eq!(binding.source_snapshot_digest, digest('e'));
        assert_eq!(binding.field_commitment, digest('a'));
        assert_eq!(binding.replay_projection_digest, digest('b'));
        assert!(verified.result().receipt().has_valid_digest());
        assert!(!verified.authorizes_source_write_or_release());
    }

    #[test]
    fn tampered_rows_cross_tenant_future_cutoff_labels_and_later_joins_fail_closed() {
        let (mut lab, access, session) = lab_for("tenant_a", 100, false);
        let result = lab
            .query(
                session.session_id(),
                &access,
                LabQuery::select("case", vec!["event_time", "status"], None),
                100,
            )
            .unwrap();
        let (mut rows, receipt) = result.into_parts();
        rows[0].insert("status".to_owned(), "tampered".to_owned());
        assert!(matches!(
            E0QueryLab::admit(
                &projection("tenant_a"),
                QueryResult::untrusted(rows, receipt)
            ),
            Err(E0QueryLabError::ReceiptInvalid)
        ));

        let (mut future_lab, future_access, future_session) = lab_for("tenant_a", 101, false);
        let future = future_lab
            .query(
                future_session.session_id(),
                &future_access,
                LabQuery::select("case", vec!["event_time"], None),
                101,
            )
            .unwrap();
        assert!(matches!(
            E0QueryLab::admit(&projection("tenant_a"), future),
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
        assert!(matches!(
            E0QueryLab::admit(&projection("tenant_a"), label),
            Err(E0QueryLabError::LabelOrUnknownFieldDenied)
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
        assert!(matches!(
            E0QueryLab::admit(&projection("tenant_a"), other),
            Err(E0QueryLabError::CrossTenantDenied)
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
        assert!(matches!(
            E0QueryLab::admit(&projection("tenant_a"), joined),
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
