//! Local, read-only dataset adapters for the improvement engine.
//!
//! This crate deliberately keeps source records out of public manifests. The
//! original-bank adapter seals CSV bytes without projecting raw row values. The
//! E0 adapter emits only a small typed event projection; evaluator labels have
//! a separate API and are never stored in `PreparedSource`.

use std::collections::BTreeMap;
use std::fmt;
use std::fs::{self, File};
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use arrow_array::{
    Array, Int32Array, Int64Array, LargeStringArray, RecordBatch, StringArray,
    TimestampMicrosecondArray, TimestampMillisecondArray, TimestampNanosecondArray,
    TimestampSecondArray,
};
use arrow_schema::{DataType, SchemaRef};
use improvement_engine_core::ArtifactReference;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use serde::Serialize;
use sha2::{Digest, Sha256};

const PREPARATION_POLICY_VERSION: &str = "pulso.source-preparation.v1";
const E0_AGENT_TABLES: &[&str] = &[
    "case",
    "identity_check",
    "turn",
    "routing_step",
    "copilot_query",
    "tool_call",
    "approval",
    "signal",
];

/// Provenance and deterministic limits supplied by the local runner.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreparationConfig {
    tenant_id: String,
    observed_cutoff: String,
    cutoff_unix_seconds: u64,
    arranque_cases: usize,
}

impl PreparationConfig {
    pub fn new(
        tenant_id: impl Into<String>,
        observed_cutoff: impl Into<String>,
        arranque_cases: usize,
    ) -> Result<Self, AdapterError> {
        let tenant_id = tenant_id.into();
        let observed_cutoff = observed_cutoff.into();
        if tenant_id.trim().is_empty() || tenant_id.len() > 128 {
            return Err(AdapterError::InvalidConfig(
                "tenant_id is empty or too long",
            ));
        }
        if !is_utc_timestamp(&observed_cutoff) {
            return Err(AdapterError::InvalidConfig(
                "observed_cutoff must be an RFC3339 UTC timestamp",
            ));
        }
        if arranque_cases == 0 {
            return Err(AdapterError::InvalidConfig(
                "arranque_cases must be greater than zero",
            ));
        }
        let cutoff_unix_seconds = parse_utc_timestamp(&observed_cutoff)? as u64;
        Ok(Self {
            tenant_id,
            observed_cutoff,
            cutoff_unix_seconds,
            arranque_cases,
        })
    }

    #[must_use]
    pub fn arranque_cases(&self) -> usize {
        self.arranque_cases
    }

    #[must_use]
    pub fn cutoff_unix_seconds(&self) -> u64 {
        self.cutoff_unix_seconds
    }

    #[must_use]
    pub fn observed_cutoff(&self) -> &str {
        &self.observed_cutoff
    }
}

/// Source identity is deliberately independent of the local path where data
/// was mounted; manifests can be compared across machines without leaking it.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    OriginalBank,
    E0,
}

/// Immutable metadata and the agent-safe projection for one prepared source.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub enum PreparedSource {
    OriginalBank(PreparedSourcePayload),
    E0(PreparedSourcePayload),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PreparedSourcePayload {
    source_kind: SourceKind,
    manifest_digest: String,
    snapshot_ref: ArtifactReference,
    cutoff_unix_seconds: u64,
    observed_cutoff: String,
    agent_inputs: AgentInputSet,
}

impl PreparedSource {
    #[must_use]
    pub fn source_kind(&self) -> SourceKind {
        match self {
            Self::OriginalBank(_) => SourceKind::OriginalBank,
            Self::E0(_) => SourceKind::E0,
        }
    }

    #[must_use]
    pub fn manifest_digest(&self) -> &str {
        self.payload().manifest_digest()
    }

    #[must_use]
    pub fn snapshot_id(&self) -> &str {
        self.payload().snapshot_id()
    }

    #[must_use]
    pub fn snapshot_ref(&self) -> &ArtifactReference {
        self.payload().snapshot_ref()
    }

    #[must_use]
    pub fn cutoff_unix_seconds(&self) -> u64 {
        self.payload().cutoff_unix_seconds()
    }

    #[must_use]
    pub fn observed_cutoff(&self) -> &str {
        self.payload().observed_cutoff()
    }

    #[must_use]
    pub fn agent_inputs(&self) -> &AgentInputSet {
        self.payload().agent_inputs()
    }

    fn payload(&self) -> &PreparedSourcePayload {
        match self {
            Self::OriginalBank(payload) | Self::E0(payload) => payload,
        }
    }
}

impl PreparedSourcePayload {
    #[must_use]
    pub fn manifest_digest(&self) -> &str {
        &self.manifest_digest
    }
    #[must_use]
    pub fn snapshot_id(&self) -> &str {
        &self.snapshot_ref.id
    }
    #[must_use]
    pub fn snapshot_ref(&self) -> &ArtifactReference {
        &self.snapshot_ref
    }
    #[must_use]
    pub fn cutoff_unix_seconds(&self) -> u64 {
        self.cutoff_unix_seconds
    }
    #[must_use]
    pub fn observed_cutoff(&self) -> &str {
        &self.observed_cutoff
    }
    #[must_use]
    pub fn agent_inputs(&self) -> &AgentInputSet {
        &self.agent_inputs
    }
}

/// Frozen, typed observations accepted by the agent/scout-facing runner.
/// There is intentionally no arbitrary column map, customer id, message, or
/// evaluator outcome in this type.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct AgentInputSet {
    cases: Vec<AgentCase>,
    facts: Vec<E0Fact>,
    unsupported_metrics: Vec<UnsupportedMetric>,
}

impl AgentInputSet {
    #[must_use]
    pub fn cases(&self) -> &[AgentCase] {
        &self.cases
    }

    #[must_use]
    pub fn facts(&self) -> &[E0Fact] {
        &self.facts
    }

    #[must_use]
    pub fn unsupported_metrics(&self) -> &[UnsupportedMetric] {
        &self.unsupported_metrics
    }
}

/// Typed, allowlisted E0 history facts. Identifiers are replaced by case and
/// per-case ordinals; text, free-form params, customer IDs, and evaluator labels
/// are absent by construction.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "fact_type", rename_all = "snake_case")]
pub enum E0Fact {
    Case {
        case_ordinal: u32,
        opened_at: String,
        opened_at_unix_micros: i64,
        channel: String,
        language: String,
        origin: Option<String>,
        topic: String,
        priority: String,
    },
    IdentityCheck {
        case_ordinal: u32,
        event_ordinal: u32,
        event_time: String,
        event_time_unix_micros: i64,
        actor_role: String,
        result: String,
        correct: Option<u64>,
        attempt: Option<u32>,
        trigger: Option<String>,
        policy_rule_id: Option<String>,
    },
    Turn {
        case_ordinal: u32,
        event_ordinal: u32,
        event_time: String,
        event_time_unix_micros: i64,
        author_role: String,
        language: String,
    },
    RoutingStep {
        case_ordinal: u32,
        event_ordinal: u32,
        event_time: String,
        event_time_unix_micros: i64,
        tier: String,
        outcome: String,
        reason_code: Option<String>,
        confidence: Option<String>,
        handoff: Option<bool>,
    },
    CopilotQuery {
        case_ordinal: u32,
        event_ordinal: u32,
        event_time: String,
        event_time_unix_micros: i64,
        query_signature: String,
        tables_read: Vec<String>,
        columns_read: Vec<String>,
        answered_by: String,
        sent_to_chat: bool,
    },
    ToolCall {
        case_ordinal: u32,
        event_ordinal: u32,
        event_time: String,
        event_time_unix_micros: i64,
        actor_role: String,
        tool_id: String,
        permission_level: String,
        status: String,
        verified: Option<bool>,
        state_change: Option<bool>,
        retry_count: Option<u32>,
        latency_ms: Option<u64>,
        technical_error: bool,
    },
    Approval {
        case_ordinal: u32,
        event_ordinal: u32,
        requested_at: String,
        requested_at_unix_micros: i64,
        requested_by_role: String,
        tool_id: String,
        reason_code: Option<String>,
        policy_rule_id: Option<String>,
        decided_at: Option<String>,
        decided_at_unix_micros: Option<i64>,
        decision: Option<String>,
        related_tool_ordinal: Option<u32>,
    },
    Signal {
        event_ordinal: u32,
        kind: String,
        scope: String,
        window_start: String,
        window_end: String,
        window_end_unix_micros: i64,
        support_cases: u32,
        support_analysts: u32,
        consistency: Option<String>,
        status: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct AgentCase {
    ordinal: u32,
    phase: CasePhase,
    opened_at: String,
    events: Vec<SafeEvent>,
}

impl AgentCase {
    #[must_use]
    pub fn ordinal(&self) -> u32 {
        self.ordinal
    }

    #[must_use]
    pub fn phase(&self) -> CasePhase {
        self.phase
    }

    #[must_use]
    pub fn opened_at(&self) -> &str {
        &self.opened_at
    }

    #[must_use]
    pub fn events(&self) -> &[SafeEvent] {
        &self.events
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CasePhase {
    Arranque,
    Reproduccion,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SafeEvent {
    case_ordinal: u32,
    event_ordinal: u32,
    parent_event_ordinal: Option<u32>,
    event_time: String,
    event_time_unix_micros: i64,
    event_kind: String,
    route_code: Option<String>,
    actor_role: Option<String>,
    tool_code: Option<String>,
    technical_error: Option<bool>,
    approval: Option<bool>,
    signal_code: Option<String>,
}

impl SafeEvent {
    #[must_use]
    pub fn case_ordinal(&self) -> u32 {
        self.case_ordinal
    }
    #[must_use]
    pub fn event_ordinal(&self) -> u32 {
        self.event_ordinal
    }
    #[must_use]
    pub fn parent_event_ordinal(&self) -> Option<u32> {
        self.parent_event_ordinal
    }
    #[must_use]
    pub fn event_time(&self) -> &str {
        &self.event_time
    }
    #[must_use]
    pub fn event_time_unix_micros(&self) -> i64 {
        self.event_time_unix_micros
    }
    #[must_use]
    pub fn event_kind(&self) -> &str {
        &self.event_kind
    }
    #[must_use]
    pub fn route_code(&self) -> Option<&str> {
        self.route_code.as_deref()
    }
    #[must_use]
    pub fn actor_role(&self) -> Option<&str> {
        self.actor_role.as_deref()
    }
    #[must_use]
    pub fn tool_code(&self) -> Option<&str> {
        self.tool_code.as_deref()
    }
    #[must_use]
    pub fn technical_error(&self) -> Option<bool> {
        self.technical_error
    }
    #[must_use]
    pub fn approval(&self) -> Option<bool> {
        self.approval
    }
    #[must_use]
    pub fn signal_code(&self) -> Option<&str> {
        self.signal_code.as_deref()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UnsupportedMetric {
    TechnicalErrorNotPresentInOriginalBankHistory,
}

/// The original-bank adapter scans source bytes to seal a manifest, but does
/// not expose source rows. Existing original-source schemas do not define an
/// approved technical-error metric; fabricating zero-error observations would
/// produce a false result.
pub fn prepare_original_bank(
    root: &Path,
    config: &PreparationConfig,
) -> Result<PreparedSource, AdapterError> {
    let files = scan_csv_tree(root)?;
    if files.is_empty() {
        return Err(AdapterError::MissingInput("no CSV files found"));
    }
    let manifest = DatasetManifest::new(
        SourceKind::OriginalBank,
        &config.tenant_id,
        &config.observed_cutoff,
        files,
    );
    prepared_source(
        SourceKind::OriginalBank,
        config,
        &manifest,
        AgentInputSet {
            cases: Vec::new(),
            facts: Vec::new(),
            unsupported_metrics: vec![
                UnsupportedMetric::TechnicalErrorNotPresentInOriginalBankHistory,
            ],
        },
    )
}

/// Reads allowlisted interaction facts from the E0 Parquet package. The
/// resulting case order is derived from `opened_at` (then private `case_id` as
/// a stable tie-breaker), never from evaluator labels or rank fields.
pub fn prepare_e0_package(
    root: &Path,
    config: &PreparationConfig,
) -> Result<PreparedSource, AdapterError> {
    let data_dir = root.join("datos");
    let mut manifest_entries = Vec::new();
    for table in E0_AGENT_TABLES {
        let relative_path = PathBuf::from("datos").join(format!("{table}.parquet"));
        let path = root.join(&relative_path);
        if path.exists() {
            manifest_entries.push(manifest_for_file(root, &path, &relative_path, table, true)?);
        } else if matches!(*table, "case" | "tool_call") {
            return Err(AdapterError::MissingInput(
                "E0 package is missing required case/tool_call tables",
            ));
        }
    }
    let platform_contract_path = PathBuf::from("contratos/platform_history.json");
    let platform_contract = root.join(&platform_contract_path);
    let contract_bytes = fs::read(&platform_contract).map_err(|source| AdapterError::ReadFile {
        path: platform_contract_path.display().to_string(),
        source,
    })?;
    let contract_digest = digest(&contract_bytes);
    manifest_entries.push(ManifestEntry {
        relative_path: "contratos/platform_history.json".to_owned(),
        table: "platform_history_contract".to_owned(),
        file_digest: contract_digest.clone(),
        header_digest: contract_digest,
        row_count: 1,
    });
    for entry in fs::read_dir(&data_dir).map_err(AdapterError::ReadDirectory)? {
        let path = entry.map_err(AdapterError::ReadDirectory)?.path();
        if path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("parquet"))
        {
            let table = path
                .file_stem()
                .and_then(|name| name.to_str())
                .unwrap_or_default();
            if !E0_AGENT_TABLES.contains(&table)
                && !matches!(table, "labels" | "timeline" | "case_close")
            {
                return Err(AdapterError::InvalidInput(
                    "E0 package contains an unclassified Parquet table",
                ));
            }
        }
    }
    manifest_entries.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));

    let mut raw_cases = read_cases(&data_dir.join("case.parquet"))?;
    raw_cases.sort_by(|left, right| {
        left.opened_at_unix_micros
            .cmp(&right.opened_at_unix_micros)
            .then_with(|| left.internal_case_id.cmp(&right.internal_case_id))
    });
    let cutoff_micros = (config.cutoff_unix_seconds as i64).saturating_mul(1_000_000);
    let included_case_count =
        raw_cases.partition_point(|case| case.opened_at_unix_micros <= cutoff_micros);
    if included_case_count < config.arranque_cases {
        return Err(AdapterError::InvalidInput(
            "E0 package has fewer pre-cutoff cases than arranque_cases",
        ));
    }
    // Keep the complete chronological id map only while resolving source
    // references. Future case ordinals form a suffix, then all future data is
    // removed before the discovery projection is built.
    let sorted = raw_cases;
    let calls = read_tool_calls(&data_dir.join("tool_call.parquet"))?;
    let ordinal_by_id: BTreeMap<String, u32> = sorted
        .iter()
        .enumerate()
        .map(|(index, case)| (case.internal_case_id.clone(), (index + 1) as u32))
        .collect();
    let mut facts = Vec::new();
    for (index, case) in sorted.iter().enumerate() {
        facts.push(E0Fact::Case {
            case_ordinal: (index + 1) as u32,
            opened_at: case.opened_at.clone(),
            opened_at_unix_micros: case.opened_at_unix_micros,
            channel: safe_domain(
                &case.channel,
                &[
                    "app_chat", "web_chat", "whatsapp", "phone", "email", "video",
                ],
            ),
            language: safe_domain(&case.language, &["es", "pt"]),
            origin: case.origin.as_deref().and_then(|value| {
                safe_optional_domain(value, &["customer", "regulator", "branch"])
            }),
            topic: safe_domain(
                &case.topic,
                &[
                    "consultar_movimientos",
                    "consultar_cargo",
                    "disputar_cargo",
                    "cobro_duplicado",
                    "estado_disputa",
                    "fraude_urgente",
                    "hablar_con_humano",
                    "fuera_de_alcance",
                    "problema_app",
                ],
            ),
            priority: safe_domain(&case.priority, &["low", "medium", "high"]),
        });
    }
    let mut tool_ordinal_by_id = BTreeMap::new();
    let mut calls = calls;
    calls.sort_by(|left, right| {
        left.internal_case_id
            .cmp(&right.internal_case_id)
            .then_with(|| {
                left.event_time_unix_micros
                    .cmp(&right.event_time_unix_micros)
            })
            .then_with(|| left.internal_call_id.cmp(&right.internal_call_id))
    });
    for call in calls {
        let ordinal =
            *ordinal_by_id
                .get(&call.internal_case_id)
                .ok_or(AdapterError::InvalidInput(
                    "tool_call references unknown case",
                ))?;
        let technical_error = matches!(call.status.as_str(), "error" | "timeout");
        let event_ordinal = tool_ordinal_by_id.len() as u32 + 1;
        tool_ordinal_by_id.insert(call.internal_call_id.clone(), (ordinal, event_ordinal));
        facts.push(E0Fact::ToolCall {
            case_ordinal: ordinal,
            event_ordinal,
            event_time: call.event_time,
            event_time_unix_micros: call.event_time_unix_micros,
            actor_role: call.actor_role,
            tool_id: call.tool_id,
            permission_level: call.permission_level,
            status: call.status,
            verified: call.verified,
            state_change: call.state_change,
            retry_count: call.retry_count,
            latency_ms: call.latency_ms,
            technical_error,
        });
    }
    facts.extend(read_other_facts(
        &data_dir,
        &ordinal_by_id,
        &tool_ordinal_by_id,
    )?);
    facts.retain_mut(|fact| {
        if let E0Fact::Approval {
            decided_at_unix_micros,
            decided_at,
            decision,
            ..
        } = fact
        {
            if decided_at_unix_micros.is_none_or(|time| time > cutoff_micros) {
                *decided_at_unix_micros = None;
                *decided_at = None;
                *decision = None;
            }
        }
        fact_case_ordinal(fact).is_none_or(|ordinal| ordinal <= included_case_count as u32)
            && fact_event_unix_micros(fact).is_none_or(|time| time <= cutoff_micros)
    });
    let events_by_ordinal: BTreeMap<u32, Vec<SafeEvent>> = (1..=included_case_count as u32)
        .map(|case_ordinal| Ok((case_ordinal, events_for_case(case_ordinal, &facts)?)))
        .collect::<Result<_, AdapterError>>()?;
    let cases = sorted
        .into_iter()
        .take(included_case_count)
        .enumerate()
        .map(|(index, case)| {
            let ordinal = (index + 1) as u32;
            let events = events_by_ordinal.get(&ordinal).cloned().unwrap_or_default();
            AgentCase {
                ordinal,
                phase: if index < config.arranque_cases {
                    CasePhase::Arranque
                } else {
                    CasePhase::Reproduccion
                },
                opened_at: case.opened_at,
                events,
            }
        })
        .collect();

    let manifest = DatasetManifest::new(
        SourceKind::E0,
        &config.tenant_id,
        &config.observed_cutoff,
        manifest_entries,
    );
    prepared_source(
        SourceKind::E0,
        config,
        &manifest,
        AgentInputSet {
            cases,
            facts,
            unsupported_metrics: Vec::new(),
        },
    )
}

/// Evaluator-only API. It is intentionally separate from `PreparedSource` and
/// reads `labels.parquet` only when the evaluator explicitly asks for labels.
pub mod evaluator {
    use std::fs::File;
    use std::path::Path;

    use super::required_strings;
    use arrow_array::BooleanArray;
    use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
    use serde::Serialize;

    use crate::{AdapterError, optional_u64, require_column};

    #[derive(Clone, Debug, Eq, PartialEq, Serialize)]
    pub struct EvaluatorOnlyLabels {
        rows: Vec<EvaluatorLabel>,
    }

    impl EvaluatorOnlyLabels {
        #[must_use]
        pub fn rows(&self) -> &[EvaluatorLabel] {
            &self.rows
        }
    }

    #[derive(Clone, Debug, Eq, PartialEq, Serialize)]
    pub struct EvaluatorLabel {
        rank: u32,
        case_id: String,
        split: String,
        final_status: String,
        final_sla_breached: bool,
    }

    impl EvaluatorLabel {
        #[must_use]
        pub fn rank(&self) -> u32 {
            self.rank
        }

        #[must_use]
        pub fn final_status(&self) -> &str {
            &self.final_status
        }
    }

    pub fn load_labels(root: &Path) -> Result<EvaluatorOnlyLabels, AdapterError> {
        let path = root.join("datos").join("labels.parquet");
        let file = File::open(&path).map_err(|source| AdapterError::ReadFile {
            path: "datos/labels.parquet".to_owned(),
            source,
        })?;
        let mut reader = ParquetRecordBatchReaderBuilder::try_new(file)
            .map_err(|error| AdapterError::Parquet(error.to_string()))?
            .build()
            .map_err(|error| AdapterError::Parquet(error.to_string()))?;
        let mut rows = Vec::new();
        while let Some(batch) = reader.next() {
            let batch = batch.map_err(|error| AdapterError::Parquet(error.to_string()))?;
            let case_ids = required_strings(&batch, "case_id")?;
            let splits = required_strings(&batch, "split")?;
            let final_statuses = required_strings(&batch, "final_status")?;
            let sla = require_column::<BooleanArray>(&batch, "final_sla_breached")?;
            for row in 0..batch.num_rows() {
                let rank = optional_u64(&batch, "rank", row)?
                    .filter(|rank| *rank > 0 && *rank <= u32::MAX as u64)
                    .ok_or(AdapterError::InvalidInput(
                        "evaluator label rank is invalid",
                    ))?;
                rows.push(EvaluatorLabel {
                    rank: rank as u32,
                    case_id: case_ids.value(row).to_owned(),
                    split: splits.value(row).to_owned(),
                    final_status: final_statuses.value(row).to_owned(),
                    final_sla_breached: sla.value(row),
                });
            }
        }
        rows.sort_by_key(|row| row.rank);
        Ok(EvaluatorOnlyLabels { rows })
    }
}

#[derive(Debug)]
pub enum AdapterError {
    InvalidConfig(&'static str),
    InvalidInput(&'static str),
    InvalidInputOwned(String),
    MissingInput(&'static str),
    ReadDirectory(std::io::Error),
    ReadFile {
        path: String,
        source: std::io::Error,
    },
    Parquet(String),
    UnsupportedSchema(String),
    Serialization(serde_json::Error),
}

impl fmt::Display for AdapterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfig(message)
            | Self::InvalidInput(message)
            | Self::MissingInput(message) => formatter.write_str(message),
            Self::InvalidInputOwned(message) => formatter.write_str(message),
            Self::ReadDirectory(source) => {
                write!(formatter, "cannot read source directory: {source}")
            }
            Self::ReadFile { path, source } => {
                write!(formatter, "cannot read source file {path}: {source}")
            }
            Self::Parquet(message) => write!(formatter, "cannot read source Parquet: {message}"),
            Self::UnsupportedSchema(message) => {
                write!(formatter, "unsupported source schema: {message}")
            }
            Self::Serialization(source) => {
                write!(formatter, "cannot seal source manifest: {source}")
            }
        }
    }
}

impl std::error::Error for AdapterError {}

#[derive(Serialize)]
struct DatasetManifest<'a> {
    manifest_version: u32,
    policy_version: &'static str,
    source_kind: SourceKind,
    tenant_id: &'a str,
    observed_cutoff: &'a str,
    entries: Vec<ManifestEntry>,
}

impl<'a> DatasetManifest<'a> {
    fn new(
        source_kind: SourceKind,
        tenant_id: &'a str,
        observed_cutoff: &'a str,
        entries: Vec<ManifestEntry>,
    ) -> Self {
        Self {
            manifest_version: 1,
            policy_version: PREPARATION_POLICY_VERSION,
            source_kind,
            tenant_id,
            observed_cutoff,
            entries,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
struct ManifestEntry {
    relative_path: String,
    table: String,
    file_digest: String,
    header_digest: String,
    row_count: u64,
}

fn prepared_source(
    source_kind: SourceKind,
    config: &PreparationConfig,
    manifest: &DatasetManifest<'_>,
    agent_inputs: AgentInputSet,
) -> Result<PreparedSource, AdapterError> {
    let bytes = serde_json::to_vec(manifest).map_err(AdapterError::Serialization)?;
    let manifest_digest = digest(&bytes);
    let id = uuid_v7();
    let snapshot_ref = ArtifactReference {
        tenant_id: config.tenant_id.clone(),
        id,
        revision: 1,
        digest: manifest_digest.clone(),
    };
    let payload = PreparedSourcePayload {
        source_kind,
        manifest_digest,
        snapshot_ref,
        cutoff_unix_seconds: config.cutoff_unix_seconds,
        observed_cutoff: config.observed_cutoff.clone(),
        agent_inputs,
    };
    Ok(match source_kind {
        SourceKind::OriginalBank => PreparedSource::OriginalBank(payload),
        SourceKind::E0 => PreparedSource::E0(payload),
    })
}

fn scan_csv_tree(root: &Path) -> Result<Vec<ManifestEntry>, AdapterError> {
    let mut paths = Vec::new();
    collect_csv_paths(root, root, &mut paths)?;
    paths.sort();
    paths
        .into_iter()
        .map(|path| {
            let relative = path.strip_prefix(root).expect("collected under root");
            let table = relative
                .components()
                .next()
                .and_then(|part| part.as_os_str().to_str())
                .unwrap_or("source")
                .to_owned();
            manifest_for_csv(root, &path, relative, &table)
        })
        .collect()
}

fn collect_csv_paths(
    root: &Path,
    directory: &Path,
    output: &mut Vec<PathBuf>,
) -> Result<(), AdapterError> {
    let entries = fs::read_dir(directory).map_err(AdapterError::ReadDirectory)?;
    for entry in entries {
        let entry = entry.map_err(AdapterError::ReadDirectory)?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path).map_err(AdapterError::ReadDirectory)?;
        if metadata.file_type().is_symlink() {
            return Err(AdapterError::InvalidInput(
                "symlink sources are not permitted",
            ));
        }
        if metadata.is_dir() {
            collect_csv_paths(root, &path, output)?;
        } else if metadata.is_file()
            && path
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("csv"))
        {
            output.push(path);
        }
    }
    Ok(())
}

fn manifest_for_csv(
    root: &Path,
    path: &Path,
    relative: &Path,
    table: &str,
) -> Result<ManifestEntry, AdapterError> {
    let mut hasher = Sha256::new();
    let mut source = File::open(path).map_err(|source| AdapterError::ReadFile {
        path: relative.display().to_string(),
        source,
    })?;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = source
            .read(&mut buffer)
            .map_err(|source| AdapterError::ReadFile {
                path: relative.display().to_string(),
                source,
            })?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(true)
        .flexible(false)
        .from_path(path)
        .map_err(|error| {
            AdapterError::InvalidInputOwned(format!(
                "cannot parse CSV {}: {error}",
                relative.display()
            ))
        })?;
    let headers = reader
        .byte_headers()
        .map_err(|error| {
            AdapterError::InvalidInputOwned(format!(
                "cannot parse CSV header {}: {error}",
                relative.display()
            ))
        })?
        .iter()
        .map(|value| value.to_vec())
        .collect::<Vec<_>>();
    if headers.is_empty() {
        return Err(AdapterError::InvalidInput("CSV source has no header"));
    }
    let canonical_header = serde_json::to_vec(&headers).map_err(AdapterError::Serialization)?;
    let mut row_count = 0_u64;
    let mut records = reader.into_byte_records();
    while let Some(record) = records.next() {
        record.map_err(|error| {
            AdapterError::InvalidInputOwned(format!(
                "cannot parse CSV record {}: {error}",
                relative.display()
            ))
        })?;
        row_count = row_count.saturating_add(1);
    }
    let relative_path = path
        .strip_prefix(root)
        .unwrap_or(relative)
        .to_string_lossy()
        .replace('\\', "/");
    Ok(ManifestEntry {
        relative_path,
        table: table.to_owned(),
        file_digest: format!("sha256:{:x}", hasher.finalize()),
        header_digest: digest(&canonical_header),
        row_count,
    })
}

fn manifest_for_file(
    root: &Path,
    path: &Path,
    relative: &Path,
    table: &str,
    parquet_rows: bool,
) -> Result<ManifestEntry, AdapterError> {
    let file = File::open(path).map_err(|source| AdapterError::ReadFile {
        path: relative.display().to_string(),
        source,
    })?;
    let mut reader = BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = reader
            .read(&mut buffer)
            .map_err(|source| AdapterError::ReadFile {
                path: relative.display().to_string(),
                source,
            })?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    let file_digest = format!("sha256:{:x}", hasher.finalize());
    let (header_digest, row_count) = if parquet_rows {
        let file = File::open(path).map_err(|source| AdapterError::ReadFile {
            path: relative.display().to_string(),
            source,
        })?;
        let builder = ParquetRecordBatchReaderBuilder::try_new(file)
            .map_err(|error| AdapterError::Parquet(error.to_string()))?;
        let schema_digest = digest(format!("{:?}", builder.schema()).as_bytes());
        let mut batches = builder
            .build()
            .map_err(|error| AdapterError::Parquet(error.to_string()))?;
        let mut count = 0_u64;
        while let Some(batch) = batches.next() {
            count += batch
                .map_err(|error| AdapterError::Parquet(error.to_string()))?
                .num_rows() as u64;
        }
        (schema_digest, count)
    } else {
        (file_digest.clone(), 1)
    };
    Ok(ManifestEntry {
        relative_path: path
            .strip_prefix(root)
            .unwrap_or(relative)
            .to_string_lossy()
            .replace('\\', "/"),
        table: table.to_owned(),
        file_digest,
        header_digest,
        row_count,
    })
}

#[derive(Debug)]
struct RawCase {
    internal_case_id: String,
    opened_at: String,
    opened_at_unix_micros: i64,
    channel: String,
    language: String,
    origin: Option<String>,
    topic: String,
    priority: String,
}

#[derive(Debug)]
struct RawToolCall {
    internal_case_id: String,
    internal_call_id: String,
    event_time: String,
    event_time_unix_micros: i64,
    actor_role: String,
    tool_id: String,
    permission_level: String,
    status: String,
    verified: Option<bool>,
    state_change: Option<bool>,
    retry_count: Option<u32>,
    latency_ms: Option<u64>,
}

fn read_cases(path: &Path) -> Result<Vec<RawCase>, AdapterError> {
    let file = File::open(path).map_err(|source| AdapterError::ReadFile {
        path: "datos/case.parquet".to_owned(),
        source,
    })?;
    let mut reader = ParquetRecordBatchReaderBuilder::try_new(file)
        .map_err(|error| AdapterError::Parquet(error.to_string()))?
        .build()
        .map_err(|error| AdapterError::Parquet(error.to_string()))?;
    let mut cases = Vec::new();
    while let Some(batch) = reader.next() {
        let batch = batch.map_err(|error| AdapterError::Parquet(error.to_string()))?;
        let ids = required_strings(&batch, "case_id")?;
        let channel = required_strings(&batch, "channel")?;
        let language = required_strings(&batch, "language")?;
        let topic = required_strings(&batch, "topic")?;
        let priority = required_strings(&batch, "priority")?;
        let time_index = column_index(batch.schema(), "opened_at")?;
        let times = batch.column(time_index);
        for row in 0..batch.num_rows() {
            let (opened_at, opened_at_unix_micros) = timestamp_value(times.as_ref(), row)?;
            cases.push(RawCase {
                internal_case_id: ids.value(row).to_owned(),
                opened_at,
                opened_at_unix_micros,
                channel: channel.value(row).to_owned(),
                language: language.value(row).to_owned(),
                origin: optional_string(&batch, "origin", row)?,
                topic: topic.value(row).to_owned(),
                priority: priority.value(row).to_owned(),
            });
        }
    }
    if cases.is_empty() {
        return Err(AdapterError::InvalidInput("E0 case table is empty"));
    }
    Ok(cases)
}

fn read_tool_calls(path: &Path) -> Result<Vec<RawToolCall>, AdapterError> {
    let file = File::open(path).map_err(|source| AdapterError::ReadFile {
        path: "datos/tool_call.parquet".to_owned(),
        source,
    })?;
    let mut reader = ParquetRecordBatchReaderBuilder::try_new(file)
        .map_err(|error| AdapterError::Parquet(error.to_string()))?
        .build()
        .map_err(|error| AdapterError::Parquet(error.to_string()))?;
    let mut calls = Vec::new();
    while let Some(batch) = reader.next() {
        let batch = batch.map_err(|error| AdapterError::Parquet(error.to_string()))?;
        let ids = required_strings(&batch, "case_id")?;
        let call_ids = required_strings(&batch, "call_id")?;
        let status = required_strings(&batch, "status")?;
        let actor_roles = required_strings(&batch, "actor_role")?;
        let tools = required_strings(&batch, "tool_id")?;
        let permissions = required_strings(&batch, "permission_level")?;
        let time_index = column_index(batch.schema(), "event_time")?;
        let times = batch.column(time_index);
        for row in 0..batch.num_rows() {
            let status = status.value(row);
            if !matches!(status, "ok" | "error" | "timeout" | "denied") {
                return Err(AdapterError::InvalidInput(
                    "tool_call status is outside the E0 contract",
                ));
            }
            let (event_time, event_time_unix_micros) = timestamp_value(times.as_ref(), row)?;
            calls.push(RawToolCall {
                internal_case_id: ids.value(row).to_owned(),
                internal_call_id: call_ids.value(row).to_owned(),
                event_time,
                event_time_unix_micros,
                actor_role: safe_domain(
                    actor_roles.value(row),
                    &["tree", "judge", "ai_agent", "analyst"],
                ),
                tool_id: opaque_category(tools.value(row)),
                permission_level: safe_domain(
                    permissions.value(row),
                    &["read", "confirm", "human_only"],
                ),
                status: status.to_owned(),
                verified: optional_bool(&batch, "verified", row)?,
                state_change: optional_presence(&batch, "state_change", row)?,
                retry_count: optional_u64(&batch, "retry_count", row)?.map(|v| v as u32),
                latency_ms: optional_u64(&batch, "latency_ms", row)?,
            });
        }
    }
    Ok(calls)
}

fn read_other_facts(
    data_dir: &Path,
    ordinal_by_case_id: &BTreeMap<String, u32>,
    tool_ordinal_by_id: &BTreeMap<String, (u32, u32)>,
) -> Result<Vec<E0Fact>, AdapterError> {
    let mut facts = Vec::new();
    read_identity_checks(data_dir, ordinal_by_case_id, &mut facts)?;
    read_turns(data_dir, ordinal_by_case_id, &mut facts)?;
    read_routing_steps(data_dir, ordinal_by_case_id, &mut facts)?;
    read_copilot_queries(data_dir, ordinal_by_case_id, &mut facts)?;
    read_approvals(data_dir, ordinal_by_case_id, tool_ordinal_by_id, &mut facts)?;
    read_signals(data_dir, &mut facts)?;
    facts.sort_by(fact_order);
    Ok(facts)
}

fn events_for_case(case_ordinal: u32, facts: &[E0Fact]) -> Result<Vec<SafeEvent>, AdapterError> {
    let mut candidates = Vec::new();
    for fact in facts {
        let (
            kind,
            source_ordinal,
            time,
            exact_micros,
            route,
            actor,
            tool,
            technical_error,
            approval,
            parent_source_ordinal,
        ) = match fact {
            E0Fact::IdentityCheck {
                case_ordinal: c,
                event_ordinal,
                event_time,
                event_time_unix_micros,
                actor_role,
                result,
                ..
            } if *c == case_ordinal => (
                "identity_check",
                *event_ordinal,
                event_time.as_str(),
                Some(*event_time_unix_micros),
                Some(result.as_str()),
                Some(actor_role.as_str()),
                None,
                None,
                None,
                None,
            ),
            E0Fact::Turn {
                case_ordinal: c,
                event_ordinal,
                event_time,
                event_time_unix_micros,
                author_role,
                ..
            } if *c == case_ordinal => (
                "turn",
                *event_ordinal,
                event_time.as_str(),
                Some(*event_time_unix_micros),
                None,
                Some(author_role.as_str()),
                None,
                None,
                None,
                None,
            ),
            E0Fact::RoutingStep {
                case_ordinal: c,
                event_ordinal,
                event_time,
                event_time_unix_micros,
                tier,
                outcome,
                ..
            } if *c == case_ordinal => (
                "routing_step",
                *event_ordinal,
                event_time.as_str(),
                Some(*event_time_unix_micros),
                Some(tier.as_str()),
                None,
                Some(outcome.as_str()),
                None,
                None,
                None,
            ),
            E0Fact::CopilotQuery {
                case_ordinal: c,
                event_ordinal,
                event_time,
                event_time_unix_micros,
                answered_by,
                ..
            } if *c == case_ordinal => (
                "copilot_query",
                *event_ordinal,
                event_time.as_str(),
                Some(*event_time_unix_micros),
                None,
                Some(answered_by.as_str()),
                None,
                None,
                None,
                None,
            ),
            E0Fact::ToolCall {
                case_ordinal: c,
                event_ordinal,
                event_time,
                event_time_unix_micros,
                actor_role,
                tool_id,
                technical_error,
                ..
            } if *c == case_ordinal => (
                "tool_call",
                *event_ordinal,
                event_time.as_str(),
                Some(*event_time_unix_micros),
                None,
                Some(actor_role.as_str()),
                Some(tool_id.as_str()),
                Some(*technical_error),
                None,
                None,
            ),
            E0Fact::Approval {
                case_ordinal: c,
                event_ordinal,
                requested_at,
                requested_at_unix_micros,
                requested_by_role,
                tool_id,
                decision,
                related_tool_ordinal,
                ..
            } if *c == case_ordinal => (
                "approval",
                *event_ordinal,
                requested_at.as_str(),
                Some(*requested_at_unix_micros),
                None,
                Some(requested_by_role.as_str()),
                Some(tool_id.as_str()),
                None,
                decision
                    .as_deref()
                    .map(|d| d.eq_ignore_ascii_case("approved")),
                *related_tool_ordinal,
            ),
            _ => continue,
        };
        let parsed = parse_utc_timestamp(time)?;
        candidates.push((
            parsed,
            kind,
            source_ordinal,
            exact_micros.unwrap_or(parsed * 1_000_000),
            route.map(str::to_owned),
            actor.map(str::to_owned),
            tool.map(str::to_owned),
            technical_error,
            approval,
            parent_source_ordinal,
        ));
    }
    candidates.sort_by_key(|entry| (entry.3, entry.1, entry.2));
    let ordinal_by_source: BTreeMap<(&str, u32), u32> = candidates
        .iter()
        .enumerate()
        .map(|(index, entry)| ((entry.1, entry.2), index as u32 + 1))
        .collect();
    let approval_by_tool_source: BTreeMap<u32, u32> = candidates
        .iter()
        .filter(|entry| entry.1 == "approval")
        .filter_map(|entry| entry.9.map(|tool_source| (tool_source, entry.2)))
        .collect();
    Ok(candidates
        .into_iter()
        .enumerate()
        .map(|(index, entry)| SafeEvent {
            case_ordinal,
            event_ordinal: index as u32 + 1,
            // An approval authorizes the linked tool call, so represent the
            // causal edge on the later tool event (child -> prior approval).
            parent_event_ordinal: (entry.1 == "tool_call")
                .then(|| approval_by_tool_source.get(&entry.2).copied())
                .flatten()
                .and_then(|approval_source| {
                    ordinal_by_source
                        .get(&("approval", approval_source))
                        .copied()
                }),
            event_time: format_unix_micros(entry.3),
            event_time_unix_micros: entry.3,
            event_kind: entry.1.to_owned(),
            route_code: entry.4,
            actor_role: entry.5,
            tool_code: entry.6,
            technical_error: entry.7,
            approval: entry.8,
            signal_code: None,
        })
        .collect())
}

fn read_identity_checks(
    data_dir: &Path,
    ordinals: &BTreeMap<String, u32>,
    output: &mut Vec<E0Fact>,
) -> Result<(), AdapterError> {
    for batch in parquet_batches_if_present(data_dir, "identity_check")? {
        let checks = required_strings(&batch, "check_id")?;
        let case_ids = required_strings(&batch, "case_id")?;
        let roles = required_strings(&batch, "actor_role")?;
        let results = required_strings(&batch, "result")?;
        let time_index = column_index(batch.schema(), "started_at")?;
        for row in 0..batch.num_rows() {
            let case_ordinal = case_ordinal(ordinals, case_ids.value(row))?;
            let (event_time, event_time_unix_micros) =
                timestamp_value(batch.column(time_index).as_ref(), row)?;
            output.push(E0Fact::IdentityCheck {
                case_ordinal,
                event_ordinal: stable_event_ordinal(checks.value(row), row),
                event_time,
                event_time_unix_micros,
                actor_role: safe_domain(roles.value(row), &["analyst", "ai_agent"]),
                result: safe_domain(results.value(row), &["verified", "failed"]),
                correct: optional_u64(&batch, "correct", row)?,
                attempt: optional_u64(&batch, "attempt", row)?.map(|v| v as u32),
                trigger: optional_string(&batch, "trigger", row)?
                    .map(|v| safe_domain(&v, &["abono", "cambio_de_datos", "canal_sin_identidad"])),
                policy_rule_id: optional_string(&batch, "policy_rule_id", row)?
                    .map(|v| safe_domain(&v, &["R1"])),
            });
        }
    }
    Ok(())
}

fn read_turns(
    data_dir: &Path,
    ordinals: &BTreeMap<String, u32>,
    output: &mut Vec<E0Fact>,
) -> Result<(), AdapterError> {
    for batch in parquet_batches_if_present(data_dir, "turn")? {
        let ids = required_strings(&batch, "turn_id")?;
        let case_ids = required_strings(&batch, "case_id")?;
        let roles = required_strings(&batch, "author_role")?;
        let languages = required_strings(&batch, "language")?;
        let time_index = column_index(batch.schema(), "event_time")?;
        for row in 0..batch.num_rows() {
            let (event_time, event_time_unix_micros) =
                timestamp_value(batch.column(time_index).as_ref(), row)?;
            output.push(E0Fact::Turn {
                case_ordinal: case_ordinal(ordinals, case_ids.value(row))?,
                event_ordinal: stable_event_ordinal(ids.value(row), row),
                event_time,
                event_time_unix_micros,
                author_role: safe_domain(roles.value(row), &["customer", "analyst", "ai_agent"]),
                language: safe_domain(languages.value(row), &["es", "pt"]),
            });
        }
    }
    Ok(())
}

fn read_routing_steps(
    data_dir: &Path,
    ordinals: &BTreeMap<String, u32>,
    output: &mut Vec<E0Fact>,
) -> Result<(), AdapterError> {
    for batch in parquet_batches_if_present(data_dir, "routing_step")? {
        let ids = required_strings(&batch, "step_id")?;
        let case_ids = required_strings(&batch, "case_id")?;
        let tiers = required_strings(&batch, "tier")?;
        let outcomes = required_strings(&batch, "outcome")?;
        let time_index = column_index(batch.schema(), "event_time")?;
        for row in 0..batch.num_rows() {
            let (event_time, event_time_unix_micros) =
                timestamp_value(batch.column(time_index).as_ref(), row)?;
            output.push(E0Fact::RoutingStep {
                case_ordinal: case_ordinal(ordinals, case_ids.value(row))?,
                event_ordinal: stable_event_ordinal(ids.value(row), row),
                event_time,
                event_time_unix_micros,
                tier: safe_domain(
                    tiers.value(row),
                    &["tree", "judge", "ai_agent", "human", "supervisor"],
                ),
                outcome: safe_domain(
                    outcomes.value(row),
                    &["resolved", "mitigated", "handed_off", "abstained"],
                ),
                reason_code: optional_string(&batch, "reason_code", row)?
                    .map(|v| opaque_category(&v)),
                confidence: optional_number_text(&batch, "confidence", row)?,
                handoff: optional_bool(&batch, "handoff", row)?,
            });
        }
    }
    Ok(())
}

fn read_copilot_queries(
    data_dir: &Path,
    ordinals: &BTreeMap<String, u32>,
    output: &mut Vec<E0Fact>,
) -> Result<(), AdapterError> {
    for batch in parquet_batches_if_present(data_dir, "copilot_query")? {
        let ids = required_strings(&batch, "query_id")?;
        let case_ids = required_strings(&batch, "case_id")?;
        let signatures = required_strings(&batch, "query_signature")?;
        let answers = required_strings(&batch, "answered_by")?;
        let time_index = column_index(batch.schema(), "event_time")?;
        for row in 0..batch.num_rows() {
            let (event_time, event_time_unix_micros) =
                timestamp_value(batch.column(time_index).as_ref(), row)?;
            output.push(E0Fact::CopilotQuery {
                case_ordinal: case_ordinal(ordinals, case_ids.value(row))?,
                event_ordinal: stable_event_ordinal(ids.value(row), row),
                event_time,
                event_time_unix_micros,
                query_signature: opaque_category(signatures.value(row)),
                tables_read: optional_string_list(&batch, "tables_read", row)?
                    .iter()
                    .map(|v| opaque_category(v))
                    .collect(),
                columns_read: optional_string_list(&batch, "columns_read", row)?
                    .iter()
                    .map(|v| opaque_category(v))
                    .collect(),
                answered_by: match answers.value(row) {
                    "freeform" => "freeform".to_owned(),
                    value if value.starts_with("tool:") => "tool".to_owned(),
                    _ => "unknown".to_owned(),
                },
                sent_to_chat: optional_bool(&batch, "sent_to_chat", row)?.unwrap_or(false),
            });
        }
    }
    Ok(())
}

fn read_approvals(
    data_dir: &Path,
    ordinals: &BTreeMap<String, u32>,
    calls: &BTreeMap<String, (u32, u32)>,
    output: &mut Vec<E0Fact>,
) -> Result<(), AdapterError> {
    for batch in parquet_batches_if_present(data_dir, "approval")? {
        let ids = required_strings(&batch, "approval_id")?;
        let case_ids = required_strings(&batch, "case_id")?;
        let roles = required_strings(&batch, "requested_by_role")?;
        let tools = required_strings(&batch, "tool_id")?;
        let time_index = column_index(batch.schema(), "requested_at")?;
        for row in 0..batch.num_rows() {
            let (requested_at, requested_at_unix_micros) =
                timestamp_value(batch.column(time_index).as_ref(), row)?;
            let executed_call = optional_string(&batch, "executed_call_id", row)?;
            let case_ordinal = case_ordinal(ordinals, case_ids.value(row))?;
            let decided_at = optional_timestamp_with_micros(&batch, "decided_at", row)?;
            output.push(E0Fact::Approval {
                case_ordinal,
                event_ordinal: stable_event_ordinal(ids.value(row), row),
                requested_at,
                requested_at_unix_micros,
                requested_by_role: safe_domain(roles.value(row), &["analyst", "ai_agent"]),
                tool_id: opaque_category(tools.value(row)),
                reason_code: optional_string(&batch, "reason_code", row)?.map(|v| {
                    safe_domain(
                        &v,
                        &[
                            "over_role_limit",
                            "outside_policy_window",
                            "regulator_complaint",
                        ],
                    )
                }),
                policy_rule_id: optional_string(&batch, "policy_rule_id", row)?
                    .map(|v| opaque_category(&v)),
                decided_at: decided_at.as_ref().map(|value| value.0.clone()),
                decided_at_unix_micros: decided_at.map(|value| value.1),
                decision: optional_string(&batch, "decision", row)?
                    .map(|v| safe_domain(&v, &["approved", "rejected", "expired"])),
                related_tool_ordinal: executed_call
                    .and_then(|call| calls.get(&call))
                    .filter(|(call_case, _)| *call_case == case_ordinal)
                    .map(|(_, ordinal)| *ordinal),
            });
        }
    }
    Ok(())
}

fn read_signals(data_dir: &Path, output: &mut Vec<E0Fact>) -> Result<(), AdapterError> {
    for batch in parquet_batches_if_present(data_dir, "signal")? {
        let ids = required_strings(&batch, "signal_id")?;
        let kinds = required_strings(&batch, "kind")?;
        let scopes = required_strings(&batch, "scope")?;
        let statuses = required_strings(&batch, "status")?;
        let start_index = column_index(batch.schema(), "window_start")?;
        let end_index = column_index(batch.schema(), "window_end")?;
        for row in 0..batch.num_rows() {
            let (window_end, window_end_unix_micros) =
                timestamp_value(batch.column(end_index).as_ref(), row)?;
            output.push(E0Fact::Signal {
                event_ordinal: stable_event_ordinal(ids.value(row), row),
                kind: safe_domain(
                    kinds.value(row),
                    &[
                        "repeated_query",
                        "consistent_sequence",
                        "high_acceptance",
                        "drift",
                    ],
                ),
                scope: opaque_category(scopes.value(row)),
                window_start: timestamp_value(batch.column(start_index).as_ref(), row)?.0,
                window_end,
                window_end_unix_micros,
                support_cases: optional_u64(&batch, "support_cases", row)?.unwrap_or(0) as u32,
                support_analysts: optional_u64(&batch, "support_analysts", row)?.unwrap_or(0)
                    as u32,
                consistency: optional_number_text(&batch, "consistency", row)?,
                status: safe_domain(
                    statuses.value(row),
                    &["new", "accepted", "in_build", "discarded"],
                ),
            });
        }
    }
    Ok(())
}

fn parquet_batches_if_present(
    data_dir: &Path,
    table: &str,
) -> Result<Vec<RecordBatch>, AdapterError> {
    let path = data_dir.join(format!("{table}.parquet"));
    if !path.exists() {
        return Ok(Vec::new());
    }
    let file = File::open(&path).map_err(|source| AdapterError::ReadFile {
        path: format!("datos/{table}.parquet"),
        source,
    })?;
    let reader = ParquetRecordBatchReaderBuilder::try_new(file)
        .map_err(|error| AdapterError::Parquet(error.to_string()))?
        .build()
        .map_err(|error| AdapterError::Parquet(error.to_string()))?;
    reader
        .map(|batch| batch.map_err(|error| AdapterError::Parquet(error.to_string())))
        .collect()
}

enum StringColumn<'a> {
    Utf8(&'a StringArray),
    LargeUtf8(&'a LargeStringArray),
}

impl StringColumn<'_> {
    fn value(&self, index: usize) -> &str {
        match self {
            Self::Utf8(values) => values.value(index),
            Self::LargeUtf8(values) => values.value(index),
        }
    }

    fn is_null(&self, index: usize) -> bool {
        match self {
            Self::Utf8(values) => values.is_null(index),
            Self::LargeUtf8(values) => values.is_null(index),
        }
    }

    fn null_count(&self) -> usize {
        match self {
            Self::Utf8(values) => values.null_count(),
            Self::LargeUtf8(values) => values.null_count(),
        }
    }
}

fn string_column<'a>(
    batch: &'a RecordBatch,
    column: &str,
) -> Result<StringColumn<'a>, AdapterError> {
    let index = column_index(batch.schema(), column)?;
    let array = batch.column(index);
    if let Some(values) = array.as_any().downcast_ref::<StringArray>() {
        Ok(StringColumn::Utf8(values))
    } else if let Some(values) = array.as_any().downcast_ref::<LargeStringArray>() {
        Ok(StringColumn::LargeUtf8(values))
    } else {
        Err(AdapterError::UnsupportedSchema(format!(
            "column {column} must be Utf8 or LargeUtf8 text, found {:?}",
            array.data_type()
        )))
    }
}

fn required_strings<'a>(
    batch: &'a RecordBatch,
    column: &str,
) -> Result<StringColumn<'a>, AdapterError> {
    let values = string_column(batch, column)?;
    if values.null_count() != 0 {
        return Err(AdapterError::InvalidInput(
            "required E0 text column contains nulls",
        ));
    }
    Ok(values)
}

fn optional_string(
    batch: &RecordBatch,
    column: &str,
    row: usize,
) -> Result<Option<String>, AdapterError> {
    let index = match batch.schema().index_of(column) {
        Ok(index) => index,
        Err(_) => return Ok(None),
    };
    if batch.column(index).data_type() == &DataType::Null {
        return Ok(None);
    }
    let values = string_column(batch, column)?;
    Ok((!values.is_null(row)).then(|| values.value(row).to_owned()))
}

fn optional_bool(
    batch: &RecordBatch,
    column: &str,
    row: usize,
) -> Result<Option<bool>, AdapterError> {
    let index = match batch.schema().index_of(column) {
        Ok(index) => index,
        Err(_) => return Ok(None),
    };
    let array = batch.column(index);
    if array.data_type() == &DataType::Null {
        return Ok(None);
    }
    let Some(values) = array.as_any().downcast_ref::<arrow_array::BooleanArray>() else {
        return Err(AdapterError::UnsupportedSchema(format!(
            "optional column {column} must be boolean"
        )));
    };
    Ok((!values.is_null(row)).then(|| values.value(row)))
}

fn optional_presence(
    batch: &RecordBatch,
    column: &str,
    row: usize,
) -> Result<Option<bool>, AdapterError> {
    let index = match batch.schema().index_of(column) {
        Ok(index) => index,
        Err(_) => return Ok(None),
    };
    let array = batch.column(index);
    if array.data_type() == &DataType::Null {
        return Ok(None);
    }
    Ok(Some(!array.is_null(row)))
}

fn optional_u64(
    batch: &RecordBatch,
    column: &str,
    row: usize,
) -> Result<Option<u64>, AdapterError> {
    let index = match batch.schema().index_of(column) {
        Ok(index) => index,
        Err(_) => return Ok(None),
    };
    let array = batch.column(index);
    if array.data_type() == &DataType::Null {
        return Ok(None);
    }
    if array.is_null(row) {
        return Ok(None);
    }
    match array.data_type() {
        DataType::Int32 => {
            let value = require_column::<Int32Array>(batch, column)?.value(row);
            u64::try_from(value)
                .map(Some)
                .map_err(|_| AdapterError::InvalidInput("optional E0 count cannot be negative"))
        }
        DataType::Int64 => {
            let value = require_column::<Int64Array>(batch, column)?.value(row);
            u64::try_from(value)
                .map(Some)
                .map_err(|_| AdapterError::InvalidInput("optional E0 count cannot be negative"))
        }
        other => Err(AdapterError::UnsupportedSchema(format!(
            "optional column {column} has unsupported integer type {other:?}"
        ))),
    }
}

fn optional_number_text(
    batch: &RecordBatch,
    column: &str,
    row: usize,
) -> Result<Option<String>, AdapterError> {
    let index = match batch.schema().index_of(column) {
        Ok(index) => index,
        Err(_) => return Ok(None),
    };
    let array = batch.column(index);
    if array.data_type() == &DataType::Null {
        return Ok(None);
    }
    if array.is_null(row) {
        return Ok(None);
    }
    match array.data_type() {
        DataType::Float64 => Ok(Some(
            require_column::<arrow_array::Float64Array>(batch, column)?
                .value(row)
                .to_string(),
        )),
        DataType::Float32 => Ok(Some(
            require_column::<arrow_array::Float32Array>(batch, column)?
                .value(row)
                .to_string(),
        )),
        DataType::Int32 => Ok(Some(
            require_column::<Int32Array>(batch, column)?
                .value(row)
                .to_string(),
        )),
        DataType::Int64 => Ok(Some(
            require_column::<Int64Array>(batch, column)?
                .value(row)
                .to_string(),
        )),
        DataType::Utf8 | DataType::LargeUtf8 => {
            Ok(Some(string_column(batch, column)?.value(row).to_owned()))
        }
        other => Err(AdapterError::UnsupportedSchema(format!(
            "optional metric column {column} has unsupported type {other:?}"
        ))),
    }
}

fn optional_string_list(
    batch: &RecordBatch,
    column: &str,
    row: usize,
) -> Result<Vec<String>, AdapterError> {
    let index = match batch.schema().index_of(column) {
        Ok(index) => index,
        Err(_) => return Ok(Vec::new()),
    };
    let array = batch.column(index);
    if array.data_type() == &DataType::Null {
        return Ok(Vec::new());
    }
    if array.is_null(row) {
        return Ok(Vec::new());
    }
    if matches!(array.data_type(), DataType::Utf8 | DataType::LargeUtf8) {
        let encoded_column = string_column(batch, column)?;
        let encoded = encoded_column.value(row);
        if encoded.len() > 16_384 {
            return Err(AdapterError::UnsupportedSchema(format!(
                "optional column {column} JSON array exceeds size limit"
            )));
        }
        let items = serde_json::from_str::<Vec<String>>(encoded).map_err(|_| {
            AdapterError::UnsupportedSchema(format!(
                "optional column {column} must contain a JSON array of strings"
            ))
        })?;
        if items.len() > 128 || items.iter().any(|item| item.len() > 256) {
            return Err(AdapterError::UnsupportedSchema(format!(
                "optional column {column} JSON array exceeds item limits"
            )));
        }
        return Ok(items);
    }
    let values = if let Some(list) = array.as_any().downcast_ref::<arrow_array::ListArray>() {
        list.value(row)
    } else if let Some(list) = array.as_any().downcast_ref::<arrow_array::LargeListArray>() {
        list.value(row)
    } else {
        return Err(AdapterError::UnsupportedSchema(format!(
            "optional column {column} must be a list of strings, found {:?}",
            array.data_type()
        )));
    };
    let strings = if let Some(strings) = values.as_any().downcast_ref::<StringArray>() {
        StringColumn::Utf8(strings)
    } else if let Some(strings) = values.as_any().downcast_ref::<LargeStringArray>() {
        StringColumn::LargeUtf8(strings)
    } else {
        return Err(AdapterError::UnsupportedSchema(format!(
            "optional column {column} must contain Utf8 or LargeUtf8 values"
        )));
    };
    Ok((0..values.len())
        .filter(|index| !strings.is_null(*index))
        .map(|index| strings.value(index).to_owned())
        .collect())
}

fn optional_timestamp_with_micros(
    batch: &RecordBatch,
    column: &str,
    row: usize,
) -> Result<Option<(String, i64)>, AdapterError> {
    let index = match batch.schema().index_of(column) {
        Ok(index) => index,
        Err(_) => return Ok(None),
    };
    if batch.column(index).is_null(row) {
        return Ok(None);
    }
    Ok(Some(timestamp_value(batch.column(index).as_ref(), row)?))
}

fn case_ordinal(ordinals: &BTreeMap<String, u32>, id: &str) -> Result<u32, AdapterError> {
    ordinals.get(id).copied().ok_or(AdapterError::InvalidInput(
        "E0 history row references unknown case",
    ))
}

fn safe_domain(value: &str, allowed: &[&str]) -> String {
    if allowed.contains(&value) {
        value.to_owned()
    } else {
        "unknown".to_owned()
    }
}

fn safe_optional_domain(value: &str, allowed: &[&str]) -> Option<String> {
    allowed.contains(&value).then(|| value.to_owned())
}

fn opaque_category(value: &str) -> String {
    // Stable pseudonymous category for open vocabularies. Do not expose the
    // source token itself in serialized discovery facts.
    let digest = digest(value.as_bytes());
    format!("sha256_{}", &digest[7..63])
}

fn stable_event_ordinal(_id: &str, row: usize) -> u32 {
    // Use the deterministic row ordinal as an opaque per-table reference; the
    // raw source identifier is never copied into an agent-visible fact.
    u32::try_from(row.saturating_add(1)).unwrap_or(u32::MAX)
}

fn fact_order(left: &E0Fact, right: &E0Fact) -> std::cmp::Ordering {
    fact_key(left).cmp(&fact_key(right))
}

fn fact_case_ordinal(fact: &E0Fact) -> Option<u32> {
    match fact {
        E0Fact::Case { case_ordinal, .. }
        | E0Fact::IdentityCheck { case_ordinal, .. }
        | E0Fact::Turn { case_ordinal, .. }
        | E0Fact::RoutingStep { case_ordinal, .. }
        | E0Fact::CopilotQuery { case_ordinal, .. }
        | E0Fact::ToolCall { case_ordinal, .. }
        | E0Fact::Approval { case_ordinal, .. } => Some(*case_ordinal),
        E0Fact::Signal { .. } => None,
    }
}

fn fact_event_unix_micros(fact: &E0Fact) -> Option<i64> {
    let micros = match fact {
        E0Fact::Case {
            opened_at_unix_micros,
            ..
        } => *opened_at_unix_micros,
        E0Fact::IdentityCheck {
            event_time_unix_micros,
            ..
        }
        | E0Fact::Turn {
            event_time_unix_micros,
            ..
        }
        | E0Fact::RoutingStep {
            event_time_unix_micros,
            ..
        }
        | E0Fact::CopilotQuery {
            event_time_unix_micros,
            ..
        }
        | E0Fact::ToolCall {
            event_time_unix_micros,
            ..
        } => *event_time_unix_micros,
        E0Fact::Approval {
            requested_at_unix_micros,
            ..
        } => *requested_at_unix_micros,
        E0Fact::Signal {
            window_end_unix_micros,
            ..
        } => *window_end_unix_micros,
    };
    Some(micros)
}

fn fact_key(fact: &E0Fact) -> (u32, &'static str, u32, &str) {
    match fact {
        E0Fact::Case {
            case_ordinal,
            opened_at,
            ..
        } => (*case_ordinal, "case", 0, opened_at),
        E0Fact::IdentityCheck {
            case_ordinal,
            event_ordinal,
            event_time,
            ..
        } => (*case_ordinal, "identity_check", *event_ordinal, event_time),
        E0Fact::Turn {
            case_ordinal,
            event_ordinal,
            event_time,
            ..
        } => (*case_ordinal, "turn", *event_ordinal, event_time),
        E0Fact::RoutingStep {
            case_ordinal,
            event_ordinal,
            event_time,
            ..
        } => (*case_ordinal, "routing_step", *event_ordinal, event_time),
        E0Fact::CopilotQuery {
            case_ordinal,
            event_ordinal,
            event_time,
            ..
        } => (*case_ordinal, "copilot_query", *event_ordinal, event_time),
        E0Fact::ToolCall {
            case_ordinal,
            event_ordinal,
            event_time,
            ..
        } => (*case_ordinal, "tool_call", *event_ordinal, event_time),
        E0Fact::Approval {
            case_ordinal,
            event_ordinal,
            requested_at,
            ..
        } => (*case_ordinal, "approval", *event_ordinal, requested_at),
        E0Fact::Signal {
            event_ordinal,
            window_start,
            ..
        } => (0, "signal", *event_ordinal, window_start),
    }
}

fn require_column<'a, T: Array + 'static>(
    batch: &'a RecordBatch,
    name: &str,
) -> Result<&'a T, AdapterError> {
    let index = column_index(batch.schema(), name)?;
    batch
        .column(index)
        .as_any()
        .downcast_ref::<T>()
        .ok_or_else(|| {
            AdapterError::UnsupportedSchema(format!(
                "column {name} has an unexpected physical type"
            ))
        })
}

fn column_index(schema: SchemaRef, name: &str) -> Result<usize, AdapterError> {
    schema
        .index_of(name)
        .map_err(|_| AdapterError::UnsupportedSchema(format!("required column {name} is missing")))
}

fn timestamp_value(array: &dyn Array, row: usize) -> Result<(String, i64), AdapterError> {
    if array.is_null(row) {
        return Err(AdapterError::InvalidInput(
            "required E0 event clock is null",
        ));
    }
    let micros = match array.data_type() {
        DataType::Timestamp(arrow_schema::TimeUnit::Microsecond, _) => array
            .as_any()
            .downcast_ref::<TimestampMicrosecondArray>()
            .map(|values| values.value(row)),
        DataType::Timestamp(arrow_schema::TimeUnit::Millisecond, _) => array
            .as_any()
            .downcast_ref::<TimestampMillisecondArray>()
            .map(|values| values.value(row).saturating_mul(1_000)),
        DataType::Timestamp(arrow_schema::TimeUnit::Nanosecond, _) => array
            .as_any()
            .downcast_ref::<TimestampNanosecondArray>()
            .map(|values| values.value(row).div_euclid(1_000)),
        DataType::Timestamp(arrow_schema::TimeUnit::Second, _) => array
            .as_any()
            .downcast_ref::<TimestampSecondArray>()
            .map(|values| values.value(row).saturating_mul(1_000_000)),
        _ => None,
    }
    .ok_or_else(|| {
        AdapterError::UnsupportedSchema("event clock must be a non-null Arrow timestamp".to_owned())
    })?;
    Ok((format_unix_micros(micros), micros))
}

fn format_unix_micros(value: i64) -> String {
    let seconds = value.div_euclid(1_000_000);
    let micros = value.rem_euclid(1_000_000);
    let days = seconds.div_euclid(86_400);
    let day_seconds = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let hour = day_seconds / 3_600;
    let minute = (day_seconds % 3_600) / 60;
    let second = day_seconds % 60;
    let _ = micros;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

fn civil_from_days(days_since_epoch: i64) -> (i64, i64, i64) {
    let z = days_since_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    (year, month, day)
}

fn digest(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn uuid_v7() -> String {
    let milliseconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let mut bytes = [0_u8; 16];
    for (index, byte) in bytes[..6].iter_mut().enumerate() {
        *byte = (milliseconds >> (40 - index * 8)) as u8;
    }
    let entropy = Sha256::digest(format!("{}:{}", milliseconds, std::process::id()).as_bytes());
    bytes[6] = 0x70 | (entropy[0] & 0x0f);
    bytes[7] = entropy[1];
    bytes[8] = 0x80 | (entropy[2] & 0x3f);
    bytes[9..].copy_from_slice(&entropy[3..10]);
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0],
        bytes[1],
        bytes[2],
        bytes[3],
        bytes[4],
        bytes[5],
        bytes[6],
        bytes[7],
        bytes[8],
        bytes[9],
        bytes[10],
        bytes[11],
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15]
    )
}

fn is_utc_timestamp(value: &str) -> bool {
    parse_utc_timestamp(value).is_ok()
}

fn parse_utc_timestamp(value: &str) -> Result<i64, AdapterError> {
    let invalid =
        || AdapterError::InvalidConfig("timestamp must be a valid RFC3339 UTC instant ending in Z");
    if value.len() < 20 || !value.ends_with('Z') || value.as_bytes().get(10) != Some(&b'T') {
        return Err(invalid());
    }
    let bytes = value.as_bytes();
    let number = |range: std::ops::Range<usize>| -> Result<i64, AdapterError> {
        let part =
            std::str::from_utf8(bytes.get(range).ok_or_else(invalid)?).map_err(|_| invalid())?;
        if part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(invalid());
        }
        part.parse().map_err(|_| invalid())
    };
    if bytes.get(4) != Some(&b'-')
        || bytes.get(7) != Some(&b'-')
        || bytes.get(13) != Some(&b':')
        || bytes.get(16) != Some(&b':')
    {
        return Err(invalid());
    }
    let year = number(0..4)?;
    let month = number(5..7)?;
    let day = number(8..10)?;
    let hour = number(11..13)?;
    let minute = number(14..16)?;
    let second = number(17..19)?;
    if !(1..=12).contains(&month)
        || !(0..=23).contains(&hour)
        || !(0..=59).contains(&minute)
        || !(0..=59).contains(&second)
    {
        return Err(invalid());
    }
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let month_days = match month {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    if day < 1 || day > month_days {
        return Err(invalid());
    }
    let day_of_year = |month: i64| -> i64 {
        match month {
            1 => 0,
            2 => 31,
            3 => 59,
            4 => 90,
            5 => 120,
            6 => 151,
            7 => 181,
            8 => 212,
            9 => 243,
            10 => 273,
            11 => 304,
            _ => 334,
        }
    };
    let adjusted_year = year - i64::from(month <= 2);
    let era = adjusted_year.div_euclid(400);
    let year_of_era = adjusted_year - era * 400;
    let day_of_year_from_march = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let days = era * 146_097 + year_of_era * 365 + year_of_era / 4 - year_of_era / 100
        + day_of_year_from_march
        - 719_468;
    let _ = day_of_year;
    days.checked_mul(86_400)
        .and_then(|base| base.checked_add(hour * 3_600 + minute * 60 + second))
        .filter(|seconds| *seconds >= 0)
        .ok_or_else(invalid)
}
