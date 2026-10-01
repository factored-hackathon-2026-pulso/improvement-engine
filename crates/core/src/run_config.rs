//! Immutable, validated execution boundaries for autonomous detection runs.
//!
//! A `RunConfig` is created at the orchestration boundary and then passed by
//! value or shared immutably. It deliberately has no global default, mutable
//! registry, source reader, persistence, Agent Core dependency, or model call.

use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fmt;

const SUPPORTED_CONFIG_VERSION: u16 = 1;
const MIN_SCHEDULED_CADENCE_MINUTES: u32 = 15;
const MAX_SCHEDULED_CADENCE_MINUTES: u32 = 7 * 24 * 60;
const MAX_SOURCES_PER_RUN: u16 = 100;
const MAX_ROWS_PER_SOURCE: u64 = 5_000_000;
const MAX_BYTES_PER_SOURCE: u64 = 1024 * 1024 * 1024;

/// How an autonomous scan becomes eligible to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cadence {
    EventTriggered,
    Scheduled { every_minutes: u32 },
}

impl Cadence {
    pub fn scheduled_every_minutes(every_minutes: u32) -> Result<Self, RunConfigError> {
        if every_minutes < MIN_SCHEDULED_CADENCE_MINUTES {
            return Err(RunConfigError::CadenceBelowMinimum {
                minimum_minutes: MIN_SCHEDULED_CADENCE_MINUTES,
                actual_minutes: every_minutes,
            });
        }
        if every_minutes > MAX_SCHEDULED_CADENCE_MINUTES {
            return Err(RunConfigError::CadenceExceedsMaximum {
                maximum_minutes: MAX_SCHEDULED_CADENCE_MINUTES,
                actual_minutes: every_minutes,
            });
        }
        Ok(Self::Scheduled { every_minutes })
    }
}

/// Upper limits the worker must enforce while scanning one run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanBudget {
    max_sources: u16,
    max_rows_per_source: u64,
    max_bytes_per_source: u64,
}

impl ScanBudget {
    pub fn new(
        max_sources: u16,
        max_rows_per_source: u64,
        max_bytes_per_source: u64,
    ) -> Result<Self, RunConfigError> {
        validate_nonzero_and_max(
            "max_sources",
            max_sources as u64,
            MAX_SOURCES_PER_RUN as u64,
        )?;
        validate_nonzero_and_max(
            "max_rows_per_source",
            max_rows_per_source,
            MAX_ROWS_PER_SOURCE,
        )?;
        validate_nonzero_and_max(
            "max_bytes_per_source",
            max_bytes_per_source,
            MAX_BYTES_PER_SOURCE,
        )?;

        Ok(Self {
            max_sources,
            max_rows_per_source,
            max_bytes_per_source,
        })
    }

    pub fn max_sources(&self) -> u16 {
        self.max_sources
    }

    pub fn max_rows_per_source(&self) -> u64 {
        self.max_rows_per_source
    }

    pub fn max_bytes_per_source(&self) -> u64 {
        self.max_bytes_per_source
    }
}

/// A source whose immutable contract is permitted for a run.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct EligibleSource {
    source_namespace: String,
    table: String,
}

impl EligibleSource {
    pub fn new(
        source_namespace: impl Into<String>,
        table: impl Into<String>,
    ) -> Result<Self, RunConfigError> {
        let source_namespace = source_namespace.into();
        let table = table.into();
        validate_identifier(&source_namespace, "source_namespace")?;
        validate_identifier(&table, "table")?;
        Ok(Self {
            source_namespace,
            table,
        })
    }

    pub fn source_namespace(&self) -> &str {
        &self.source_namespace
    }

    pub fn table(&self) -> &str {
        &self.table
    }
}

/// Content-addressed identity of the complete immutable run configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunConfigIdentity(String);

impl RunConfigIdentity {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A fully validated configuration that bounds an autonomous detection run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunConfig {
    version: u16,
    cadence: Cadence,
    scan_budget: ScanBudget,
    eligible_sources: BTreeSet<EligibleSource>,
    identity: RunConfigIdentity,
}

impl RunConfig {
    pub fn new(
        version: u16,
        cadence: Cadence,
        scan_budget: ScanBudget,
        eligible_sources: Vec<EligibleSource>,
    ) -> Result<Self, RunConfigError> {
        if version != SUPPORTED_CONFIG_VERSION {
            return Err(RunConfigError::UnsupportedVersion {
                supported: SUPPORTED_CONFIG_VERSION,
                actual: version,
            });
        }
        if eligible_sources.is_empty() {
            return Err(RunConfigError::EmptyEligibleSources);
        }

        let source_count = eligible_sources.len();
        let eligible_sources: BTreeSet<_> = eligible_sources.into_iter().collect();
        if eligible_sources.len() != source_count {
            return Err(RunConfigError::DuplicateEligibleSource);
        }
        if eligible_sources.len() > usize::from(scan_budget.max_sources) {
            return Err(RunConfigError::EligibleSourcesExceedBudget {
                eligible: eligible_sources.len(),
                maximum: scan_budget.max_sources,
            });
        }

        let identity = identity_for(version, &cadence, &scan_budget, &eligible_sources);
        Ok(Self {
            version,
            cadence,
            scan_budget,
            eligible_sources,
            identity,
        })
    }

    pub fn version(&self) -> u16 {
        self.version
    }

    pub fn cadence(&self) -> &Cadence {
        &self.cadence
    }

    pub fn scan_budget(&self) -> &ScanBudget {
        &self.scan_budget
    }

    pub fn eligible_sources(&self) -> impl Iterator<Item = &EligibleSource> {
        self.eligible_sources.iter()
    }

    pub fn allows(&self, source: &EligibleSource) -> Result<(), RunConfigError> {
        if self.eligible_sources.contains(source) {
            Ok(())
        } else {
            Err(RunConfigError::UnsupportedSource {
                source_namespace: source.source_namespace.clone(),
                table: source.table.clone(),
            })
        }
    }

    pub fn identity(&self) -> &RunConfigIdentity {
        &self.identity
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunConfigError {
    UnsupportedVersion {
        supported: u16,
        actual: u16,
    },
    CadenceBelowMinimum {
        minimum_minutes: u32,
        actual_minutes: u32,
    },
    CadenceExceedsMaximum {
        maximum_minutes: u32,
        actual_minutes: u32,
    },
    BudgetMustBePositive {
        field: &'static str,
    },
    BudgetExceedsMaximum {
        field: &'static str,
        maximum: u64,
        actual: u64,
    },
    InvalidSourceIdentifier {
        field: &'static str,
    },
    EmptyEligibleSources,
    DuplicateEligibleSource,
    EligibleSourcesExceedBudget {
        eligible: usize,
        maximum: u16,
    },
    UnsupportedSource {
        source_namespace: String,
        table: String,
    },
}

impl fmt::Display for RunConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedVersion { supported, actual } => write!(
                formatter,
                "unsupported run config version {actual}; supported version is {supported}"
            ),
            Self::CadenceBelowMinimum {
                minimum_minutes,
                actual_minutes,
            } => write!(
                formatter,
                "scheduled cadence {actual_minutes}m is below the {minimum_minutes}m minimum"
            ),
            Self::CadenceExceedsMaximum {
                maximum_minutes,
                actual_minutes,
            } => write!(
                formatter,
                "scheduled cadence {actual_minutes}m exceeds the {maximum_minutes}m maximum"
            ),
            Self::BudgetMustBePositive { field } => write!(formatter, "{field} must be positive"),
            Self::BudgetExceedsMaximum {
                field,
                maximum,
                actual,
            } => {
                write!(formatter, "{field} {actual} exceeds maximum {maximum}")
            }
            Self::InvalidSourceIdentifier { field } => {
                write!(formatter, "{field} must be a lowercase identifier")
            }
            Self::EmptyEligibleSources => {
                formatter.write_str("at least one eligible source is required")
            }
            Self::DuplicateEligibleSource => formatter.write_str("eligible sources must be unique"),
            Self::EligibleSourcesExceedBudget { eligible, maximum } => write!(
                formatter,
                "{eligible} eligible sources exceed scan budget maximum {maximum}"
            ),
            Self::UnsupportedSource {
                source_namespace,
                table,
            } => write!(
                formatter,
                "source {source_namespace}.{table} is not eligible for this run"
            ),
        }
    }
}

impl std::error::Error for RunConfigError {}

fn validate_nonzero_and_max(
    field: &'static str,
    actual: u64,
    maximum: u64,
) -> Result<(), RunConfigError> {
    if actual == 0 {
        return Err(RunConfigError::BudgetMustBePositive { field });
    }
    if actual > maximum {
        return Err(RunConfigError::BudgetExceedsMaximum {
            field,
            maximum,
            actual,
        });
    }
    Ok(())
}

fn validate_identifier(value: &str, field: &'static str) -> Result<(), RunConfigError> {
    let mut characters = value.chars();
    if !matches!(characters.next(), Some(character) if character.is_ascii_lowercase())
        || !characters.all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '_'
        })
    {
        return Err(RunConfigError::InvalidSourceIdentifier { field });
    }
    Ok(())
}

fn identity_for(
    version: u16,
    cadence: &Cadence,
    scan_budget: &ScanBudget,
    eligible_sources: &BTreeSet<EligibleSource>,
) -> RunConfigIdentity {
    let mut canonical = String::new();
    append_atom(&mut canonical, "version", &version.to_string());
    match cadence {
        Cadence::EventTriggered => append_atom(&mut canonical, "cadence", "event_triggered"),
        Cadence::Scheduled { every_minutes } => {
            append_atom(&mut canonical, "cadence", "scheduled");
            append_atom(&mut canonical, "every_minutes", &every_minutes.to_string());
        }
    }
    append_atom(
        &mut canonical,
        "max_sources",
        &scan_budget.max_sources.to_string(),
    );
    append_atom(
        &mut canonical,
        "max_rows_per_source",
        &scan_budget.max_rows_per_source.to_string(),
    );
    append_atom(
        &mut canonical,
        "max_bytes_per_source",
        &scan_budget.max_bytes_per_source.to_string(),
    );
    for source in eligible_sources {
        append_atom(&mut canonical, "source_namespace", &source.source_namespace);
        append_atom(&mut canonical, "table", &source.table);
    }
    RunConfigIdentity(format!(
        "run-config:sha256:{:x}",
        Sha256::digest(canonical.as_bytes())
    ))
}

fn append_atom(target: &mut String, name: &str, value: &str) {
    target.push_str(name);
    target.push(':');
    target.push_str(&value.len().to_string());
    target.push(':');
    target.push_str(value);
    target.push('\n');
}
