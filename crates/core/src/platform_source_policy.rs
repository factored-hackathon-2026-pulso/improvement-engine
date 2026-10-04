//! Fail-closed source projection policy for the additive `platform_live` profile.
//!
//! This module is deliberately independent of the platform exporter's wire
//! schema. It publishes a closed relation/field vocabulary for the future
//! adapter; it has no API that accepts SQL, an arbitrary table name, or a
//! credential relation. The adapter must map this plan to the platform's
//! versioned schema and reject a mapping it cannot satisfy exactly.

use std::fmt;

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::core_task::CoreTaskScope;
use crate::model_provider::{
    ModelPolicy, ModelProviderError, ProjectionBrokerPort, VerifiedProjection,
};

/// Relations intentionally exposed by the platform-live profile.
/// Credential tables are not representable by this type.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlatformRelation {
    Cases,
    Turns,
    Assignments,
    CustomerCaseSlots,
    Customers,
    EventLog,
    Staff,
}

/// Semantic fields the platform adapter may project. Names, emails, password
/// hashes, MFA challenges, session material, and arbitrary JSON are absent.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlatformColumn {
    CaseId,
    CustomerId,
    StaffId,
    Channel,
    Priority,
    CreatedAt,
    SlaDueAt,
    TurnSequence,
    TurnBody,
    AssignmentTime,
    Slot,
    SimulatorFlag,
    EventSequence,
    EventType,
    EventTime,
    IngestedAt,
    EventPayloadLocalOnly,
    StaffRoles,
    StaffLanguages,
    StaffTeam,
    StaffActive,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RelationProjection {
    relation: PlatformRelation,
    columns: &'static [PlatformColumn],
}

impl RelationProjection {
    pub fn relation(&self) -> PlatformRelation {
        self.relation
    }

    pub fn columns(&self) -> &'static [PlatformColumn] {
        self.columns
    }
}

/// The only capability handed to a platform source adapter. It has no dynamic
/// table-name constructor and never includes credential relations.
pub trait PlatformSourceReader {
    type Error;

    fn read_relation(
        &self,
        relation: PlatformRelation,
        columns: &[PlatformColumn],
    ) -> Result<(), Self::Error>;
}

/// Caller-declared access mode used by the local read-plan guard.
///
/// This is not an authorization capability: callers can construct either
/// value, and the policy layer cannot prove the reader's underlying database
/// credentials or implementation behavior.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlatformSourceAccessMode {
    ReadOnly,
    ReadWrite,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlatformSourcePolicyError<E> {
    ReadWriteAccessDenied,
    Reader(E),
}

/// Immutable default projection for the published platform-live source
/// contract. `cases` intentionally omits mutable status and closure labels;
/// those are reconstructed as-of from the append-only event stream.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PlatformSourceReadPlan {
    event_catalog: PlatformEventCatalog,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PlatformSourcePolicyProvenance {
    pub event_catalog: PlatformEventCatalogProvenance,
}

const CASE_COLUMNS: &[PlatformColumn] = &[
    PlatformColumn::CaseId,
    PlatformColumn::CustomerId,
    PlatformColumn::Channel,
    PlatformColumn::Priority,
    PlatformColumn::CreatedAt,
    PlatformColumn::SlaDueAt,
];
const TURN_COLUMNS: &[PlatformColumn] = &[
    PlatformColumn::CaseId,
    PlatformColumn::TurnSequence,
    PlatformColumn::TurnBody,
];
const ASSIGNMENT_COLUMNS: &[PlatformColumn] = &[
    PlatformColumn::CaseId,
    PlatformColumn::StaffId,
    PlatformColumn::AssignmentTime,
];
const SLOT_COLUMNS: &[PlatformColumn] = &[PlatformColumn::CaseId, PlatformColumn::CustomerId];
const CUSTOMER_COLUMNS: &[PlatformColumn] =
    &[PlatformColumn::CustomerId, PlatformColumn::SimulatorFlag];
const EVENT_COLUMNS: &[PlatformColumn] = &[
    PlatformColumn::EventSequence,
    PlatformColumn::EventType,
    PlatformColumn::EventTime,
    PlatformColumn::IngestedAt,
    PlatformColumn::EventPayloadLocalOnly,
];
const STAFF_COLUMNS: &[PlatformColumn] = &[
    PlatformColumn::StaffId,
    PlatformColumn::StaffRoles,
    PlatformColumn::StaffLanguages,
    PlatformColumn::StaffTeam,
    PlatformColumn::StaffActive,
];
const PLATFORM_LIVE_PROJECTIONS: &[RelationProjection] = &[
    RelationProjection {
        relation: PlatformRelation::Cases,
        columns: CASE_COLUMNS,
    },
    RelationProjection {
        relation: PlatformRelation::Turns,
        columns: TURN_COLUMNS,
    },
    RelationProjection {
        relation: PlatformRelation::Assignments,
        columns: ASSIGNMENT_COLUMNS,
    },
    RelationProjection {
        relation: PlatformRelation::CustomerCaseSlots,
        columns: SLOT_COLUMNS,
    },
    RelationProjection {
        relation: PlatformRelation::Customers,
        columns: CUSTOMER_COLUMNS,
    },
    RelationProjection {
        relation: PlatformRelation::EventLog,
        columns: EVENT_COLUMNS,
    },
    RelationProjection {
        relation: PlatformRelation::Staff,
        columns: STAFF_COLUMNS,
    },
];

impl PlatformSourceReadPlan {
    pub fn platform_live() -> Self {
        Self::default()
    }

    pub fn with_event_catalog(event_catalog: PlatformEventCatalog) -> Self {
        Self { event_catalog }
    }

    pub fn provenance(&self) -> PlatformSourcePolicyProvenance {
        PlatformSourcePolicyProvenance {
            event_catalog: self.event_catalog.provenance(),
        }
    }

    pub fn event_catalog(&self) -> &PlatformEventCatalog {
        &self.event_catalog
    }

    pub fn projections(&self) -> &'static [RelationProjection] {
        PLATFORM_LIVE_PROJECTIONS
    }

    /// Execute the closed projection only when the caller declares read-only
    /// access. A read-write declaration is refused before calling the reader.
    /// This does not authenticate access, create a database snapshot, guarantee
    /// a consistent cut across relations, or sandbox the reader. Those remain
    /// requirements of a future concrete source adapter.
    pub fn execute_read_plan<R: PlatformSourceReader>(
        &self,
        access_mode: PlatformSourceAccessMode,
        reader: &R,
    ) -> Result<(), PlatformSourcePolicyError<R::Error>> {
        if access_mode != PlatformSourceAccessMode::ReadOnly {
            return Err(PlatformSourcePolicyError::ReadWriteAccessDenied);
        }

        for projection in self.projections() {
            reader
                .read_relation(projection.relation, projection.columns)
                .map_err(PlatformSourcePolicyError::Reader)?;
        }
        Ok(())
    }
}

/// Known business events in the platform-v1 profile. New events must be
/// explicitly reviewed rather than being accepted through a wildcard.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlatformBusinessEvent {
    CaseStatusChanged,
    CaseAssigned,
    CaseClosed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlatformEventDisposition {
    AcceptBusiness(PlatformBusinessEvent),
    AcceptAllowlistedAuth,
    QuarantineUnknown,
    DenyAuthNotAllowlisted,
}

/// Versioned by its exact, sorted allow-list. The default deliberately admits
/// no `auth.*` event. An integrator must supply an explicit reviewed list.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlatformEventCatalog {
    version: u32,
    allowed_auth_types: Vec<String>,
    digest: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PlatformEventCatalogProvenance {
    pub version: u32,
    pub digest: String,
}

impl Default for PlatformEventCatalog {
    fn default() -> Self {
        Self::with_auth_allowlist(1, Vec::new()).expect("the built-in empty event catalog is valid")
    }
}

impl PlatformEventCatalog {
    pub fn with_auth_allowlist(
        version: u32,
        mut allowed_auth_types: Vec<String>,
    ) -> Result<Self, PlatformEventCatalogError> {
        if version == 0 {
            return Err(PlatformEventCatalogError::InvalidVersion);
        }
        if allowed_auth_types
            .iter()
            .any(|event_type| !valid_auth_type(event_type))
        {
            return Err(PlatformEventCatalogError::InvalidAuthEventType);
        }
        allowed_auth_types.sort();
        allowed_auth_types.dedup();
        let digest = event_catalog_digest(version, &allowed_auth_types);
        Ok(Self {
            version,
            allowed_auth_types,
            digest,
        })
    }

    pub fn version(&self) -> u32 {
        self.version
    }

    pub fn digest(&self) -> &str {
        &self.digest
    }

    /// Copy this immutable reference into the platform source/run manifest so
    /// replay binds to the exact event catalog used for classification.
    pub fn provenance(&self) -> PlatformEventCatalogProvenance {
        PlatformEventCatalogProvenance {
            version: self.version,
            digest: self.digest.clone(),
        }
    }

    pub fn classify(&self, event_type: &str) -> PlatformEventDisposition {
        match event_type {
            "case.status_changed" => {
                PlatformEventDisposition::AcceptBusiness(PlatformBusinessEvent::CaseStatusChanged)
            }
            "case.assigned" => {
                PlatformEventDisposition::AcceptBusiness(PlatformBusinessEvent::CaseAssigned)
            }
            "case.closed" => {
                PlatformEventDisposition::AcceptBusiness(PlatformBusinessEvent::CaseClosed)
            }
            value if value.starts_with("auth.") => {
                if self
                    .allowed_auth_types
                    .iter()
                    .any(|allowed| allowed == value)
                {
                    PlatformEventDisposition::AcceptAllowlistedAuth
                } else {
                    PlatformEventDisposition::DenyAuthNotAllowlisted
                }
            }
            _ => PlatformEventDisposition::QuarantineUnknown,
        }
    }
}

fn event_catalog_digest(version: u32, allowed_auth_types: &[String]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"pulso-platform-event-catalog-v1\0");
    hasher.update(version.to_be_bytes());
    for event_type in ["case.status_changed", "case.assigned", "case.closed"]
        .into_iter()
        .chain(allowed_auth_types.iter().map(String::as_str))
    {
        let len = u32::try_from(event_type.len()).expect("event type length is bounded");
        hasher.update(len.to_be_bytes());
        hasher.update(event_type.as_bytes());
    }
    format!("sha256:{:x}", hasher.finalize())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlatformEventCatalogError {
    InvalidVersion,
    InvalidAuthEventType,
}

fn valid_auth_type(value: &str) -> bool {
    value
        .strip_prefix("auth.")
        .is_some_and(|suffix| !suffix.is_empty() && valid_event_suffix(suffix))
}

fn valid_event_suffix(value: &str) -> bool {
    value.split('.').all(|part| {
        !part.is_empty()
            && part
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
    })
}

/// Simulator-generated customers are not evidence for real-platform metrics,
/// even when their surrounding case rows are otherwise valid.
pub fn include_customer_in_population(simulator: bool) -> bool {
    !simulator
}

/// Free text stays local by default. This value intentionally has no serde
/// implementation or raw-text getter; the only model-egress path asks the
/// trusted projection broker to authorize/treat it before `ModelPort` use.
pub struct PlatformSourceText {
    value: String,
}

/// Provenance-bearing text emitted by the trusted source-treatment boundary.
/// Its constructor is crate-private so an ordinary caller cannot assert that
/// unreviewed source text is safe for model egress.
pub struct PlatformTreatedText {
    source_digest: String,
    output_digest: String,
    treatment_ref: String,
}

impl PlatformTreatedText {
    #[allow(dead_code)] // The live source-treatment authority is still being integrated.
    pub(crate) fn from_authoritative_treatment(
        _source: &PlatformSourceText,
        _value: String,
        _treatment_ref: impl Into<String>,
    ) -> Result<Self, PlatformTextTreatmentError> {
        // A caller-supplied provenance label is not evidence that text was
        // actually treated. Until a real authority verifies a receipt, no
        // treated capability may be minted inside the crate.
        Err(PlatformTextTreatmentError::AuthorityUnavailable)
    }

    pub fn source_digest(&self) -> &str {
        &self.source_digest
    }

    pub fn output_digest(&self) -> &str {
        &self.output_digest
    }

    pub fn treatment_ref(&self) -> &str {
        &self.treatment_ref
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlatformTextTreatmentError {
    AuthorityUnavailable,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PlatformTextEgressError {
    TreatmentRequired,
    TreatmentSourceMismatch,
    TreatmentAuthorityUnavailable,
    LocalOnlyEventPayload,
    ModelProvider(ModelProviderError),
}

/// A customer-message field from the allow-listed `turns` relation.
pub struct PlatformTurnBody {
    value: String,
}

impl PlatformTurnBody {
    pub fn from_source(value: impl Into<String>) -> Self {
        Self {
            value: value.into(),
        }
    }
}

/// An event's untrusted arbitrary JSON payload. It is local-only and deliberately
/// cannot be converted to [`PlatformTurnBody`] or [`PlatformSourceText`].
///
/// ```compile_fail
/// use improvement_engine_core::platform_source_policy::{
///     PlatformEventPayloadLocalOnly, PlatformSourceText, PlatformTurnBody,
/// };
/// let payload = PlatformEventPayloadLocalOnly::from_source("{\"secret\":true}");
/// let body = PlatformTurnBody::from_source(payload);
/// let _ = PlatformSourceText::turn_body(body);
/// ```
pub struct PlatformEventPayloadLocalOnly {
    _value: String,
}

impl PlatformEventPayloadLocalOnly {
    pub fn from_source(value: impl Into<String>) -> Self {
        Self {
            _value: value.into(),
        }
    }

    pub fn authorize_model_egress<B: ProjectionBrokerPort>(
        &self,
        _scope: &CoreTaskScope,
        _policy: &ModelPolicy,
        _broker: &mut B,
    ) -> Result<VerifiedProjection, PlatformTextEgressError> {
        Err(PlatformTextEgressError::LocalOnlyEventPayload)
    }
}

impl fmt::Debug for PlatformEventPayloadLocalOnly {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PlatformEventPayloadLocalOnly")
            .field("payload", &"<local-only/redacted>")
            .finish()
    }
}

impl PlatformSourceText {
    pub fn turn_body(value: PlatformTurnBody) -> Self {
        Self { value: value.value }
    }

    pub fn authorize_model_egress<B: ProjectionBrokerPort>(
        self,
        _treated: Option<PlatformTreatedText>,
        _scope: &CoreTaskScope,
        _policy: &ModelPolicy,
        _broker: &mut B,
    ) -> Result<VerifiedProjection, PlatformTextEgressError> {
        let _source_digest = text_digest(&self.value);
        let _ = self;
        // Do not trust an opaque-but-self-asserted receipt. Egress remains
        // blocked until an independently verifiable treatment authority is
        // integrated and bound to this source value.
        Err(PlatformTextEgressError::TreatmentAuthorityUnavailable)
    }
}

fn text_digest(value: &str) -> String {
    format!("sha256:{:x}", Sha256::digest(value.as_bytes()))
}

impl fmt::Debug for PlatformSourceText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PlatformSourceText")
            .field("value", &"<local-only/redacted>")
            .finish()
    }
}

#[cfg(test)]
mod treatment_tests {
    use super::{
        PlatformSourceText, PlatformTextEgressError, PlatformTextTreatmentError,
        PlatformTreatedText, PlatformTurnBody, text_digest,
    };
    use crate::core_task::CoreTaskScope;
    use crate::model_provider::{
        ModelBudgetLimits, ModelCapability, ModelPolicy, ModelProvider, ModelProviderError,
        ProjectionBrokerPort, RedactionPolicy, VerifiedProjection,
    };

    #[derive(Default)]
    struct RecordingBroker(usize);

    impl ProjectionBrokerPort for RecordingBroker {
        fn authorize_projection(
            &mut self,
            _scope: &CoreTaskScope,
            _policy: &ModelPolicy,
            _treated_input: String,
        ) -> Result<VerifiedProjection, ModelProviderError> {
            self.0 += 1;
            Err(ModelProviderError::InvalidProjectionAuthorization)
        }
    }

    #[test]
    fn treatment_receipt_cannot_be_minted_without_a_verifier() {
        let source = PlatformSourceText::turn_body(PlatformTurnBody::from_source(
            "CUS-123 reported a payment issue",
        ));
        assert!(matches!(
            PlatformTreatedText::from_authoritative_treatment(
                &source,
                "reported a payment issue".into(),
                "pii-treatment-v3",
            ),
            Err(PlatformTextTreatmentError::AuthorityUnavailable)
        ));
    }

    #[test]
    fn realistic_pii_cannot_be_marked_treated_without_an_authority() {
        let source = PlatformSourceText::turn_body(PlatformTurnBody::from_source(
            "My name is Maria Perez, email maria.perez@example.com, phone +57 310 555 0198",
        ));

        assert!(matches!(
            PlatformTreatedText::from_authoritative_treatment(
                &source,
                "My name is Maria Perez, email maria.perez@example.com, phone +57 310 555 0198"
                    .into(),
                "self-asserted-treatment",
            ),
            Err(PlatformTextTreatmentError::AuthorityUnavailable)
        ));
    }

    #[test]
    fn forged_receipt_is_rejected_before_broker_invocation() {
        let source = PlatformSourceText::turn_body(PlatformTurnBody::from_source(
            "Maria Perez: maria.perez@example.com, +57 310 555 0198",
        ));
        let forged = PlatformTreatedText {
            source_digest: text_digest("Maria Perez: maria.perez@example.com, +57 310 555 0198"),
            output_digest: text_digest("Maria Perez: maria.perez@example.com, +57 310 555 0198"),
            treatment_ref: "self-asserted-treatment".into(),
        };
        let mut broker = RecordingBroker::default();
        let error = source
            .authorize_model_egress(
                Some(forged),
                &CoreTaskScope::new("tenant_a", "job_a", "grant_a", "authority_a").unwrap(),
                &test_policy(),
                &mut broker,
            )
            .expect_err("a forged receipt cannot authorize egress");

        assert_eq!(
            error,
            PlatformTextEgressError::TreatmentAuthorityUnavailable
        );
        assert_eq!(broker.0, 0);
    }

    fn test_policy() -> ModelPolicy {
        ModelPolicy::with_budget(
            "policy_a",
            ModelCapability::new(
                ModelProvider::OpenRouter,
                "https://openrouter.ai/api/v1",
                "openai/gpt-4.1-mini",
                "secret://pulso/test",
                "test-v1",
            )
            .unwrap(),
            "platform_investigation",
            RedactionPolicy::TokenizeKnownMarkers,
            0,
            1_000,
            ModelBudgetLimits::new(2_000, 100, 1_000).unwrap(),
        )
        .unwrap()
    }
}
