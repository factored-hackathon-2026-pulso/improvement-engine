//! Core vocabulary and immutable revision boundary for the Pulso improvement service.
//!
//! This crate deliberately does not execute Agent Core primitives or call model providers.

pub mod original_contact_projection;

use std::collections::BTreeMap;

use postgres::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

pub mod artifact_kind_flow;
pub mod authority;
pub mod autonomous_scout;
pub mod change_compiler;
pub mod core_task;
pub mod debug_console;
pub mod deterministic_sensor;
pub mod durable_jobs;
pub mod durable_run_events;
#[cfg(feature = "local-simulation")]
pub mod e0_builder_design;
#[cfg(feature = "local-simulation")]
pub mod e0_core_draft_binding;
pub mod e0_deterministic_sensor;
pub(crate) mod e0_frozen_memory_cycle;
pub mod e0_frozen_memory_publication;
pub mod e0_frozen_summary;
pub mod e0_frozen_verifier;
#[cfg(feature = "local-simulation")]
pub mod e0_investigation_plan;
#[cfg(feature = "local-simulation")]
pub mod e0_mechanism_resolution;
pub mod e0_opportunity_qualification;
#[cfg(feature = "local-simulation")]
pub mod e0_proposal_assembly;
pub mod e0_query_lab;
pub mod e0_safety_oracle;
pub mod enriched_history;
pub mod evaluation_plan;
pub mod facade_steps;
pub use facade_steps::{
    ArmBinding, ArmObservation, InfrastructureFailure, PairError, PairPlan, PairVerdict,
    PlatformColumn, PlatformRelation, PlatformSourceReadPlan, PlatformSourceText,
    PlatformTreatedText,
};
pub mod final_eligibility;
pub mod governed_memory_use;
pub mod governed_registry;
pub mod independent_verifier;
pub mod jev_decision;
pub mod local_lab;
#[cfg(feature = "local-simulation")]
pub mod local_simulation;
pub mod memory_store;
pub mod memory_temporal_protocol;
pub mod model_provider;
pub mod native_evaluation;
pub mod paired_scenario;
pub mod pipeline;
pub mod platform_discovery;
pub mod platform_observations;
pub mod platform_sensor;
pub mod platform_source_policy;
pub mod quota_grant;
pub mod replay_clock;
pub mod run_activity;
pub mod run_config;
pub mod run_fork;
pub(crate) mod run_timeline_v2;
pub mod sandbox;
pub mod scenario_factory;
pub mod signal_portfolio;
pub mod source_validation;
pub mod value_model;
pub mod wiki_scratch;
pub mod workflow_bridge;

pub mod detectors {
    pub mod family_01;
}

/// Stable identifier used by diagnostics and future service composition.
pub fn service_name() -> &'static str {
    "improvement-engine-core"
}

/// Kinds owned by Pulso's immutable artifact store.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactKind {
    Signal,
    Opportunity,
    Proposal,
    CapabilityBundle,
    ScenarioSet,
    Evaluation,
    MemoryWiki,
    Detector,
    RunConfig,
    SourceSnapshot,
}

impl ArtifactKind {
    fn from_storage(value: &str) -> Result<Self, RepositoryError> {
        match value {
            "signal" => Ok(Self::Signal),
            "opportunity" => Ok(Self::Opportunity),
            "proposal" => Ok(Self::Proposal),
            "capability_bundle" => Ok(Self::CapabilityBundle),
            "scenario_set" => Ok(Self::ScenarioSet),
            "evaluation" => Ok(Self::Evaluation),
            "memory_wiki" => Ok(Self::MemoryWiki),
            "detector" => Ok(Self::Detector),
            "run_config" => Ok(Self::RunConfig),
            "source_snapshot" => Ok(Self::SourceSnapshot),
            other => Err(RepositoryError::InvalidArtifactKind {
                kind: other.to_owned(),
            }),
        }
    }
}

/// Stable, tenant-scoped pointer to one immutable artifact revision.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ArtifactReference {
    pub tenant_id: String,
    pub id: String,
    pub revision: u64,
    pub digest: String,
}

/// Candidate immutable revision supplied to the repository boundary.
///
/// `digest` is `sha256:` plus the SHA-256 of the compact, deterministically
/// serialized [`DigestContent`]. It excludes the digest itself, identity and
/// revision so a content digest is stable across immutable revisions.
#[derive(Clone, Debug, PartialEq)]
pub struct ArtifactDraft {
    pub tenant_id: String,
    pub id: String,
    pub revision: u64,
    pub kind: ArtifactKind,
    pub payload: Value,
    pub source_snapshot_ref: Option<ArtifactReference>,
    pub digest: String,
}

impl ArtifactDraft {
    #[must_use]
    pub fn new(
        tenant_id: impl Into<String>,
        id: impl Into<String>,
        revision: u64,
        kind: ArtifactKind,
        payload: Value,
        source_snapshot_ref: Option<ArtifactReference>,
    ) -> Self {
        let tenant_id = tenant_id.into();
        let id = id.into();
        let digest = content_digest(&kind, &payload, source_snapshot_ref.as_ref());

        Self {
            tenant_id,
            id,
            revision,
            kind,
            payload,
            source_snapshot_ref,
            digest,
        }
    }

    #[must_use]
    pub fn reference(&self) -> ArtifactReference {
        ArtifactReference {
            tenant_id: self.tenant_id.clone(),
            id: self.id.clone(),
            revision: self.revision,
            digest: self.digest.clone(),
        }
    }

    fn verify_digest(&self) -> bool {
        self.digest == content_digest(&self.kind, &self.payload, self.source_snapshot_ref.as_ref())
    }
}

#[derive(Serialize)]
struct DigestContent<'a> {
    kind: &'a ArtifactKind,
    payload: &'a Value,
    source_snapshot_ref: &'a Option<ArtifactReference>,
}

fn content_digest(
    kind: &ArtifactKind,
    payload: &Value,
    source_snapshot_ref: Option<&ArtifactReference>,
) -> String {
    let source_snapshot_ref = source_snapshot_ref.cloned();
    let bytes = serde_json::to_vec(&DigestContent {
        kind,
        payload,
        source_snapshot_ref: &source_snapshot_ref,
    })
    .expect("serialization of supported artifact content cannot fail");
    format!("sha256:{:x}", Sha256::digest(bytes))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RepositoryError {
    InvalidTenant,
    InvalidArtifactId {
        artifact_id: String,
    },
    InvalidRevision {
        revision: u64,
    },
    InvalidArtifactKind {
        kind: String,
    },
    DigestMismatch {
        artifact_id: String,
    },
    RevisionConflict {
        artifact_id: String,
        expected_head: Option<u64>,
        actual_head: Option<u64>,
    },
    RevisionNumberMismatch {
        artifact_id: String,
        expected: u64,
        actual: u64,
    },
    CrossTenantReference {
        artifact_id: String,
    },
    SourceSnapshotNotFound {
        artifact_id: String,
    },
    SourceSnapshotReferenceMismatch {
        artifact_id: String,
    },
    SourceSnapshotKindMismatch {
        artifact_id: String,
    },
    Storage {
        message: String,
    },
}

/// Persistence port for tenant-scoped, append-only Pulso artifact revisions.
pub trait ArtifactRepository {
    /// Appends only the next revision when the observed head still equals `expected_head`.
    fn append(
        &mut self,
        expected_head: Option<u64>,
        draft: ArtifactDraft,
    ) -> Result<ArtifactDraft, RepositoryError>;

    /// Retrieves only a revision belonging to the caller's tenant.
    fn get(
        &mut self,
        tenant_id: &str,
        artifact_id: &str,
        revision: u64,
    ) -> Result<Option<ArtifactDraft>, RepositoryError>;
}

/// Deterministic adapter used by unit tests and local, dependency-free development.
/// It has the same boundary checks as the future PostgreSQL adapter, but does not
/// claim PostgreSQL transaction or concurrent-process behavior.
#[derive(Default)]
pub struct InMemoryArtifactRepository {
    revisions: BTreeMap<(String, String, u64), ArtifactDraft>,
    heads: BTreeMap<(String, String), u64>,
}

impl ArtifactRepository for InMemoryArtifactRepository {
    fn append(
        &mut self,
        expected_head: Option<u64>,
        draft: ArtifactDraft,
    ) -> Result<ArtifactDraft, RepositoryError> {
        validate_draft(&draft)?;
        validate_source_snapshot_reference(&self.revisions, &draft)?;

        let head_key = (draft.tenant_id.clone(), draft.id.clone());
        let actual_head = self.heads.get(&head_key).copied();
        if expected_head != actual_head {
            return Err(RepositoryError::RevisionConflict {
                artifact_id: draft.id,
                expected_head,
                actual_head,
            });
        }

        let next_revision = match actual_head {
            None => 1,
            Some(head) => head
                .checked_add(1)
                .ok_or(RepositoryError::InvalidRevision { revision: u64::MAX })?,
        };
        if draft.revision != next_revision {
            return Err(RepositoryError::RevisionNumberMismatch {
                artifact_id: draft.id,
                expected: next_revision,
                actual: draft.revision,
            });
        }

        self.revisions.insert(
            (draft.tenant_id.clone(), draft.id.clone(), draft.revision),
            draft.clone(),
        );
        self.heads.insert(head_key, draft.revision);
        Ok(draft)
    }

    fn get(
        &mut self,
        tenant_id: &str,
        artifact_id: &str,
        revision: u64,
    ) -> Result<Option<ArtifactDraft>, RepositoryError> {
        let result = self
            .revisions
            .get(&(tenant_id.to_owned(), artifact_id.to_owned(), revision))
            .cloned();
        if let Some(ref draft) = result {
            validate_draft(draft)?;
            validate_source_snapshot_reference(&self.revisions, draft)?;
        }
        Ok(result)
    }
}

/// PostgreSQL adapter for the durable U02 boundary. It delegates append/CAS and
/// source-snapshot checks to `pulso_append_artifact_revision` in the migration.
pub struct PostgresArtifactRepository {
    client: Client,
}

impl PostgresArtifactRepository {
    #[must_use]
    pub fn new(client: Client) -> Self {
        Self { client }
    }

    /// Returns the connection after this adapter is no longer used.
    ///
    /// Composition roots use this to enter another explicit persistence boundary;
    /// callers must not retain concurrent mutable owners of one connection.
    #[must_use]
    pub fn into_inner(self) -> Client {
        self.client
    }
}

impl ArtifactRepository for PostgresArtifactRepository {
    fn append(
        &mut self,
        expected_head: Option<u64>,
        draft: ArtifactDraft,
    ) -> Result<ArtifactDraft, RepositoryError> {
        validate_draft(&draft)?;
        let expected_head = expected_head
            .map(i64::try_from)
            .transpose()
            .map_err(|_| RepositoryError::InvalidRevision { revision: u64::MAX })?;
        let revision =
            i64::try_from(draft.revision).map_err(|_| RepositoryError::InvalidRevision {
                revision: draft.revision,
            })?;
        let source = draft.source_snapshot_ref.as_ref();
        let source_tenant = source.map(|reference| reference.tenant_id.as_str());
        let source_id = source.map(|reference| reference.id.as_str());
        let source_revision = source
            .map(|reference| i64::try_from(reference.revision))
            .transpose()
            .map_err(|_| RepositoryError::InvalidRevision { revision: u64::MAX })?;
        let source_digest = source.map(|reference| reference.digest.as_str());

        self.client
            .query_one(
                "SELECT public.pulso_append_artifact_revision(
                    $1, $2::text::uuid, $3, $4, $5, $6, $7::jsonb, $8, $9::text::uuid, $10, $11
                )",
                &[
                    &draft.tenant_id,
                    &draft.id,
                    &expected_head,
                    &revision,
                    &database_kind(&draft.kind),
                    &draft.digest,
                    &draft.payload,
                    &source_tenant,
                    &source_id,
                    &source_revision,
                    &source_digest,
                ],
            )
            .map_err(storage_error)?;
        self.get(&draft.tenant_id, &draft.id, draft.revision)?
            .ok_or_else(|| RepositoryError::Storage {
                message: "append succeeded but the immutable revision was not readable".to_owned(),
            })
    }

    fn get(
        &mut self,
        tenant_id: &str,
        artifact_id: &str,
        revision: u64,
    ) -> Result<Option<ArtifactDraft>, RepositoryError> {
        let revision = i64::try_from(revision)
            .map_err(|_| RepositoryError::InvalidRevision { revision: u64::MAX })?;
        let row = self
            .client
            .query_opt(
                "SELECT * FROM public.pulso_get_artifact_revision($1, $2::text::uuid, $3)",
                &[&tenant_id, &artifact_id, &revision],
            )
            .map_err(storage_error)?;
        let Some(row) = row else {
            return Ok(None);
        };
        let source_tenant: Option<String> = row.get(6);
        let source_id: Option<String> = row.get(7);
        let source_revision: Option<i64> = row.get(8);
        let source_digest: Option<String> = row.get(9);
        let source_snapshot_ref = match (source_tenant, source_id, source_revision, source_digest) {
            (None, None, None, None) => None,
            (Some(tenant_id), Some(id), Some(revision), Some(digest)) if revision > 0 => {
                Some(ArtifactReference {
                    tenant_id,
                    id,
                    revision: revision as u64,
                    digest,
                })
            }
            _ => {
                return Err(RepositoryError::SourceSnapshotReferenceMismatch {
                    artifact_id: row.get(1),
                });
            }
        };
        let draft = ArtifactDraft {
            tenant_id: row.get(0),
            id: row.get(1),
            revision: row.get::<_, i64>(2) as u64,
            kind: ArtifactKind::from_storage(row.get::<_, String>(3).as_str())?,
            digest: row.get(4),
            payload: row.get(5),
            source_snapshot_ref,
        };
        validate_draft(&draft)?;
        if let Some(reference) = &draft.source_snapshot_ref {
            if reference.tenant_id != draft.tenant_id {
                return Err(RepositoryError::CrossTenantReference {
                    artifact_id: draft.id,
                });
            }
            let target_kind: Option<String> = row.get(10);
            let target_digest: Option<String> = row.get(11);
            match (target_kind.as_deref(), target_digest.as_deref()) {
                (None, _) => {
                    return Err(RepositoryError::SourceSnapshotNotFound {
                        artifact_id: draft.id,
                    });
                }
                (Some("source_snapshot"), Some(digest)) if digest == reference.digest => {}
                (Some("source_snapshot"), _) => {
                    return Err(RepositoryError::SourceSnapshotReferenceMismatch {
                        artifact_id: draft.id,
                    });
                }
                _ => {
                    return Err(RepositoryError::SourceSnapshotKindMismatch {
                        artifact_id: draft.id,
                    });
                }
            }
        }
        Ok(Some(draft))
    }
}

fn database_kind(kind: &ArtifactKind) -> &'static str {
    match kind {
        ArtifactKind::Signal => "signal",
        ArtifactKind::Opportunity => "opportunity",
        ArtifactKind::Proposal => "proposal",
        ArtifactKind::CapabilityBundle => "capability_bundle",
        ArtifactKind::ScenarioSet => "scenario_set",
        ArtifactKind::Evaluation => "evaluation",
        ArtifactKind::MemoryWiki => "memory_wiki",
        ArtifactKind::Detector => "detector",
        ArtifactKind::RunConfig => "run_config",
        ArtifactKind::SourceSnapshot => "source_snapshot",
    }
}

fn storage_error(error: postgres::Error) -> RepositoryError {
    RepositoryError::Storage {
        message: error.to_string(),
    }
}

fn validate_draft(draft: &ArtifactDraft) -> Result<(), RepositoryError> {
    if draft.tenant_id.is_empty() {
        return Err(RepositoryError::InvalidTenant);
    }
    if !is_uuid_v7(&draft.id) {
        return Err(RepositoryError::InvalidArtifactId {
            artifact_id: draft.id.clone(),
        });
    }
    if draft.revision == 0 || draft.revision > i64::MAX as u64 {
        return Err(RepositoryError::InvalidRevision {
            revision: draft.revision,
        });
    }
    if !draft.verify_digest() {
        return Err(RepositoryError::DigestMismatch {
            artifact_id: draft.id.clone(),
        });
    }
    Ok(())
}

fn validate_source_snapshot_reference(
    revisions: &BTreeMap<(String, String, u64), ArtifactDraft>,
    draft: &ArtifactDraft,
) -> Result<(), RepositoryError> {
    let Some(reference) = &draft.source_snapshot_ref else {
        return Ok(());
    };
    if reference.tenant_id != draft.tenant_id {
        return Err(RepositoryError::CrossTenantReference {
            artifact_id: draft.id.clone(),
        });
    }
    if reference.revision == 0 || !is_uuid_v7(&reference.id) || !is_sha256_digest(&reference.digest)
    {
        return Err(RepositoryError::SourceSnapshotReferenceMismatch {
            artifact_id: draft.id.clone(),
        });
    }
    let Some(target) = revisions.get(&(
        reference.tenant_id.clone(),
        reference.id.clone(),
        reference.revision,
    )) else {
        return Err(RepositoryError::SourceSnapshotNotFound {
            artifact_id: draft.id.clone(),
        });
    };
    if target.digest != reference.digest {
        return Err(RepositoryError::SourceSnapshotReferenceMismatch {
            artifact_id: draft.id.clone(),
        });
    }
    if target.kind != ArtifactKind::SourceSnapshot {
        return Err(RepositoryError::SourceSnapshotKindMismatch {
            artifact_id: draft.id.clone(),
        });
    }
    Ok(())
}

fn is_uuid_v7(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 36
        && [8, 13, 18, 23]
            .into_iter()
            .all(|index| bytes[index] == b'-')
        && bytes[14] == b'7'
        && matches!(bytes[19], b'8' | b'9' | b'a' | b'b')
        && bytes.iter().enumerate().all(|(index, byte)| {
            [8, 13, 18, 23].contains(&index) || matches!(*byte, b'0'..=b'9' | b'a'..=b'f')
        })
}

fn is_sha256_digest(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value.as_bytes()[7..]
            .iter()
            .all(|byte| matches!(*byte, b'0'..=b'9' | b'a'..=b'f'))
}
