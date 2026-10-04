use std::ffi::OsString;
use std::io::{self, Write};
use std::path::PathBuf;

use improvement_engine_source_adapters::{
    PackageValidationError, PreparationConfig, PreparedSource, SourceKind, prepare_e0_package,
    validate_e0_operational_package, validate_original_contact_complaint_profile,
};
use serde::Serialize;

const VALIDATION_TENANT: &str = "pulso_source_validation";
const VALIDATION_CUTOFF: &str = "9999-12-31T23:59:59Z";
const VALIDATION_ARRANQUE_CASES: usize = 1;

#[derive(Debug)]
struct Options {
    kind: SourceKindArg,
    input: PathBuf,
    contract_version: String,
}

#[derive(Clone, Copy, Debug)]
enum SourceKindArg {
    EnrichedHistory,
    OriginalBank,
}

impl SourceKindArg {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "enriched_history" => Some(Self::EnrichedHistory),
            "original_bank" => Some(Self::OriginalBank),
            _ => None,
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::EnrichedHistory => "enriched_history",
            Self::OriginalBank => "original_bank",
        }
    }
}

#[derive(Serialize)]
struct Summary {
    schema_version: u32,
    source_kind: &'static str,
    validation_status: &'static str,
    validation_scope: &'static str,
    contract_version: Option<String>,
    manifest_digest: Option<String>,
    snapshot_digest: Option<String>,
    operational_table_count: Option<usize>,
    validated_table_count: Option<usize>,
    findings: Vec<&'static str>,
}

impl Summary {
    fn unsupported(source_kind: &'static str, finding: &'static str) -> Self {
        Self {
            schema_version: 1,
            source_kind,
            validation_status: "unsupported_source",
            validation_scope: "not_validated",
            contract_version: None,
            manifest_digest: None,
            snapshot_digest: None,
            operational_table_count: None,
            validated_table_count: None,
            findings: vec![finding],
        }
    }

    fn valid(
        source_kind: &'static str,
        contract_version: String,
        validation_scope: &'static str,
        table_count: Option<usize>,
        prepared: &PreparedSource,
    ) -> Self {
        Self::valid_with_digests(
            source_kind,
            contract_version,
            validation_scope,
            table_count,
            Some(prepared.manifest_digest().to_owned()),
            Some(prepared.snapshot_ref().digest.clone()),
        )
    }

    fn bounded_valid(
        source_kind: &'static str,
        contract_version: String,
        validation_scope: &'static str,
        table_count: usize,
    ) -> Self {
        Self {
            schema_version: 1,
            source_kind,
            validation_status: "valid",
            validation_scope,
            contract_version: Some(contract_version),
            manifest_digest: None,
            snapshot_digest: None,
            operational_table_count: None,
            validated_table_count: Some(table_count),
            findings: Vec::new(),
        }
    }

    fn valid_with_digests(
        source_kind: &'static str,
        contract_version: String,
        validation_scope: &'static str,
        table_count: Option<usize>,
        manifest_digest: Option<String>,
        snapshot_digest: Option<String>,
    ) -> Self {
        Self {
            schema_version: 1,
            source_kind,
            validation_status: "valid",
            validation_scope,
            contract_version: Some(contract_version),
            manifest_digest,
            snapshot_digest,
            operational_table_count: table_count,
            validated_table_count: None,
            findings: Vec::new(),
        }
    }

    fn invalid(kind: SourceKindArg, source_kind: &'static str, finding: &'static str) -> Self {
        Self {
            schema_version: 1,
            source_kind,
            validation_status: "invalid_source",
            validation_scope: match kind {
                SourceKindArg::EnrichedHistory => "operational",
                SourceKindArg::OriginalBank => "contacts_and_complaints_header_structure",
            },
            contract_version: None,
            manifest_digest: None,
            snapshot_digest: None,
            operational_table_count: None,
            validated_table_count: None,
            findings: vec![finding],
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ValidationFailure {
    Invalid(&'static str),
}

pub fn run(args: impl IntoIterator<Item = OsString>) -> Result<(), String> {
    let options = match Options::parse(args) {
        Ok(options) => options,
        Err(ParseError::Help) => {
            print_help();
            return Ok(());
        }
        Err(ParseError::InvalidKind) => {
            write_summary(&Summary::unsupported("unknown", "unsupported_source"))?;
            return Err("source validation failed: unsupported_source".to_owned());
        }
        Err(ParseError::InvalidArguments) => {
            return Err("required options: --kind --input --contract-version".to_owned());
        }
    };

    let summary = match validate_source(&options) {
        Ok(summary) => summary,
        Err(ValidationFailure::Invalid(finding)) => {
            Summary::invalid(options.kind, options.kind.label(), finding)
        }
    };
    write_summary(&summary)?;
    if summary.validation_status == "valid" {
        Ok(())
    } else {
        Err(format!(
            "source validation failed: {}",
            summary.validation_status
        ))
    }
}

fn validate_source(options: &Options) -> Result<Summary, ValidationFailure> {
    match options.kind {
        SourceKindArg::EnrichedHistory => {
            let config = PreparationConfig::new(
                VALIDATION_TENANT,
                VALIDATION_CUTOFF,
                VALIDATION_ARRANQUE_CASES,
            )
            .map_err(|_| ValidationFailure::Invalid("source_preparation_failed"))?;
            let validated = validate_e0_operational_package(&options.input)
                .map_err(|error| ValidationFailure::Invalid(validation_finding(&error)))?;
            if validated.contract_version() != options.contract_version {
                return Err(ValidationFailure::Invalid("contract_version_mismatch"));
            }
            // The source adapter reads only the operational discovery scope.
            // `validate_e0_operational_package` also validates the remaining
            // operational tables; neither path opens labels or timeline data.
            let prepared = prepare_e0_package(&options.input, &config)
                .map_err(|_| ValidationFailure::Invalid("source_preparation_failed"))?;
            if prepared.source_kind() != SourceKind::E0 {
                return Err(ValidationFailure::Invalid("source_kind_mismatch"));
            }
            Ok(Summary::valid(
                options.kind.label(),
                validated.contract_version().to_owned(),
                "operational",
                Some(validated.table_count()),
                &prepared,
            ))
        }
        SourceKindArg::OriginalBank => {
            let validated = validate_original_contact_complaint_profile(
                &options.input,
                &options.contract_version,
            )
            .map_err(|_| ValidationFailure::Invalid("contract_or_schema_invalid"))?;
            Ok(Summary::bounded_valid(
                options.kind.label(),
                options.contract_version.clone(),
                "contacts_and_complaints_header_structure",
                validated.table_count(),
            ))
        }
    }
}

fn validation_finding(error: &PackageValidationError) -> &'static str {
    match error {
        PackageValidationError::BrokenRelation(_) => "relationship_invalid",
        PackageValidationError::Cardinality(_) => "cardinality_invalid",
        PackageValidationError::ClockViolation(_) => "event_clock_invalid",
        PackageValidationError::MissingContract
        | PackageValidationError::InvalidContract
        | PackageValidationError::MissingContractTable(_)
        | PackageValidationError::MissingTable(_)
        | PackageValidationError::InvalidTable(_)
        | PackageValidationError::DuplicateColumn { .. }
        | PackageValidationError::MissingColumn { .. }
        | PackageValidationError::WrongColumnType { .. }
        | PackageValidationError::RequiredValueNull { .. } => "contract_or_schema_invalid",
    }
}

fn write_summary(summary: &Summary) -> Result<(), String> {
    let mut stdout = io::stdout().lock();
    serde_json::to_writer(&mut stdout, summary)
        .map_err(|_| "source validation summary serialization failed".to_owned())?;
    stdout
        .write_all(b"\n")
        .map_err(|_| "source validation summary write failed".to_owned())
}

fn print_help() {
    println!(
        "improvement-engine source validate --kind <enriched_history|original_bank> --input <path> --contract-version <version>\nE0 validation covers operational tables. original_bank covers only the versioned contacts+complaints header/row-structure profile, not all 13 source tables or value types."
    );
}

enum ParseError {
    Help,
    InvalidKind,
    InvalidArguments,
}

impl Options {
    fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Self, ParseError> {
        let mut kind = None;
        let mut input = None;
        let mut contract_version = None;
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            if arg == "--help" || arg == "-h" {
                return Err(ParseError::Help);
            }
            let value = args.next().ok_or(ParseError::InvalidArguments)?;
            match arg.to_str() {
                Some("--kind") => {
                    kind = Some(
                        SourceKindArg::parse(&value.to_string_lossy())
                            .ok_or(ParseError::InvalidKind)?,
                    )
                }
                Some("--input") => input = Some(PathBuf::from(value)),
                Some("--contract-version") => {
                    contract_version = Some(value.to_string_lossy().into_owned())
                }
                _ => return Err(ParseError::InvalidArguments),
            }
        }
        let (Some(kind), Some(input), Some(contract_version)) = (kind, input, contract_version)
        else {
            return Err(ParseError::InvalidArguments);
        };
        if input.as_os_str().is_empty() || contract_version.trim().is_empty() {
            return Err(ParseError::InvalidArguments);
        }
        Ok(Self {
            kind,
            input,
            contract_version,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::validation_finding;
    use improvement_engine_source_adapters::PackageValidationError;

    #[test]
    fn validation_findings_classify_every_rejection_without_source_details() {
        let cases = [
            (
                PackageValidationError::BrokenRelation("turn (845)".to_owned()),
                "relationship_invalid",
            ),
            (
                PackageValidationError::Cardinality("turn".to_owned()),
                "cardinality_invalid",
            ),
            (
                PackageValidationError::ClockViolation("turn".to_owned()),
                "event_clock_invalid",
            ),
            (
                PackageValidationError::MissingContract,
                "contract_or_schema_invalid",
            ),
            (
                PackageValidationError::InvalidContract,
                "contract_or_schema_invalid",
            ),
            (
                PackageValidationError::MissingContractTable("turn".to_owned()),
                "contract_or_schema_invalid",
            ),
            (
                PackageValidationError::MissingTable("turn".to_owned()),
                "contract_or_schema_invalid",
            ),
            (
                PackageValidationError::InvalidTable("turn".to_owned()),
                "contract_or_schema_invalid",
            ),
            (
                PackageValidationError::DuplicateColumn {
                    table: "turn".to_owned(),
                    column: "private-column".to_owned(),
                },
                "contract_or_schema_invalid",
            ),
            (
                PackageValidationError::MissingColumn {
                    table: "turn".to_owned(),
                    column: "private-column".to_owned(),
                },
                "contract_or_schema_invalid",
            ),
            (
                PackageValidationError::WrongColumnType {
                    table: "turn".to_owned(),
                    column: "private-column".to_owned(),
                },
                "contract_or_schema_invalid",
            ),
            (
                PackageValidationError::RequiredValueNull {
                    table: "turn".to_owned(),
                    column: "private-column".to_owned(),
                },
                "contract_or_schema_invalid",
            ),
        ];

        for (error, expected) in cases {
            let finding = validation_finding(&error);
            assert_eq!(finding, expected);
            assert!(!finding.contains("turn"));
            assert!(!finding.contains("845"));
            assert!(!finding.contains("private-column"));
        }
    }
}
