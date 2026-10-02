//! Explicit opt-in, local-only composition for exercising the current engine
//! capabilities end to end without a provider or Agent Core runtime.
//!
//! The run uses real U08/U12/U13/U14 code. U09/U10, proposal authoring and the
//! evaluation adapter are explicitly simulated. The resulting draft is
//! unverified and non-executable; no business improvement or release is
//! claimed. Production callers must not use this module as a native runtime.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::ArtifactReference;
use crate::autonomous_scout::{
    CandidateKind, InMemoryScoutCandidateRepository, NonProductionScoutInvocationAuthority,
    ScoutCandidateAdmissionAuthority, ScoutInvocationAuthority, ScoutResult,
    record_scout_discovery,
};
use crate::core_task::{
    CoreTaskBinding, CoreTaskBindingRegistry, CoreTaskInvocation, CoreTaskPort, CoreTaskScope,
    CoreTaskSimulator,
};
use crate::deterministic_sensor::{BooleanRateSpec, DeterministicSensor, DeterministicSignal};
use crate::independent_verifier::{
    IndependentEvidenceVerifierPort, IndependentVerificationInput,
    IndependentVerificationPortError, IndependentVerificationReceipt, IndependentVerifier,
    VerificationReport, VerificationStatus,
};
use crate::local_lab::{
    InMemoryLabGrantAuthority, InMemoryLabSourceAuthority, LabAccess, LabDataClassification,
    LabGrant, LabQuery, LabSource, LabSourceApprovalPort, LabSourceManifest, LabTable,
    LocalInvestigationLab, QueryFilter,
};
use crate::model_provider::{
    HmacProjectionBroker, ModelBudgetLimits, ModelCapability, ModelInvocation, ModelPolicy,
    ModelPort, ModelProvider, ModelProviderSimulator, ProjectionBrokerPort, RedactionPolicy,
};

const SIMULATION_VERSION: &str = "pulso-local-simulation-v1";
const AGENT_CORE_CONTRACT_VERSION: &str = "0.5.0";
const AGENT_CORE_CONTRACT_SHA: &str = "53e729d624c8284e906249df84c1a1df84cc8d40";
const MAX_INPUT_EVENTS: usize = 100_000;
const MAX_CASES: usize = 5_000;
const QUERY_BATCH_SIZE: usize = 100;
const DEFAULT_MIN_RECURRING_QUERY_CASES: u64 = 20;
const MIN_RECURRING_QUERY_CASES: u64 = 5;
const RECURRING_QUERY_POLICY: &str = "e0_recurring_copilot_query_support_v1";
const RECURRING_QUERY_POLICY_VERSION: u16 = 1;

/// Minimal, treated event projection passed from a local source adapter.
/// Identity, prompts, transcripts, customer values and evaluator labels have
/// no fields in this type. Codes are validated before they are used or emitted.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalObservedEvent {
    pub case_ordinal: u32,
    pub event_ordinal: u32,
    pub parent_event_ordinal: Option<u32>,
    pub event_time: String,
    pub event_kind: String,
    pub route_code: Option<String>,
    pub actor_layer: Option<String>,
    pub tool_code: Option<String>,
    pub technical_error: Option<bool>,
    pub approval: Option<String>,
    pub signal_code: Option<String>,
}

/// Opaque, already-treated signature of one Copilot query. The signature is
/// used only in memory to group recurrence and is never serialized or logged.
#[derive(Clone, Eq, PartialEq)]
pub struct LocalObservedQuery {
    case_ordinal: u32,
    opaque_signature: String,
    event_time: String,
}

impl LocalObservedQuery {
    #[must_use]
    pub fn new(
        case_ordinal: u32,
        opaque_signature: impl Into<String>,
        event_time: impl Into<String>,
    ) -> Self {
        Self {
            case_ordinal,
            opaque_signature: opaque_signature.into(),
            event_time: event_time.into(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LocalSourceKind {
    OriginalBank,
    E0,
}

/// Input that is safe to use for discovery. There is deliberately no field for
/// a holdout handle or any evaluator-only labels.
#[derive(Clone)]
pub struct LocalRunInput {
    metadata: LocalRunMetadata,
    case_ordinals: Vec<u32>,
    excluded_replay_cases: u64,
    events: Vec<LocalObservedEvent>,
    queries: Vec<LocalObservedQuery>,
    minimum_recurring_query_support: u64,
}

/// Immutable commitments and point-in-time boundary for one local run.
#[derive(Clone, Debug)]
pub struct LocalRunMetadata {
    run_id: String,
    tenant_id: String,
    source_kind: LocalSourceKind,
    manifest_digest: String,
    snapshot_ref: ArtifactReference,
    cutoff_unix_seconds: u64,
    observed_cutoff_rfc3339: String,
}

impl LocalRunMetadata {
    #[must_use]
    pub fn new(
        run_id: impl Into<String>,
        tenant_id: impl Into<String>,
        source_kind: LocalSourceKind,
        manifest_digest: impl Into<String>,
        snapshot_ref: ArtifactReference,
        cutoff_unix_seconds: u64,
        observed_cutoff_rfc3339: impl Into<String>,
    ) -> Self {
        Self {
            run_id: run_id.into(),
            tenant_id: tenant_id.into(),
            source_kind,
            manifest_digest: manifest_digest.into(),
            snapshot_ref,
            cutoff_unix_seconds,
            observed_cutoff_rfc3339: observed_cutoff_rfc3339.into(),
        }
    }
}

impl LocalRunInput {
    #[must_use]
    pub fn new(
        metadata: LocalRunMetadata,
        case_ordinals: Vec<u32>,
        excluded_replay_cases: u64,
        events: Vec<LocalObservedEvent>,
    ) -> Self {
        Self {
            metadata,
            case_ordinals,
            excluded_replay_cases,
            events,
            queries: Vec::new(),
            minimum_recurring_query_support: DEFAULT_MIN_RECURRING_QUERY_CASES,
        }
    }

    #[must_use]
    pub fn with_queries(mut self, queries: Vec<LocalObservedQuery>) -> Self {
        self.queries = queries;
        self
    }

    pub fn with_minimum_recurring_query_support(
        mut self,
        minimum_support: u64,
    ) -> Result<Self, LocalRunError> {
        if !(MIN_RECURRING_QUERY_CASES..=MAX_CASES as u64).contains(&minimum_support) {
            return Err(LocalRunError::InvalidInput);
        }
        self.minimum_recurring_query_support = minimum_support;
        Ok(self)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct RunEvent {
    pub sequence: u32,
    pub stage: String,
    pub status: String,
    pub detail: String,
    pub observed_cutoff_rfc3339: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SignalSummary {
    pub metric_id: String,
    pub detector_policy_id: String,
    pub detector_policy_version: u16,
    pub minimum_support: u64,
    pub numerator: u64,
    pub denominator: u64,
    pub missing: u64,
    pub coverage_basis_points: u16,
    pub pattern_ref: Option<String>,
    pub digest: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CandidateSummary {
    pub kind: String,
    pub digest: String,
    pub admission: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ImprovementDraft {
    pub status: String,
    pub execution_status: String,
    pub hypothesis: String,
    pub evidence: SignalSummary,
    pub proposed_artifact: serde_json::Value,
    pub digest: String,
    pub simulation_version: String,
    pub native_agent_core_status: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct EvaluationSummary {
    pub status: String,
    pub evaluator: String,
    pub checks_passed: u32,
    pub checks_total: u32,
    pub claims_business_improvement: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct LocalRunResult {
    pub run_id: String,
    pub tenant_id: String,
    pub source_kind: LocalSourceKind,
    pub manifest_digest: String,
    pub snapshot_ref: ArtifactReference,
    pub observed_cutoff_rfc3339: String,
    pub execution_mode: String,
    pub simulation_version: String,
    pub simulation_seed: String,
    pub determinism: String,
    pub terminal_status: String,
    pub formal_route: String,
    pub primary_signal_policy: String,
    pub discovery_case_count: u64,
    pub excluded_replay_case_count: u64,
    pub signal: Option<SignalSummary>,
    pub signals: Vec<SignalSummary>,
    pub candidates: Vec<CandidateSummary>,
    pub verification_status: Option<String>,
    pub proposal: Option<ImprovementDraft>,
    pub evaluation: Option<EvaluationSummary>,
    pub events: Vec<RunEvent>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LocalRunError {
    InvalidInput,
    InvalidEventProjection,
    TooManyEvents,
    TooManyCases,
    Lab(String),
    CoreTask(String),
    Model(String),
    Scout(String),
    Verification(String),
}

impl std::fmt::Display for LocalRunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidInput => f.write_str("local run identity or source provenance is invalid"),
            Self::InvalidEventProjection => f.write_str("safe event projection is invalid"),
            Self::TooManyEvents => f.write_str("local simulation event cap exceeded"),
            Self::TooManyCases => f.write_str("local query case cap exceeded"),
            Self::Lab(_) => f.write_str("local read-only investigation failed"),
            Self::CoreTask(_) => f.write_str("local Agent Core task simulation failed"),
            Self::Model(_) => f.write_str("local model-port simulation failed"),
            Self::Scout(_) => f.write_str("Scout discovery or admission failed"),
            Self::Verification(_) => f.write_str("independent verification simulation failed"),
        }
    }
}

impl std::error::Error for LocalRunError {}

/// Runs an explicitly simulated local path. No API in this function can read
/// evaluator-only data or dispatch a real model/Agent Core request.
pub fn run_local_simulation(input: LocalRunInput) -> Result<LocalRunResult, LocalRunError> {
    validate_input(&input)?;
    let mut events = Vec::new();
    record_event(
        &mut events,
        "run_started",
        "simulated",
        "local-only execution",
    );
    record_event(
        &mut events,
        "source_loaded",
        "complete",
        "complete allowlisted discovery-source manifest and snapshot committed",
    );

    if input.metadata.source_kind == LocalSourceKind::OriginalBank {
        record_event(
            &mut events,
            "detection",
            "unsupported_source",
            "no currently allowlisted original-bank detector matches this projection",
        );
        record_event(
            &mut events,
            "run_completed",
            "complete",
            "no signal was fabricated",
        );
        bind_observed_cutoff(&mut events, &input.metadata.observed_cutoff_rfc3339);
        let simulation_seed = derive_digest(&format!(
            "{SIMULATION_VERSION}:{}",
            input.metadata.manifest_digest
        ));
        return Ok(LocalRunResult {
            run_id: input.metadata.run_id,
            tenant_id: input.metadata.tenant_id,
            source_kind: input.metadata.source_kind,
            manifest_digest: input.metadata.manifest_digest,
            snapshot_ref: input.metadata.snapshot_ref,
            observed_cutoff_rfc3339: input.metadata.observed_cutoff_rfc3339,
            execution_mode: "local_simulation".into(),
            simulation_version: SIMULATION_VERSION.into(),
            simulation_seed,
            determinism: "deterministic_given_identical_run_input".into(),
            terminal_status: "unsupported_source".into(),
            formal_route: "do_nothing".into(),
            primary_signal_policy: "local_primary_signal_v1".into(),
            discovery_case_count: 0,
            excluded_replay_case_count: 0,
            signal: None,
            signals: Vec::new(),
            candidates: Vec::new(),
            verification_status: None,
            proposal: None,
            evaluation: None,
            events,
        });
    }

    let signals = measure_signals(&input)?;
    let primary_signal_index =
        select_primary_signal_index(&signals, input.minimum_recurring_query_support)
            .ok_or(LocalRunError::InvalidEventProjection)?;
    let signal = signals[primary_signal_index].clone();
    let summaries = signals
        .iter()
        .map(|signal| summarize_signal(&input, signal))
        .collect::<Vec<_>>();
    let summary = summaries[primary_signal_index].clone();
    record_event(
        &mut events,
        "detection",
        "complete",
        &if signal.metric_id == "e0_recurring_copilot_query_cases" {
            format!(
                "the leading opaque copilot query signature appears in {} of {} discovery cases",
                signal.numerator, signal.denominator
            )
        } else {
            format!(
                "{} observed cases with {} technical errors across {} measured cases",
                signal.denominator + signal.missing,
                signal.numerator,
                signal.denominator
            )
        },
    );

    let signal_qualifies = if signal.metric_id == "e0_recurring_copilot_query_cases" {
        signal.numerator >= input.minimum_recurring_query_support
    } else {
        signal.numerator > 0
    };
    if !signal_qualifies || signal.denominator == 0 {
        let no_opportunity_detail = if signal.metric_id == "e0_recurring_copilot_query_cases" {
            "no opaque Copilot-query signature met the versioned minimum distinct-case support; no candidate or proposal was generated"
        } else {
            "no positive observed technical-error evidence; no candidate or proposal was generated"
        };
        record_event(
            &mut events,
            "scout",
            "no_opportunity",
            no_opportunity_detail,
        );
        record_event(
            &mut events,
            "run_completed",
            "complete_no_opportunity",
            "descriptive evidence did not meet the detector's support policy; formal route remains do_nothing",
        );
        bind_observed_cutoff(&mut events, &input.metadata.observed_cutoff_rfc3339);
        let simulation_seed = derive_digest(&format!(
            "{SIMULATION_VERSION}:{}:{}:{}",
            input.metadata.manifest_digest, input.metadata.run_id, signal.digest
        ));
        return Ok(LocalRunResult {
            run_id: input.metadata.run_id,
            tenant_id: input.metadata.tenant_id,
            source_kind: input.metadata.source_kind,
            manifest_digest: input.metadata.manifest_digest,
            snapshot_ref: input.metadata.snapshot_ref,
            observed_cutoff_rfc3339: input.metadata.observed_cutoff_rfc3339,
            execution_mode: "local_simulation".into(),
            simulation_version: SIMULATION_VERSION.into(),
            simulation_seed,
            determinism: "deterministic_given_identical_run_input".into(),
            terminal_status: "complete_no_opportunity".into(),
            formal_route: "do_nothing".into(),
            primary_signal_policy: "local_primary_signal_v1".into(),
            discovery_case_count: input.case_ordinals.len() as u64,
            excluded_replay_case_count: input.excluded_replay_cases,
            signal: Some(summary),
            signals: summaries,
            candidates: Vec::new(),
            verification_status: None,
            proposal: None,
            evaluation: None,
            events,
        });
    }

    let (scope, core_receipt, model_receipt) = simulate_agent_core_and_model(&input, &signal)?;
    record_event(
        &mut events,
        "jev_or_agent_core_scout",
        "simulated",
        "U09 task receipt is locally scripted; native Agent Core was not called",
    );
    record_event(
        &mut events,
        "model_provider",
        "simulated",
        "U10 receipt is locally scripted; no provider request or credential lookup occurred",
    );

    let mut invocation_authority = NonProductionScoutInvocationAuthority;
    let expectation = invocation_authority
        .seal(&scope, &signal, &core_receipt, &model_receipt)
        .map_err(|_| LocalRunError::Scout("Scout invocation was not sealed".into()))?;
    let (scout_result, mut admission) = record_scout_discovery(
        InMemoryScoutCandidateRepository::default(),
        &scope,
        &expectation,
        &signal,
        &core_receipt,
        &model_receipt,
    )
    .map_err(|_| LocalRunError::Scout("Scout candidates could not be recorded".into()))?;
    let ScoutResult::Candidates(drafts) = scout_result else {
        return Err(LocalRunError::Scout(
            "local simulated dependencies did not complete".into(),
        ));
    };
    let mut candidate_summaries = Vec::with_capacity(drafts.len());
    let mut opportunity = None;
    for draft in &drafts {
        let admitted = admission
            .admit(&scope, draft)
            .map_err(|_| LocalRunError::Scout("candidate admission failed".into()))?;
        if draft.kind == CandidateKind::Opportunity {
            opportunity = Some(admitted);
        }
        candidate_summaries.push(CandidateSummary {
            kind: format!("{:?}", draft.kind).to_lowercase(),
            digest: draft.digest.clone(),
            admission: "recorded_and_rehydrated".into(),
        });
    }
    record_event(
        &mut events,
        "scout",
        "complete",
        &format!(
            "{} descriptive candidates were recorded and re-admitted",
            drafts.len()
        ),
    );
    let opportunity =
        opportunity.ok_or_else(|| LocalRunError::Scout("opportunity candidate missing".into()))?;

    let mut verifier = LocalUncertainVerifier;
    let report = IndependentVerifier::verify(&opportunity, &mut verifier).map_err(|_| {
        LocalRunError::Verification("uncertain verifier receipt was rejected".into())
    })?;
    let verification_status = verification_status_name(report.status()).to_owned();
    record_event(
        &mut events,
        "independent_verification",
        "simulated_uncertain",
        "local verifier has no independent business oracle; formal eligibility remains closed",
    );

    let improvement_draft = build_exploratory_draft(&input, &summary, &report);
    record_event(
        &mut events,
        "improvement_draft",
        "simulated_unverified",
        "data-derived, non-executable exploratory proposal; not an eligible release candidate",
    );
    record_event(
        &mut events,
        "agent_core_native",
        "dependency_unavailable",
        "native Agent Core artifact compilation, registration and execution are not connected",
    );

    let evaluation = evaluate_draft_shape(&improvement_draft);
    record_event(
        &mut events,
        "evaluation",
        "simulated",
        "structural draft checks only; no Agent Core execution or holdout labels were used",
    );
    record_event(
        &mut events,
        "run_completed",
        "complete_simulated",
        "formal route is do_nothing until independent evidence and native gates exist",
    );
    bind_observed_cutoff(&mut events, &input.metadata.observed_cutoff_rfc3339);

    let simulation_seed = derive_digest(&format!(
        "{SIMULATION_VERSION}:{}:{}:{}",
        input.metadata.manifest_digest, input.metadata.run_id, signal.digest
    ));
    Ok(LocalRunResult {
        run_id: input.metadata.run_id,
        tenant_id: input.metadata.tenant_id,
        source_kind: input.metadata.source_kind,
        manifest_digest: input.metadata.manifest_digest,
        snapshot_ref: input.metadata.snapshot_ref,
        observed_cutoff_rfc3339: input.metadata.observed_cutoff_rfc3339,
        execution_mode: "local_simulation".into(),
        simulation_version: SIMULATION_VERSION.into(),
        simulation_seed,
        determinism: "deterministic_given_identical_run_input".into(),
        terminal_status: "complete_simulated".into(),
        formal_route: "do_nothing".into(),
        primary_signal_policy: "local_primary_signal_v1".into(),
        discovery_case_count: input.case_ordinals.len() as u64,
        excluded_replay_case_count: input.excluded_replay_cases,
        signal: Some(summary),
        signals: summaries,
        candidates: candidate_summaries,
        verification_status: Some(verification_status),
        proposal: Some(improvement_draft),
        evaluation: Some(evaluation),
        events,
    })
}

fn validate_input(input: &LocalRunInput) -> Result<(), LocalRunError> {
    if !is_identifier(&input.metadata.run_id)
        || !is_identifier(&input.metadata.tenant_id)
        || input.metadata.snapshot_ref.tenant_id != input.metadata.tenant_id
        || input.metadata.snapshot_ref.revision == 0
        || !is_digest(&input.metadata.snapshot_ref.digest)
        || !is_digest(&input.metadata.manifest_digest)
        || input.metadata.cutoff_unix_seconds == 0
    {
        return Err(LocalRunError::InvalidInput);
    }
    if input.events.len() > MAX_INPUT_EVENTS {
        return Err(LocalRunError::TooManyEvents);
    }
    if input.case_ordinals.len() > MAX_CASES {
        return Err(LocalRunError::TooManyCases);
    }
    if input.case_ordinals.contains(&0)
        || input.case_ordinals.iter().collect::<BTreeSet<_>>().len() != input.case_ordinals.len()
    {
        return Err(LocalRunError::InvalidEventProjection);
    }
    let mut seen = BTreeSet::new();
    for event in &input.events {
        if event.case_ordinal == 0
            || event.event_ordinal == 0
            || event.parent_event_ordinal == Some(event.event_ordinal)
            || !valid_code(&event.event_kind)
            || event
                .route_code
                .iter()
                .chain(event.actor_layer.iter())
                .chain(event.tool_code.iter())
                .chain(event.approval.iter())
                .chain(event.signal_code.iter())
                .any(|value| !valid_code(value))
            || parse_utc_seconds(&event.event_time)
                .is_none_or(|event_time| event_time > input.metadata.cutoff_unix_seconds)
            || !seen.insert((event.case_ordinal, event.event_ordinal))
        {
            return Err(LocalRunError::InvalidEventProjection);
        }
    }
    if input
        .events
        .iter()
        .any(|event| !input.case_ordinals.contains(&event.case_ordinal))
    {
        return Err(LocalRunError::InvalidEventProjection);
    }
    for event in &input.events {
        if event.parent_event_ordinal.is_some_and(|parent| {
            !seen.contains(&(event.case_ordinal, parent)) || parent >= event.event_ordinal
        }) {
            return Err(LocalRunError::InvalidEventProjection);
        }
    }
    if input.queries.len() > MAX_INPUT_EVENTS
        || input.queries.iter().any(|query| {
            query.case_ordinal == 0
                || !input.case_ordinals.contains(&query.case_ordinal)
                || !is_opaque_query_signature(&query.opaque_signature)
                || parse_utc_seconds(&query.event_time)
                    .is_none_or(|time| time > input.metadata.cutoff_unix_seconds)
        })
    {
        return Err(LocalRunError::InvalidEventProjection);
    }
    Ok(())
}

fn measure_signals(input: &LocalRunInput) -> Result<Vec<DeterministicSignal>, LocalRunError> {
    let mut signals = vec![measure_signal(input, false)?];
    if !input.queries.is_empty() {
        signals.push(measure_signal(input, true)?);
    }
    Ok(signals)
}

fn measure_signal(
    input: &LocalRunInput,
    recurrence_metric: bool,
) -> Result<DeterministicSignal, LocalRunError> {
    let mut cases = input
        .case_ordinals
        .iter()
        .map(|ordinal| (*ordinal, Vec::new()))
        .collect::<BTreeMap<u32, Vec<&LocalObservedEvent>>>();
    for event in &input.events {
        cases
            .get_mut(&event.case_ordinal)
            .expect("validated case")
            .push(event);
    }
    if cases.is_empty() {
        return Err(LocalRunError::InvalidEventProjection);
    }
    if cases.len() > MAX_CASES {
        return Err(LocalRunError::TooManyCases);
    }

    let mut support_by_signature = BTreeMap::<String, BTreeSet<u32>>::new();
    for query in &input.queries {
        support_by_signature
            .entry(query.opaque_signature.clone())
            .or_default()
            .insert(query.case_ordinal);
    }
    let top_signature = support_by_signature
        .iter()
        .max_by(|left, right| {
            left.1
                .len()
                .cmp(&right.1.len())
                .then_with(|| right.0.cmp(left.0))
        })
        .map(|(signature, _)| signature);
    let top_supporting_cases = top_signature
        .and_then(|signature| support_by_signature.get(signature))
        .cloned()
        .unwrap_or_default();
    let snapshot = input.metadata.snapshot_ref.clone();
    let rows = cases
        .iter()
        .enumerate()
        .map(|(index, (_case_ordinal, events))| {
            let values = events
                .iter()
                .filter_map(|event| event.technical_error)
                .collect::<Vec<_>>();
            let technical_error = if values.iter().any(|value| *value) {
                "true"
            } else if values.is_empty() {
                ""
            } else {
                "false"
            };
            let metric_field = if recurrence_metric {
                "recurring_query_case"
            } else {
                "technical_error"
            };
            let metric_value = if recurrence_metric {
                if top_supporting_cases.contains(_case_ordinal) {
                    "true"
                } else {
                    "false"
                }
            } else {
                technical_error
            };
            BTreeMap::from([
                (
                    "case_bucket".to_owned(),
                    format!("b{:04}", index / QUERY_BATCH_SIZE),
                ),
                (metric_field.to_owned(), metric_value.to_owned()),
            ])
        })
        .collect::<Vec<_>>();
    let access = LabAccess::new(
        input.metadata.run_id.clone(),
        input.metadata.tenant_id.clone(),
        "local_discovery",
        "local_grant",
        "local_adapter",
        snapshot.clone(),
        input.metadata.cutoff_unix_seconds.saturating_add(3_600),
    );
    let mut grants = InMemoryLabGrantAuthority::default();
    grants.issue(LabGrant::from_access(&access));
    let mut lab = LocalInvestigationLab::new(grants);
    let source = LabSource::new(
        LabSourceManifest {
            tenant_id: input.metadata.tenant_id.clone(),
            snapshot_ref: snapshot.clone(),
            source_contract_digest: derive_digest("pulso-safe-agent-input-v1"),
            source_digest: input.metadata.manifest_digest.clone(),
            transform_digest: derive_digest(&format!(
                "local_multi_signal_v1:{}:{}:{}",
                RECURRING_QUERY_POLICY_VERSION,
                input.minimum_recurring_query_support,
                recurrence_pattern_ref(input).unwrap_or_else(|| "no_recurrent_pattern".into())
            )),
            cutoff_unix_seconds: input.metadata.cutoff_unix_seconds,
            classification: LabDataClassification::Treated,
            safe_for_discovery: true,
        },
        vec![LabTable::new(
            "case_facts",
            vec![
                "case_bucket",
                if recurrence_metric {
                    "recurring_query_case"
                } else {
                    "technical_error"
                },
            ],
            rows,
        )],
    )
    .map_err(|error| LocalRunError::Lab(format!("source rejected: {error:?}")))?;
    let mut approval = InMemoryLabSourceAuthority;
    let approved = approval
        .approve(source)
        .map_err(|error| LocalRunError::Lab(format!("approval rejected: {error:?}")))?;
    let now = input.metadata.cutoff_unix_seconds.saturating_add(1);
    let session = lab
        .open(access.clone(), approved, now)
        .map_err(|error| LocalRunError::Lab(format!("session denied: {error:?}")))?;
    let case_count = cases.len();
    let batch_count = case_count.div_ceil(QUERY_BATCH_SIZE);
    let mut results = Vec::with_capacity(batch_count);
    for batch in 0..batch_count {
        let bucket = format!("b{batch:04}");
        results.push(
            lab.query(
                session.session_id(),
                &access,
                LabQuery::select(
                    "case_facts",
                    vec![if recurrence_metric {
                        "recurring_query_case"
                    } else {
                        "technical_error"
                    }],
                    Some(QueryFilter::equals("case_bucket", bucket)),
                ),
                now + 1,
            )
            .map_err(|error| LocalRunError::Lab(format!("read-only query failed: {error:?}")))?,
        );
    }
    let metric = if recurrence_metric {
        BooleanRateSpec::new(
            "e0_recurring_copilot_query_cases",
            "recurring_query_case",
            "true",
        )
    } else {
        BooleanRateSpec::new("e0_technical_error_rate", "technical_error", "true")
    }
    .map_err(|_| LocalRunError::InvalidInput)?;
    let signal = DeterministicSensor::measure(&metric, &results)
        .map_err(|error| LocalRunError::Lab(format!("sensor rejected evidence: {error:?}")))?;
    lab.close(session.session_id(), &access)
        .map_err(|error| LocalRunError::Lab(format!("ephemeral lab close failed: {error:?}")))?;
    Ok(signal)
}

fn recurrence_pattern_ref(input: &LocalRunInput) -> Option<String> {
    let mut support_by_signature = BTreeMap::<&str, BTreeSet<u32>>::new();
    for query in &input.queries {
        support_by_signature
            .entry(&query.opaque_signature)
            .or_default()
            .insert(query.case_ordinal);
    }
    let (signature, _) = support_by_signature.iter().max_by(|left, right| {
        left.1
            .len()
            .cmp(&right.1.len())
            .then_with(|| right.0.cmp(left.0))
    })?;
    Some(derive_digest(&format!(
        "pattern-v1:{}:{}:{}:{}",
        input.metadata.tenant_id,
        input.metadata.manifest_digest,
        input.metadata.snapshot_ref.digest,
        signature
    )))
}

fn summarize_signal(input: &LocalRunInput, signal: &DeterministicSignal) -> SignalSummary {
    let recurrence = signal.metric_id == "e0_recurring_copilot_query_cases";
    SignalSummary {
        metric_id: signal.metric_id.clone(),
        detector_policy_id: if recurrence {
            RECURRING_QUERY_POLICY.to_owned()
        } else {
            "e0_technical_error_rate_v1".to_owned()
        },
        detector_policy_version: if recurrence {
            RECURRING_QUERY_POLICY_VERSION
        } else {
            1
        },
        minimum_support: if recurrence {
            input.minimum_recurring_query_support
        } else {
            1
        },
        numerator: signal.numerator,
        denominator: signal.denominator,
        missing: signal.missing,
        coverage_basis_points: signal.coverage_basis_points,
        pattern_ref: recurrence.then(|| recurrence_pattern_ref(input)).flatten(),
        digest: signal.digest.clone(),
    }
}

fn select_primary_signal_index(
    signals: &[DeterministicSignal],
    minimum_recurring_query_support: u64,
) -> Option<usize> {
    signals
        .iter()
        .enumerate()
        .max_by(|left, right| {
            let qualifies = |signal: &DeterministicSignal| {
                if signal.metric_id == "e0_recurring_copilot_query_cases" {
                    signal.numerator >= minimum_recurring_query_support && signal.denominator > 0
                } else {
                    signal.numerator > 0 && signal.denominator > 0
                }
            };
            let left_signal = left.1;
            let right_signal = right.1;
            qualifies(left_signal)
                .cmp(&qualifies(right_signal))
                .then_with(|| {
                    (left_signal.numerator * right_signal.denominator)
                        .cmp(&(right_signal.numerator * left_signal.denominator))
                })
                .then_with(|| {
                    (left_signal.metric_id == "e0_technical_error_rate")
                        .cmp(&(right_signal.metric_id == "e0_technical_error_rate"))
                })
        })
        .map(|(index, _)| index)
}

fn simulate_agent_core_and_model(
    input: &LocalRunInput,
    signal: &DeterministicSignal,
) -> Result<
    (
        CoreTaskScope,
        crate::core_task::CoreTaskReceipt,
        crate::model_provider::ModelReceipt,
    ),
    LocalRunError,
> {
    let job_id = format!("job_{}", input.metadata.run_id.replace('-', "_"));
    let scope = CoreTaskScope::new(
        input.metadata.tenant_id.clone(),
        job_id,
        "local_grant",
        "local_adapter",
    )
    .map_err(|error| LocalRunError::CoreTask(format!("invalid local scope: {error:?}")))?;
    let binding = CoreTaskBinding::new(
        "pulso_scout",
        "local_simulation_v1",
        AGENT_CORE_CONTRACT_VERSION,
        AGENT_CORE_CONTRACT_SHA,
    )
    .map_err(|error| LocalRunError::CoreTask(format!("invalid pinned binding: {error:?}")))?;
    let registry = CoreTaskBindingRegistry::new(vec![binding.clone()])
        .map_err(|error| LocalRunError::CoreTask(format!("binding not approved: {error:?}")))?;
    let mut core = CoreTaskSimulator::new(registry);
    core.script_success("local_core_run", derive_digest(signal.digest.as_str()))
        .map_err(|error| {
            LocalRunError::CoreTask(format!("local receipt script invalid: {error:?}"))
        })?;
    let core_receipt = core
        .invoke(
            CoreTaskInvocation::new(
                scope.clone(),
                binding,
                format!("attempt_{}", input.metadata.run_id.replace('-', "_")),
                signal.digest.clone(),
            )
            .map_err(|error| {
                LocalRunError::CoreTask(format!("local invocation invalid: {error:?}"))
            })?,
        )
        .map_err(|error| LocalRunError::CoreTask(format!("local task failed: {error:?}")))?;

    let capability = ModelCapability::new(
        ModelProvider::OpenRouter,
        "https://openrouter.ai/api/v1",
        "local/simulation-only",
        "secret://local-simulation/not-used",
        "local_simulation_v1",
    )
    .map_err(|error| LocalRunError::Model(format!("local model capability invalid: {error:?}")))?;
    let policy = ModelPolicy::with_budget(
        "local_scout_policy",
        capability,
        "scout_local_simulation",
        RedactionPolicy::RejectMarkedInput,
        0,
        500,
        ModelBudgetLimits::new(256, 32, 1)
            .map_err(|error| LocalRunError::Model(format!("local budget invalid: {error:?}")))?,
    )
    .map_err(|error| LocalRunError::Model(format!("local model policy invalid: {error:?}")))?;
    let safe_projection = format!(
        "metric={} numerator={} denominator={} missing={}",
        signal.metric_id, signal.numerator, signal.denominator, signal.missing
    );
    let mut broker = HmacProjectionBroker::new_for_local_simulation(
        b"pulso-local-simulation-projection-key-32b",
    )
    .map_err(|error| LocalRunError::Model(format!("local projection broker invalid: {error:?}")))?;
    let projection = broker
        .authorize_projection(&scope, &policy, safe_projection)
        .map_err(|error| LocalRunError::Model(format!("safe projection denied: {error:?}")))?;
    let invocation = ModelInvocation::from_verified(
        scope.clone(),
        policy.clone(),
        format!("attempt_model_{}", input.metadata.run_id.replace('-', "_")),
        projection,
    )
    .map_err(|error| LocalRunError::Model(format!("local invocation invalid: {error:?}")))?;
    let mut model = ModelProviderSimulator::new(policy);
    model.script_success("local simulated review", "local_request");
    let model_receipt = model
        .invoke(invocation)
        .map_err(|error| LocalRunError::Model(format!("local model port failed: {error:?}")))?;
    Ok((scope, core_receipt, model_receipt))
}

struct LocalUncertainVerifier;

impl IndependentEvidenceVerifierPort for LocalUncertainVerifier {
    fn verify(
        &mut self,
        input: IndependentVerificationInput,
    ) -> Result<IndependentVerificationReceipt, IndependentVerificationPortError> {
        IndependentVerificationReceipt::new(
            input.commitment(),
            "local_uncertain_verifier",
            SIMULATION_VERSION,
            derive_digest(&format!(
                "{}:no_independent_oracle",
                input.candidate_digest()
            )),
            VerificationStatus::Uncertain,
        )
        .map_err(|_| IndependentVerificationPortError::Unknown)
    }
}

fn build_exploratory_draft(
    input: &LocalRunInput,
    signal: &SignalSummary,
    report: &VerificationReport,
) -> ImprovementDraft {
    let mut route_counts = BTreeMap::<String, u64>::new();
    let mut layer_counts = BTreeMap::<String, u64>::new();
    let mut tool_counts = BTreeMap::<String, u64>::new();
    let mut event_kind_counts = BTreeMap::<String, u64>::new();
    let mut approval_counts = BTreeMap::<String, u64>::new();
    let mut signal_counts = BTreeMap::<String, u64>::new();
    let linked_event_count = input
        .events
        .iter()
        .filter(|event| event.parent_event_ordinal.is_some())
        .count();
    for event in &input.events {
        *event_kind_counts
            .entry(event.event_kind.clone())
            .or_default() += 1;
        if let Some(approval) = &event.approval {
            *approval_counts.entry(approval.clone()).or_default() += 1;
        }
        if let Some(signal_code) = &event.signal_code {
            *signal_counts.entry(signal_code.clone()).or_default() += 1;
        }
    }
    for event in input
        .events
        .iter()
        .filter(|event| event.technical_error == Some(true))
    {
        if let Some(route) = &event.route_code {
            *route_counts.entry(route.clone()).or_default() += 1;
        }
        if let Some(layer) = &event.actor_layer {
            *layer_counts.entry(layer.clone()).or_default() += 1;
        }
        if let Some(tool) = &event.tool_code {
            *tool_counts.entry(tool.clone()).or_default() += 1;
        }
    }
    let route = most_frequent(&route_counts).unwrap_or("not_observed");
    let layer = most_frequent(&layer_counts).unwrap_or("not_observed");
    let tool = most_frequent(&tool_counts).unwrap_or("not_observed");
    let hypothesis = if signal.metric_id == "e0_recurring_copilot_query_cases" {
        format!(
            "An opaque copilot query pattern recurs across {} of {} discovery cases; investigate whether a reusable Agent Core artifact could handle it. This recurrence is descriptive and does not prove friction, causality, or business lift.",
            signal.numerator, signal.denominator
        )
    } else {
        format!(
            "Investigate whether observed technical errors cluster around route {route}, layer {layer}, and tool {tool}; this co-occurrence is descriptive, not causal."
        )
    };
    let mut draft = json!({
        "artifact_kind": if signal.metric_id == "e0_recurring_copilot_query_cases" { "unclassified_candidate" } else { "flow" },
        "draft_id": format!("local_flow_{}", &signal.digest[7..19]),
        "version": "0.1.0-local-draft",
        "status": "simulated_unverified",
        "executable": false,
        "source_snapshot": input.metadata.snapshot_ref,
        "hypothesis": hypothesis,
        "observed_evidence": {
            "cases_observed": signal.denominator + signal.missing,
            "events_observed": input.events.len(),
            "parent_linked_events": linked_event_count,
            "metric_id": signal.metric_id,
            "detector_policy_id": signal.detector_policy_id,
            "detector_policy_version": signal.detector_policy_version,
            "minimum_support": signal.minimum_support,
            "numerator": signal.numerator,
            "denominator": signal.denominator,
            "missing": signal.missing,
            "pattern_ref": signal.pattern_ref,
            "primary_signal_policy": "local_primary_signal_v1",
            "route_code_with_most_errors": if signal.metric_id == "e0_technical_error_rate" { json!(route) } else { json!(null) },
            "actor_layer_with_most_errors": if signal.metric_id == "e0_technical_error_rate" { json!(layer) } else { json!(null) },
            "tool_code_with_most_errors": if signal.metric_id == "e0_technical_error_rate" { json!(tool) } else { json!(null) },
            "error_routes": if signal.metric_id == "e0_technical_error_rate" { json!(route_counts) } else { json!({}) },
            "error_actor_layers": if signal.metric_id == "e0_technical_error_rate" { json!(layer_counts) } else { json!({}) },
            "error_tools": if signal.metric_id == "e0_technical_error_rate" { json!(tool_counts) } else { json!({}) },
            "event_kinds": event_kind_counts,
            "approval_decisions": approval_counts,
            "signals": signal_counts,
        },
        "verification": {
            "status": verification_status_name(report.status()),
            "report_digest": report.receipt().digest(),
            "formal_route": "do_nothing",
        },
        "steps": [
            {"kind": "observe", "metric_id": signal.metric_id},
            {"kind": "route_to_human_review", "reason": "unverified_local_draft"}
        ],
        "native_compilation": "dependency_unavailable",
        "release_authorized": false,
    });
    // This digest is provenance only; it is not a native Agent Core signature.
    let digest = derive_digest(&draft.to_string());
    draft["content_digest"] = json!(digest);
    ImprovementDraft {
        status: "simulated_unverified".into(),
        execution_status: "not_executed".into(),
        hypothesis,
        evidence: signal.clone(),
        proposed_artifact: draft,
        digest,
        simulation_version: SIMULATION_VERSION.into(),
        native_agent_core_status: "dependency_unavailable".into(),
    }
}

fn evaluate_draft_shape(draft: &ImprovementDraft) -> EvaluationSummary {
    let checks = [
        draft.status == "simulated_unverified",
        draft.execution_status == "not_executed",
        draft.proposed_artifact["executable"] == false,
        draft.proposed_artifact["release_authorized"] == false,
        draft.proposed_artifact["native_compilation"] == "dependency_unavailable",
    ];
    EvaluationSummary {
        status: if checks.iter().all(|check| *check) {
            "simulated"
        } else {
            "failed"
        }
        .into(),
        evaluator: "local_draft_contract_check".into(),
        checks_passed: checks.iter().filter(|check| **check).count() as u32,
        checks_total: checks.len() as u32,
        claims_business_improvement: false,
    }
}

fn most_frequent(counts: &BTreeMap<String, u64>) -> Option<&str> {
    counts
        .iter()
        .max_by(|(left_name, left_count), (right_name, right_count)| {
            left_count
                .cmp(right_count)
                .then_with(|| right_name.cmp(left_name))
        })
        .map(|(name, _)| name.as_str())
}

fn verification_status_name(status: VerificationStatus) -> &'static str {
    match status {
        VerificationStatus::Supported => "supported",
        VerificationStatus::Refuted => "refuted",
        VerificationStatus::Uncertain => "uncertain",
    }
}

fn record_event(events: &mut Vec<RunEvent>, stage: &str, status: &str, detail: &str) {
    events.push(RunEvent {
        sequence: events.len() as u32 + 1,
        stage: stage.to_owned(),
        status: status.to_owned(),
        detail: detail.to_owned(),
        observed_cutoff_rfc3339: String::new(),
    });
}

fn bind_observed_cutoff(events: &mut [RunEvent], cutoff: &str) {
    for event in events {
        event.observed_cutoff_rfc3339 = cutoff.to_owned();
    }
}

fn valid_code(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-' | b'.'))
}

fn is_opaque_query_signature(value: &str) -> bool {
    value.len() == 63
        && value.starts_with("sha256_")
        && value.as_bytes()[7..]
            .iter()
            .all(|byte| matches!(*byte, b'0'..=b'9' | b'a'..=b'f'))
}

fn is_identifier(value: &str) -> bool {
    valid_code(value) && value.len() <= 128
}

fn is_digest(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value.as_bytes()[7..]
            .iter()
            .all(|byte| matches!(*byte, b'0'..=b'9' | b'a'..=b'f'))
}

fn derive_digest(value: &str) -> String {
    format!("sha256:{:x}", Sha256::digest(value.as_bytes()))
}

/// Strict U04-compatible UTC whole-second parser. Local E0 event time is
/// treated as event provenance, never as an outcome or source of labels.
fn parse_utc_seconds(value: &str) -> Option<u64> {
    if !value.is_ascii()
        || value.len() != 20
        || &value[4..5] != "-"
        || &value[7..8] != "-"
        || &value[10..11] != "T"
        || &value[13..14] != ":"
        || &value[16..17] != ":"
        || &value[19..20] != "Z"
    {
        return None;
    }
    let year: i64 = value[0..4].parse().ok()?;
    let month: i64 = value[5..7].parse().ok()?;
    let day: i64 = value[8..10].parse().ok()?;
    let hour: i64 = value[11..13].parse().ok()?;
    let minute: i64 = value[14..16].parse().ok()?;
    let second: i64 = value[17..19].parse().ok()?;
    if !(1..=12).contains(&month)
        || day < 1
        || day > days_in_month(year, month)
        || hour > 23
        || minute > 59
        || second > 59
    {
        return None;
    }
    let mut adjusted_year = year;
    if month <= 2 {
        adjusted_year -= 1;
    }
    let era = adjusted_year.div_euclid(400);
    let year_of_era = adjusted_year - era * 400;
    let adjusted_month = month + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * adjusted_month + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days_since_epoch = era * 146_097 + day_of_era - 719_468;
    let seconds = days_since_epoch * 86_400 + hour * 3_600 + minute * 60 + second;
    u64::try_from(seconds).ok()
}

fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::parse_utc_seconds;

    #[test]
    fn parses_only_utc_whole_seconds_and_rejects_invalid_calendar_dates() {
        assert_eq!(parse_utc_seconds("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(
            parse_utc_seconds("2026-08-01T00:00:01Z"),
            Some(1_785_542_401)
        );
        assert_eq!(parse_utc_seconds("2026-02-30T00:00:00Z"), None);
        assert_eq!(parse_utc_seconds("2026-08-01T00:00:00.123Z"), None);
        assert_eq!(parse_utc_seconds("2026-08-01T00:00:00-05:00"), None);
    }
}
