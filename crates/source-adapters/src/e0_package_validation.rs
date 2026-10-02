//! Read-only preflight validation of an augmented E0 package.
//!
//! This module validates operational history only. It does not load evaluator
//! labels or timeline rows, and its report contains counts/schema metadata only.
//! `case_close` is checked as a post-contact outcome and `signal` is checked as
//! an operational table; neither is a discovery input.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::path::Path;

use arrow_array::{
    Array, Float32Array, Float64Array, LargeListArray, LargeStringArray, ListArray, StringArray,
    TimestampMicrosecondArray, TimestampMillisecondArray, TimestampNanosecondArray,
    TimestampSecondArray,
};
use arrow_schema::{DataType, SchemaRef};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use serde::Serialize;
use serde_json::Value;

const OPERATIONAL_TABLES: &[&str] = &[
    "case",
    "identity_check",
    "turn",
    "routing_step",
    "copilot_query",
    "tool_call",
    "approval",
    "case_close",
    "signal",
];

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ValidatedE0OperationalPackage {
    contract_version: String,
    tables: BTreeMap<String, ValidatedTableSummary>,
}

impl ValidatedE0OperationalPackage {
    #[must_use]
    pub fn contract_version(&self) -> &str {
        &self.contract_version
    }

    #[must_use]
    pub fn table_count(&self) -> usize {
        self.tables.len()
    }

    #[must_use]
    pub fn row_count(&self, table: &str) -> Option<u64> {
        self.tables.get(table).map(|summary| summary.row_count)
    }

    #[must_use]
    pub fn uncontracted_column_count(&self, table: &str) -> Option<usize> {
        self.tables
            .get(table)
            .map(|summary| summary.uncontracted_column_count)
    }

    /// Metadata only: no source identifiers, field values, or outcome values.
    pub fn table_names(&self) -> impl Iterator<Item = &str> {
        self.tables.keys().map(String::as_str)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct ValidatedTableSummary {
    row_count: u64,
    column_count: usize,
    uncontracted_column_count: usize,
}

/// Validate the E0 operational package without projecting data into discovery.
/// Errors identify a table and a schema/relationship category, never a row value.
pub fn validate_e0_operational_package(
    root: &Path,
) -> Result<ValidatedE0OperationalPackage, PackageValidationError> {
    let contract_path = root.join("contratos/platform_history.json");
    let contract_bytes =
        fs::read(&contract_path).map_err(|_| PackageValidationError::MissingContract)?;
    let contract: Value = serde_json::from_slice(&contract_bytes)
        .map_err(|_| PackageValidationError::InvalidContract)?;
    let entities = contract
        .get("entities")
        .and_then(Value::as_object)
        .ok_or(PackageValidationError::InvalidContract)?;
    let contract_version = contract
        .get("version")
        .and_then(Value::as_str)
        .ok_or(PackageValidationError::InvalidContract)?
        .to_owned();

    let mut tables = BTreeMap::new();
    let mut ids = BTreeMap::<String, BTreeSet<String>>::new();
    let mut cases = BTreeMap::<String, i64>::new();
    let mut close_times = Vec::<(String, i64)>::new();
    let mut operational_events = Vec::<(String, String, i64)>::new();
    let mut identity_intervals = Vec::<(i64, i64)>::new();
    let mut approval_intervals = Vec::<(i64, Option<i64>)>::new();
    let mut approvals = BTreeSet::<String>::new();
    let mut executed_call_ids = Vec::<String>::new();
    let mut tool_approval_ids = Vec::<String>::new();
    let mut turn_evidence_ids = Vec::<String>::new();
    let mut signal_evidence = Vec::<String>::new();

    for table in OPERATIONAL_TABLES {
        let table_contract = entities
            .get(*table)
            .ok_or_else(|| PackageValidationError::MissingContractTable(table.to_string()))?;
        let fields = table_contract
            .get("fields")
            .and_then(Value::as_object)
            .ok_or(PackageValidationError::InvalidContract)?;
        let path = root.join("datos").join(format!("{table}.parquet"));
        let file = File::open(&path)
            .map_err(|_| PackageValidationError::MissingTable(table.to_string()))?;
        let builder = ParquetRecordBatchReaderBuilder::try_new(file)
            .map_err(|_| PackageValidationError::InvalidTable(table.to_string()))?;
        let schema = builder.schema().clone();
        validate_schema(table, fields, &schema)?;
        let reader = builder
            .build()
            .map_err(|_| PackageValidationError::InvalidTable(table.to_string()))?;
        let mut row_count = 0_u64;

        for batch in reader {
            let batch =
                batch.map_err(|_| PackageValidationError::InvalidTable(table.to_string()))?;
            row_count = row_count.saturating_add(batch.num_rows() as u64);
            for (name, definition) in fields {
                if definition
                    .get("required")
                    .and_then(Value::as_bool)
                    .ok_or(PackageValidationError::InvalidContract)?
                {
                    let index = schema.index_of(name).map_err(|_| {
                        PackageValidationError::MissingColumn {
                            table: table.to_string(),
                            column: name.clone(),
                        }
                    })?;
                    let column = batch.column(index);
                    if (0..column.len()).any(|row| column.is_null(row)) {
                        return Err(PackageValidationError::RequiredValueNull {
                            table: table.to_string(),
                            column: name.clone(),
                        });
                    }
                }
                if definition.get("type").and_then(Value::as_str) == Some("INTEGER") {
                    let index = schema.index_of(name).map_err(|_| {
                        PackageValidationError::MissingColumn {
                            table: table.to_string(),
                            column: name.clone(),
                        }
                    })?;
                    validate_integer_storage(table, name, batch.column(index).as_ref())?;
                }
            }

            let key_column = if *table == "case_close" {
                "case_id"
            } else {
                table_id_column(table)
            };
            if *table != "case" {
                if let Some(keys) = string_column(&batch, &schema, key_column, table)? {
                    let seen = ids.entry(table.to_string()).or_default();
                    for key in keys.into_iter().flatten() {
                        if !seen.insert(key) {
                            return Err(PackageValidationError::Cardinality(table.to_string()));
                        }
                    }
                }
            }
            match *table {
                "case" => collect_case_rows(&batch, &schema, &mut cases)?,
                "identity_check" | "turn" | "routing_step" | "copilot_query" | "tool_call"
                | "approval" => {
                    let time_column = event_time_column(table);
                    for (case_id, time) in
                        string_time_pairs(&batch, &schema, "case_id", time_column, table)?
                    {
                        operational_events.push((table.to_string(), case_id, time));
                    }
                    if *table == "approval" {
                        let requested = timestamps(&batch, &schema, "requested_at", table)?;
                        let decided = optional_timestamps(&batch, &schema, "decided_at", table)?;
                        if requested.len() != decided.len() {
                            return Err(PackageValidationError::Cardinality(table.to_string()));
                        }
                        approval_intervals.extend(requested.into_iter().zip(decided));
                        if let Some(values) = string_column(&batch, &schema, "approval_id", table)?
                        {
                            approvals.extend(values.into_iter().flatten());
                        }
                        if let Some(values) =
                            string_column(&batch, &schema, "executed_call_id", table)?
                        {
                            executed_call_ids.extend(values.into_iter().flatten());
                        }
                    }
                    if *table == "tool_call" {
                        if let Some(values) = string_column(&batch, &schema, "approval_id", table)?
                        {
                            tool_approval_ids.extend(values.into_iter().flatten());
                        }
                    }
                    if *table == "identity_check" {
                        let started = timestamps(&batch, &schema, "started_at", table)?;
                        let ended = timestamps(&batch, &schema, "ended_at", table)?;
                        if started.len() != ended.len() {
                            return Err(PackageValidationError::Cardinality(table.to_string()));
                        }
                        identity_intervals.extend(started.into_iter().zip(ended));
                    }
                    if *table == "turn" {
                        turn_evidence_ids.extend(string_list_column(
                            &batch,
                            &schema,
                            "evidence_ids",
                            table,
                        )?);
                    }
                }
                "case_close" => {
                    for (case_id, time) in
                        string_time_pairs(&batch, &schema, "case_id", "closed_at", table)?
                    {
                        close_times.push((case_id, time));
                    }
                }
                "signal" => {
                    let starts = timestamps(&batch, &schema, "window_start", table)?;
                    let ends = timestamps(&batch, &schema, "window_end", table)?;
                    if starts.len() != ends.len() {
                        return Err(PackageValidationError::ClockViolation(table.to_string()));
                    }
                    if starts.iter().zip(&ends).any(|(start, end)| start > end) {
                        return Err(PackageValidationError::ClockViolation(table.to_string()));
                    }
                    signal_evidence.extend(string_list_column(
                        &batch,
                        &schema,
                        "evidence_case_ids",
                        table,
                    )?);
                }
                _ => unreachable!("operational tables are enumerated above"),
            }
        }

        if *table == "case" && row_count == 0 {
            return Err(PackageValidationError::Cardinality(table.to_string()));
        }
        tables.insert(
            table.to_string(),
            ValidatedTableSummary {
                row_count,
                column_count: schema.fields().len(),
                uncontracted_column_count: schema
                    .fields()
                    .iter()
                    .filter(|field| !fields.contains_key(field.name()))
                    .count(),
            },
        );
    }

    for (table, case_id, event_time) in &operational_events {
        let opened_at = cases
            .get(case_id)
            .ok_or_else(|| PackageValidationError::BrokenRelation(table.clone()))?;
        if event_time < opened_at {
            return Err(PackageValidationError::ClockViolation(table.clone()));
        }
    }
    for (case_id, closed_at) in close_times {
        let opened_at = cases
            .get(&case_id)
            .ok_or_else(|| PackageValidationError::BrokenRelation("case_close".to_string()))?;
        if closed_at < *opened_at {
            return Err(PackageValidationError::ClockViolation(
                "case_close".to_string(),
            ));
        }
        if operational_events
            .iter()
            .any(|(_, event_case_id, event_time)| {
                event_case_id == &case_id && *event_time > closed_at
            })
        {
            return Err(PackageValidationError::ClockViolation(
                "case_close".to_string(),
            ));
        }
    }
    if identity_intervals
        .iter()
        .any(|(started, ended)| ended < started)
        || approval_intervals
            .iter()
            .any(|(requested, decided)| decided.is_some_and(|decided| decided < *requested))
    {
        return Err(PackageValidationError::ClockViolation(
            "identity_check/approval".to_string(),
        ));
    }
    if executed_call_ids.iter().any(|call_id| {
        !ids.get("tool_call")
            .is_some_and(|values| values.contains(call_id))
    }) {
        return Err(PackageValidationError::BrokenRelation(
            "approval".to_string(),
        ));
    }
    if tool_approval_ids
        .iter()
        .any(|approval_id| !approvals.contains(approval_id))
    {
        return Err(PackageValidationError::BrokenRelation(
            "tool_call".to_string(),
        ));
    }
    let unresolved_turn_evidence = turn_evidence_ids
        .iter()
        .filter(|id| {
            !["copilot_query", "tool_call", "approval"]
                .iter()
                .any(|table| ids.get(*table).is_some_and(|values| values.contains(*id)))
        })
        .count();
    if unresolved_turn_evidence > 0 {
        return Err(PackageValidationError::BrokenRelation(format!(
            "turn ({unresolved_turn_evidence})"
        )));
    }
    if signal_evidence
        .iter()
        .any(|case_id| !cases.contains_key(case_id))
    {
        return Err(PackageValidationError::BrokenRelation("signal".to_string()));
    }

    Ok(ValidatedE0OperationalPackage {
        contract_version,
        tables,
    })
}

fn validate_schema(
    table: &str,
    contract_fields: &serde_json::Map<String, Value>,
    schema: &SchemaRef,
) -> Result<(), PackageValidationError> {
    let mut seen = BTreeSet::new();
    for field in schema.fields() {
        if !seen.insert(field.name()) {
            return Err(PackageValidationError::DuplicateColumn {
                table: table.to_string(),
                column: field.name().clone(),
            });
        }
    }
    for (name, definition) in contract_fields {
        let Some(field) = schema.field_with_name(name).ok() else {
            return Err(PackageValidationError::MissingColumn {
                table: table.to_string(),
                column: name.clone(),
            });
        };
        let expected = definition
            .get("type")
            .and_then(Value::as_str)
            .ok_or(PackageValidationError::InvalidContract)?;
        if !arrow_type_matches(expected, field.data_type(), field.is_nullable()) {
            return Err(PackageValidationError::WrongColumnType {
                table: table.to_string(),
                column: name.clone(),
            });
        }
        let _required = definition
            .get("required")
            .and_then(Value::as_bool)
            .ok_or(PackageValidationError::InvalidContract)?;
    }
    Ok(())
}

fn arrow_type_matches(contract_type: &str, data_type: &DataType, nullable: bool) -> bool {
    if matches!(data_type, DataType::Null) && nullable {
        return true;
    }
    match contract_type {
        "VARCHAR" | "TEXT" => matches!(data_type, DataType::Utf8 | DataType::LargeUtf8),
        "VARCHAR[]" => match data_type {
            DataType::List(item) | DataType::LargeList(item) => {
                matches!(item.data_type(), DataType::Utf8 | DataType::LargeUtf8)
            }
            DataType::Utf8 | DataType::LargeUtf8 => true,
            _ => false,
        },
        "JSON" => matches!(
            data_type,
            DataType::Utf8
                | DataType::LargeUtf8
                | DataType::Binary
                | DataType::LargeBinary
                | DataType::List(_)
                | DataType::LargeList(_)
                | DataType::Struct(_)
        ),
        "TIMESTAMP" => matches!(data_type, DataType::Timestamp(_, _)),
        "INTEGER" => {
            matches!(
                data_type,
                DataType::Int8
                    | DataType::Int16
                    | DataType::Int32
                    | DataType::Int64
                    | DataType::UInt8
                    | DataType::UInt16
                    | DataType::UInt32
                    | DataType::UInt64
            ) || (nullable && matches!(data_type, DataType::Float32 | DataType::Float64))
        }
        "DOUBLE" => matches!(data_type, DataType::Float32 | DataType::Float64),
        "BOOLEAN" => matches!(data_type, DataType::Boolean),
        _ => false,
    }
}

fn validate_integer_storage(
    table: &str,
    column_name: &str,
    column: &dyn Array,
) -> Result<(), PackageValidationError> {
    let integral = if let Some(values) = column.as_any().downcast_ref::<Float32Array>() {
        (0..values.len()).all(|row| {
            values.is_null(row)
                || (values.value(row).is_finite() && values.value(row).fract() == 0.0)
        })
    } else if let Some(values) = column.as_any().downcast_ref::<Float64Array>() {
        (0..values.len()).all(|row| {
            values.is_null(row)
                || (values.value(row).is_finite() && values.value(row).fract() == 0.0)
        })
    } else {
        true
    };
    if integral {
        Ok(())
    } else {
        Err(PackageValidationError::WrongColumnType {
            table: table.to_string(),
            column: column_name.to_string(),
        })
    }
}

fn collect_case_rows(
    batch: &arrow_array::RecordBatch,
    schema: &SchemaRef,
    cases: &mut BTreeMap<String, i64>,
) -> Result<(), PackageValidationError> {
    let ids = string_column(batch, schema, "case_id", "case")?.unwrap_or_default();
    let opened = timestamps(batch, schema, "opened_at", "case")?;
    if ids.len() != opened.len() {
        return Err(PackageValidationError::Cardinality("case".to_string()));
    }
    for (id, time) in ids.into_iter().zip(opened) {
        let id = id.ok_or_else(|| PackageValidationError::Cardinality("case".to_string()))?;
        if cases.insert(id, time).is_some() {
            return Err(PackageValidationError::Cardinality("case".to_string()));
        }
    }
    Ok(())
}

fn string_time_pairs(
    batch: &arrow_array::RecordBatch,
    schema: &SchemaRef,
    id_column: &str,
    time_column: &str,
    table: &str,
) -> Result<Vec<(String, i64)>, PackageValidationError> {
    let ids = string_column(batch, schema, id_column, table)?.unwrap_or_default();
    let times = timestamps(batch, schema, time_column, table)?;
    if ids.len() != times.len() {
        return Err(PackageValidationError::Cardinality(table.to_string()));
    }
    ids.into_iter()
        .zip(times)
        .map(|(id, time)| {
            id.map(|id| (id, time))
                .ok_or_else(|| PackageValidationError::BrokenRelation(table.to_string()))
        })
        .collect()
}

fn string_column(
    batch: &arrow_array::RecordBatch,
    schema: &SchemaRef,
    name: &str,
    table: &str,
) -> Result<Option<Vec<Option<String>>>, PackageValidationError> {
    let Ok(index) = schema.index_of(name) else {
        return Ok(None);
    };
    let column = batch.column(index);
    if let Some(values) = column.as_any().downcast_ref::<StringArray>() {
        return Ok(Some(
            (0..values.len())
                .map(|row| (!values.is_null(row)).then(|| values.value(row).to_owned()))
                .collect(),
        ));
    }
    if let Some(values) = column.as_any().downcast_ref::<LargeStringArray>() {
        return Ok(Some(
            (0..values.len())
                .map(|row| (!values.is_null(row)).then(|| values.value(row).to_owned()))
                .collect(),
        ));
    }
    if matches!(column.data_type(), DataType::Null) {
        return Ok(Some(vec![None; column.len()]));
    }
    Err(PackageValidationError::InvalidTable(table.to_string()))
}

fn string_list_column(
    batch: &arrow_array::RecordBatch,
    schema: &SchemaRef,
    name: &str,
    table: &str,
) -> Result<Vec<String>, PackageValidationError> {
    let index = schema
        .index_of(name)
        .map_err(|_| PackageValidationError::MissingColumn {
            table: table.to_string(),
            column: name.to_string(),
        })?;
    let column = batch.column(index);
    if let Some(values) = column.as_any().downcast_ref::<StringArray>() {
        return parse_json_string_lists(
            (0..values.len()).map(|row| (!values.is_null(row)).then(|| values.value(row))),
        );
    }
    if let Some(values) = column.as_any().downcast_ref::<LargeStringArray>() {
        return parse_json_string_lists(
            (0..values.len()).map(|row| (!values.is_null(row)).then(|| values.value(row))),
        );
    }
    let child_values = column
        .as_any()
        .downcast_ref::<ListArray>()
        .map(|list| list.values())
        .or_else(|| {
            column
                .as_any()
                .downcast_ref::<LargeListArray>()
                .map(|list| list.values())
        });
    if let Some(child_values) = child_values {
        if let Some(values) = child_values.as_any().downcast_ref::<StringArray>() {
            return Ok((0..values.len())
                .filter(|row| !values.is_null(*row))
                .map(|row| values.value(row).to_owned())
                .collect());
        }
        if let Some(values) = child_values.as_any().downcast_ref::<LargeStringArray>() {
            return Ok((0..values.len())
                .filter(|row| !values.is_null(*row))
                .map(|row| values.value(row).to_owned())
                .collect());
        }
    }
    Err(PackageValidationError::InvalidTable(table.to_string()))
}

fn parse_json_string_lists<'a>(
    values: impl Iterator<Item = Option<&'a str>>,
) -> Result<Vec<String>, PackageValidationError> {
    let mut output = Vec::new();
    for value in values.flatten() {
        let list: Vec<String> = serde_json::from_str(value)
            .map_err(|_| PackageValidationError::InvalidTable("signal".to_string()))?;
        output.extend(list);
    }
    Ok(output)
}

fn timestamps(
    batch: &arrow_array::RecordBatch,
    schema: &SchemaRef,
    name: &str,
    table: &str,
) -> Result<Vec<i64>, PackageValidationError> {
    let index = schema
        .index_of(name)
        .map_err(|_| PackageValidationError::MissingColumn {
            table: table.to_string(),
            column: name.to_string(),
        })?;
    let column = batch.column(index);
    macro_rules! values {
        ($array:ty, $multiplier:expr, $divisor:expr) => {
            if let Some(array) = column.as_any().downcast_ref::<$array>() {
                return (0..array.len())
                    .map(|row| {
                        if array.is_null(row) {
                            Err(PackageValidationError::ClockViolation(table.to_string()))
                        } else {
                            Ok(array.value(row).saturating_mul($multiplier) / $divisor)
                        }
                    })
                    .collect();
            }
        };
    }
    values!(TimestampSecondArray, 1_000_000, 1);
    values!(TimestampMillisecondArray, 1_000, 1);
    values!(TimestampMicrosecondArray, 1, 1);
    values!(TimestampNanosecondArray, 1, 1_000);
    if matches!(column.data_type(), DataType::Null) {
        return Err(PackageValidationError::ClockViolation(table.to_string()));
    }
    Err(PackageValidationError::InvalidTable(table.to_string()))
}

fn optional_timestamps(
    batch: &arrow_array::RecordBatch,
    schema: &SchemaRef,
    name: &str,
    table: &str,
) -> Result<Vec<Option<i64>>, PackageValidationError> {
    let Ok(index) = schema.index_of(name) else {
        return Ok(vec![None; batch.num_rows()]);
    };
    let column = batch.column(index);
    macro_rules! values {
        ($array:ty, $multiplier:expr, $divisor:expr) => {
            if let Some(array) = column.as_any().downcast_ref::<$array>() {
                return Ok((0..array.len())
                    .map(|row| {
                        (!array.is_null(row))
                            .then(|| array.value(row).saturating_mul($multiplier) / $divisor)
                    })
                    .collect());
            }
        };
    }
    values!(TimestampSecondArray, 1_000_000, 1);
    values!(TimestampMillisecondArray, 1_000, 1);
    values!(TimestampMicrosecondArray, 1, 1);
    values!(TimestampNanosecondArray, 1, 1_000);
    if matches!(column.data_type(), DataType::Null) {
        return Ok(vec![None; column.len()]);
    }
    Err(PackageValidationError::InvalidTable(table.to_string()))
}

fn event_time_column(table: &str) -> &'static str {
    match table {
        "identity_check" => "started_at",
        "approval" => "requested_at",
        _ => "event_time",
    }
}

fn table_id_column(table: &str) -> &'static str {
    match table {
        "identity_check" => "check_id",
        "turn" => "turn_id",
        "routing_step" => "step_id",
        "copilot_query" => "query_id",
        "tool_call" => "call_id",
        "approval" => "approval_id",
        "signal" => "signal_id",
        _ => "case_id",
    }
}

#[derive(Debug)]
pub enum PackageValidationError {
    MissingContract,
    InvalidContract,
    MissingContractTable(String),
    MissingTable(String),
    InvalidTable(String),
    DuplicateColumn { table: String, column: String },
    MissingColumn { table: String, column: String },
    WrongColumnType { table: String, column: String },
    RequiredValueNull { table: String, column: String },
    Cardinality(String),
    BrokenRelation(String),
    ClockViolation(String),
}

impl std::fmt::Display for PackageValidationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingContract => {
                formatter.write_str("E0 package is missing the platform history contract")
            }
            Self::InvalidContract => formatter.write_str("E0 platform history contract is invalid"),
            Self::MissingContractTable(table) => {
                write!(formatter, "E0 contract is missing required table {table}")
            }
            Self::MissingTable(table) => {
                write!(formatter, "E0 package is missing required table {table}")
            }
            Self::InvalidTable(table) => write!(formatter, "E0 table {table} cannot be decoded"),
            Self::DuplicateColumn { table, column } => {
                write!(formatter, "E0 table {table} has duplicate column {column}")
            }
            Self::MissingColumn { table, column } => write!(
                formatter,
                "E0 table {table} is missing contract column {column}"
            ),
            Self::WrongColumnType { table, column } => write!(
                formatter,
                "E0 table {table} has a type mismatch for column {column}"
            ),
            Self::RequiredValueNull { table, column } => write!(
                formatter,
                "E0 table {table} has null values in required column {column}"
            ),
            Self::Cardinality(table) => write!(
                formatter,
                "E0 table {table} violates a cardinality constraint"
            ),
            Self::BrokenRelation(table) => {
                write!(formatter, "E0 table {table} has a broken relationship")
            }
            Self::ClockViolation(table) => {
                write!(formatter, "E0 table {table} has an invalid event clock")
            }
        }
    }
}

impl std::error::Error for PackageValidationError {}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow_schema::{Field, Schema};
    use serde_json::json;

    #[test]
    fn duplicate_columns_are_rejected_without_reading_row_values() {
        let schema = ArcSchema::new(Schema::new(vec![
            Field::new("id", DataType::Utf8, false),
            Field::new("id", DataType::Utf8, false),
        ]));
        let contract: Value = serde_json::from_value(json!({
            "id":{"type":"VARCHAR","required":true}
        }))
        .unwrap();
        let fields = contract.as_object().unwrap();
        assert!(matches!(
            validate_schema("case", fields, &schema),
            Err(PackageValidationError::DuplicateColumn { .. })
        ));
    }

    type ArcSchema = std::sync::Arc<Schema>;
}
