//! U23-P temporal boundary for governed published-memory use.
//!
//! This is deliberately only the temporal protocol, not the U23 E0 runner.
//! It receives no wiki bytes, does not publish a revision, and cannot score or
//! release a candidate. U22/U33 remain the authority for a governed receipt.

use crate::ArtifactRepository;
use crate::governed_memory_use::{
    MemoryUseAdmission, MemoryUseAdmissionError, MemoryUseRequest, VerifiedMemoryUse,
};
use crate::memory_store::{MemoryPublisher, MemoryScope, MemoryUseReceiptAttestationPort};
use crate::wiki_scratch::WikiAuthorizationPort;

/// The replay cutoff already established by the U04-B availability boundary.
///
/// The caller obtains this only after U04-B has validated its sealed source
/// snapshot and availability projection. This small contract never reopens
/// source files or interprets outcome data.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReplayMemoryClock {
    replay_cutoff_at_unix_seconds: u64,
}

impl ReplayMemoryClock {
    #[must_use]
    pub fn new(replay_cutoff_at_unix_seconds: u64) -> Self {
        Self {
            replay_cutoff_at_unix_seconds,
        }
    }

    #[must_use]
    pub fn replay_cutoff_at_unix_seconds(&self) -> u64 {
        self.replay_cutoff_at_unix_seconds
    }
}

/// Treated timing evidence for one governed-memory admission.
///
/// `outcome_available_at_unix_seconds` proves only availability. It carries no
/// outcome value, label, memory page, or cache handle.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TemporalUseTiming {
    memory_use_at_unix_seconds: u64,
    outcome_available_at_unix_seconds: Option<u64>,
}

impl TemporalUseTiming {
    #[must_use]
    pub fn training(memory_use_at_unix_seconds: u64) -> Self {
        Self {
            memory_use_at_unix_seconds,
            outcome_available_at_unix_seconds: None,
        }
    }

    #[must_use]
    pub fn after_observed_outcome(
        memory_use_at_unix_seconds: u64,
        outcome_available_at_unix_seconds: u64,
    ) -> Self {
        Self {
            memory_use_at_unix_seconds,
            outcome_available_at_unix_seconds: Some(outcome_available_at_unix_seconds),
        }
    }

    #[must_use]
    pub fn memory_use_at_unix_seconds(&self) -> u64 {
        self.memory_use_at_unix_seconds
    }

    #[must_use]
    pub fn outcome_available_at_unix_seconds(&self) -> Option<u64> {
        self.outcome_available_at_unix_seconds
    }
}

/// The two allowed temporal semantics for a partitioned memory head.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemoryTemporalProtocol {
    Frozen,
    Continuous,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TemporalProtocolError {
    ProtocolMismatch {
        expected: &'static str,
        actual: String,
    },
    MemoryAfterReplayCutoff,
    AccessTimeMismatch,
    OutcomeForbiddenInFrozen,
    OutcomeRequired,
    OutcomeAfterMemoryUse,
    OutcomeAfterReplayCutoff,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TemporalMemoryAdmissionError {
    Temporal(TemporalProtocolError),
    Governed(MemoryUseAdmissionError),
}

/// Trusted composition entry point for the U23-P temporal gate.
///
/// It deliberately runs the pure temporal check before U22 records a receipt,
/// so a not-yet-observable outcome cannot create an audit/use side effect. The
/// underlying U22 call remains crate-private: an external caller cannot supply
/// a permissive publisher or receive a use capability from this boundary.
///
/// ```compile_fail
/// use improvement_engine_core::memory_temporal_protocol::MemoryTemporalAdmission;
/// let _ = MemoryTemporalAdmission::admit;
/// ```
///
/// ```compile_fail
/// use improvement_engine_core::memory_temporal_protocol::MemoryTemporalAdmission;
/// let _ = MemoryTemporalAdmission { _private: true };
/// ```
pub struct MemoryTemporalAdmission {
    _private: bool,
}

impl MemoryTemporalAdmission {
    #[allow(dead_code)] // Invoked by the future trusted service composition root.
    pub(crate) fn admit<
        R: ArtifactRepository,
        P: MemoryPublisher + MemoryUseReceiptAttestationPort,
        A: WikiAuthorizationPort,
    >(
        protocol: MemoryTemporalProtocol,
        replay: &ReplayMemoryClock,
        timing: TemporalUseTiming,
        publisher: &mut P,
        artifacts: &mut R,
        authority: &A,
        request: MemoryUseRequest,
    ) -> Result<VerifiedMemoryUse, TemporalMemoryAdmissionError> {
        if timing.memory_use_at_unix_seconds != request.allowed_at_unix_seconds() {
            return Err(TemporalMemoryAdmissionError::Temporal(
                TemporalProtocolError::AccessTimeMismatch,
            ));
        }
        protocol
            .validate(request.scope(), timing, replay)
            .map_err(TemporalMemoryAdmissionError::Temporal)?;
        MemoryUseAdmission::admit(publisher, artifacts, authority, request)
            .map_err(TemporalMemoryAdmissionError::Governed)
    }
}

impl MemoryTemporalProtocol {
    /// Validates only temporal eligibility. Receipt/authentication/liveness are
    /// deliberately revalidated by the U22/U33 composition boundary.
    pub fn validate(
        self,
        scope: &MemoryScope,
        timing: TemporalUseTiming,
        replay: &ReplayMemoryClock,
    ) -> Result<(), TemporalProtocolError> {
        let expected = match self {
            Self::Frozen => "frozen",
            Self::Continuous => "continuous",
        };
        if scope.protocol != expected {
            return Err(TemporalProtocolError::ProtocolMismatch {
                expected,
                actual: scope.protocol.clone(),
            });
        }
        if timing.memory_use_at_unix_seconds > replay.replay_cutoff_at_unix_seconds {
            return Err(TemporalProtocolError::MemoryAfterReplayCutoff);
        }
        match self {
            Self::Frozen if timing.outcome_available_at_unix_seconds.is_some() => {
                Err(TemporalProtocolError::OutcomeForbiddenInFrozen)
            }
            Self::Frozen => Ok(()),
            Self::Continuous => {
                let outcome = timing
                    .outcome_available_at_unix_seconds
                    .ok_or(TemporalProtocolError::OutcomeRequired)?;
                if outcome > timing.memory_use_at_unix_seconds {
                    return Err(TemporalProtocolError::OutcomeAfterMemoryUse);
                }
                if outcome > replay.replay_cutoff_at_unix_seconds {
                    return Err(TemporalProtocolError::OutcomeAfterReplayCutoff);
                }
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        MemoryTemporalAdmission, MemoryTemporalProtocol, ReplayMemoryClock,
        TemporalMemoryAdmissionError, TemporalProtocolError, TemporalUseTiming,
    };
    use crate::governed_memory_use::MemoryUseRequest;
    use crate::memory_store::{InMemoryMemoryRegistry, MemoryPublisher, MemoryScope};
    use crate::wiki_scratch::{
        InMemoryWikiGrantAuthority, MemoryScopeBinding, WikiAccess, WikiGrant,
    };
    use crate::{ArtifactDraft, ArtifactKind, ArtifactRepository, InMemoryArtifactRepository};
    use serde_json::json;

    const TENANT: &str = "tenant-a";
    const WIKI_ID: &str = "018f50a1-7f00-7000-8000-000000000023";

    fn scope(protocol: &str) -> MemoryScope {
        MemoryScope::new(
            TENANT,
            "investigation",
            "world-a",
            "campaign-a",
            protocol,
            "train",
        )
    }

    fn seeded(
        protocol: &str,
    ) -> (
        InMemoryArtifactRepository,
        InMemoryMemoryRegistry,
        InMemoryWikiGrantAuthority,
        WikiAccess,
    ) {
        let mut artifacts = InMemoryArtifactRepository::default();
        let snapshot = artifacts
            .append(
                None,
                ArtifactDraft::new(
                    TENANT,
                    WIKI_ID,
                    1,
                    ArtifactKind::MemoryWiki,
                    json!({
                        "available_at_unix_seconds": 100,
                        "purpose": "investigation",
                        "pages": {"index.md": "published"}
                    }),
                    None,
                ),
            )
            .expect("fixed memory wiki")
            .reference();
        let binding = MemoryScopeBinding::new("world-a", "campaign-a", protocol, "train");
        let access = WikiAccess::new_scoped(
            "run-2",
            TENANT,
            "investigation",
            "grant-2",
            snapshot.clone(),
            100,
            binding.clone(),
        );
        let mut authority = InMemoryWikiGrantAuthority::default();
        authority.issue(WikiGrant::new_scoped(
            "grant-2",
            "run-2",
            TENANT,
            "investigation",
            snapshot.clone(),
            binding,
        ));
        let mut registry = InMemoryMemoryRegistry::default();
        registry
            .seed_head(&mut artifacts, scope(protocol), snapshot)
            .expect("fixed memory head");
        (artifacts, registry, authority, access)
    }

    #[test]
    fn continuous_admission_rejects_an_outcome_not_yet_available_without_recording_a_receipt() {
        let (mut artifacts, mut registry, authority, access) = seeded("continuous");

        let result = MemoryTemporalAdmission::admit(
            MemoryTemporalProtocol::Continuous,
            &ReplayMemoryClock::new(100),
            TemporalUseTiming::after_observed_outcome(100, 101),
            &mut registry,
            &mut artifacts,
            &authority,
            MemoryUseRequest::new(scope("continuous"), access),
        );

        match result {
            Err(TemporalMemoryAdmissionError::Temporal(
                TemporalProtocolError::OutcomeAfterMemoryUse,
            )) => {}
            Err(other) => panic!("expected temporal outcome denial, got {other:?}"),
            Ok(_) => panic!("future outcome must not mint a capability"),
        }
        assert!(registry.receipts().is_empty());
    }

    #[test]
    fn admission_binds_its_temporal_claim_to_the_u33_receipt_clock_before_recording() {
        let (mut artifacts, mut registry, authority, access) = seeded("frozen");

        let result = MemoryTemporalAdmission::admit(
            MemoryTemporalProtocol::Frozen,
            &ReplayMemoryClock::new(100),
            TemporalUseTiming::training(99),
            &mut registry,
            &mut artifacts,
            &authority,
            MemoryUseRequest::new(scope("frozen"), access),
        );

        match result {
            Err(TemporalMemoryAdmissionError::Temporal(
                TemporalProtocolError::AccessTimeMismatch,
            )) => {}
            Err(other) => panic!("expected receipt-clock mismatch, got {other:?}"),
            Ok(_) => panic!("unbound temporal timing must not mint a capability"),
        }
        assert!(registry.receipts().is_empty());
    }

    #[test]
    fn frozen_admission_mints_only_the_existing_u22_receipt_provenance() {
        let (mut artifacts, mut registry, authority, access) = seeded("frozen");

        let admitted = MemoryTemporalAdmission::admit(
            MemoryTemporalProtocol::Frozen,
            &ReplayMemoryClock::new(100),
            TemporalUseTiming::training(100),
            &mut registry,
            &mut artifacts,
            &authority,
            MemoryUseRequest::new(scope("frozen"), access),
        )
        .expect("timely U22/U33 frozen admission");

        assert_eq!(admitted.head_version(), 1);
        assert_eq!(admitted.run_id(), "run-2");
        assert_eq!(registry.receipts().len(), 1);
    }
}
