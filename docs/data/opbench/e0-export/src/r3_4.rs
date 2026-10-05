//! R3-4 aggregate analysis over explicitly allowlisted E0 and complaint fields.
//!
//! Raw keys and query signatures are process-local join/grouping values only;
//! `aggregate` must never serialize them.

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs::{self, File},
    path::{Path, PathBuf},
};

use arrow_array::{
    Array, BooleanArray, Int32Array, Int64Array, LargeListArray, LargeStringArray, ListArray,
    RecordBatch, StringArray, TimestampMicrosecondArray,
};
use parquet::arrow::{ProjectionMask, arrow_reader::ParquetRecordBatchReaderBuilder};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug)]
pub struct BankComplaint {
    pub complaint_id: String,
    pub category: String,
    pub subcategory: String,
    pub status: String,
    pub sla_breached: Option<bool>,
    pub resolution: Option<String>,
    pub resolution_days: Option<i64>,
}

#[derive(Clone, Debug)]
pub struct E0Case {
    pub case_id: String,
    pub complaint_id: Option<String>,
    pub opened_at: i64,
}

#[derive(Clone, Debug)]
pub struct CopilotQuery {
    pub case_id: String,
    pub event_time: i64,
    pub query_signature: String,
    pub tables_read: Vec<String>,
    pub answered_by: String,
    pub sent_to_chat: Option<bool>,
}

#[derive(Clone, Debug)]
pub struct CaseClose {
    pub case_id: String,
    pub closed_at: Option<i64>,
    pub resolved: Option<bool>,
    pub resolution_code: Option<String>,
    pub csat: Option<i64>,
}

#[derive(Clone, Debug, Default)]
pub struct Input {
    pub cases: Vec<E0Case>,
    pub complaints: Vec<BankComplaint>,
    pub queries: Vec<CopilotQuery>,
    pub closes: Vec<CaseClose>,
}

pub type E0Inputs = (Vec<E0Case>, Vec<CopilotQuery>, Vec<CaseClose>);

const ALLOWED_TABLES: [&str; 12] = [
    "branches",
    "call_center_interactions",
    "call_transcripts",
    "campaign_sends",
    "complaints",
    "customers",
    "daily_exchange_rates",
    "digital_events",
    "marketing_campaigns",
    "products",
    "service_agents",
    "transactions",
];

#[derive(Clone, Debug, Default)]
struct QueryFamilyCounts {
    events: u64,
    cases: HashSet<String>,
    freeform: u64,
    tool: u64,
    sent_yes: u64,
    sent_no: u64,
    sent_unknown: u64,
}

#[derive(Clone, Debug, Default)]
struct CaseStats {
    query_signatures: HashMap<String, u64>,
    families: HashMap<&'static str, u64>,
}

#[derive(Clone, Debug, Default)]
struct CategoryStats {
    cases: u64,
    query_bearing_cases: u64,
    family_counts: BTreeMap<&'static str, QueryFamilyCounts>,
    repeated_signature: u64,
    repeated_family: u64,
    close_rows: u64,
    resolved: u64,
    unresolved: u64,
    resolved_unknown: u64,
    resolution_code_known: u64,
    resolution_code_missing: u64,
    csat_known: u64,
    csat_missing: u64,
    bank_status_open_like: u64,
    bank_status_resolved_or_closed: u64,
    bank_sla_yes: u64,
    bank_sla_no: u64,
    bank_sla_unknown: u64,
    bank_resolution_known: u64,
    bank_resolution_missing: u64,
    bank_resolution_days: Vec<i64>,
    bank_resolution_days_missing: u64,
}

type CategoryKey = (String, String);

/// R3-4 release policy uses conservative whole-table suppression when any leaf
/// or its complementary partition is below k. IDs and query signatures never
/// appear in the returned JSON.
pub fn aggregate(input: &Input, k: u64) -> Result<Value, &'static str> {
    if k < 10 {
        return Err("minimum aggregate support cannot be below 10");
    }
    if input.cases.is_empty() || input.complaints.is_empty() {
        return Err("required source is empty");
    }

    let mut cases_by_id = HashMap::new();
    for case in &input.cases {
        validate_key(&case.case_id)?;
        if cases_by_id.insert(case.case_id.as_str(), case).is_some() {
            return Err("duplicate E0 case key");
        }
    }

    let mut complaints_by_id = HashMap::new();
    for complaint in &input.complaints {
        validate_key(&complaint.complaint_id)?;
        if complaint.category.trim().is_empty() || complaint.subcategory.trim().is_empty() {
            return Err("bank complaint has an unknown category");
        }
        if complaints_by_id
            .insert(complaint.complaint_id.as_str(), complaint)
            .is_some()
        {
            return Err("duplicate bank complaint key");
        }
        complaint_status_bucket(&complaint.status)?;
    }

    let mut linked_complaints = HashSet::new();
    let mut linked_case_ids = HashSet::new();
    let mut categories: BTreeMap<CategoryKey, CategoryStats> = BTreeMap::new();
    let mut case_categories: HashMap<&str, CategoryKey> = HashMap::new();
    let mut null_link_count = 0_u64;
    let mut orphan_link_count = 0_u64;
    for case in &input.cases {
        let Some(complaint_id) = case.complaint_id.as_deref() else {
            null_link_count += 1;
            continue;
        };
        validate_key(complaint_id)?;
        let Some(complaint) = complaints_by_id.get(complaint_id) else {
            orphan_link_count += 1;
            continue;
        };
        if !linked_complaints.insert(complaint_id) {
            return Err("complaint linked to multiple E0 cases");
        }
        linked_case_ids.insert(case.case_id.as_str());
        let key = (complaint.category.clone(), complaint.subcategory.clone());
        categories.entry(key.clone()).or_default().cases += 1;
        case_categories.insert(case.case_id.as_str(), key);
    }
    if null_link_count > 0 || orphan_link_count > 0 || linked_case_ids.len() != input.cases.len() {
        return Err("E0 contains incomplete exact complaint linkage");
    }

    let mut close_by_case = HashMap::new();
    for close in &input.closes {
        validate_key(&close.case_id)?;
        if !cases_by_id.contains_key(close.case_id.as_str()) {
            return Err("case close references an unknown case");
        }
        if close_by_case
            .insert(close.case_id.as_str(), close)
            .is_some()
        {
            return Err("duplicate case close key");
        }
        if close.csat.is_some_and(|score| !(1..=4).contains(&score)) {
            return Err("E0 CSAT is outside its registered domain");
        }
        if let Some(closed_at) = close.closed_at
            && closed_at < cases_by_id[close.case_id.as_str()].opened_at
        {
            return Err("case close precedes case opening");
        }
    }

    let mut query_rows_by_case: HashMap<&str, Vec<&CopilotQuery>> = HashMap::new();
    for query in &input.queries {
        validate_key(&query.case_id)?;
        let Some(case) = cases_by_id.get(query.case_id.as_str()) else {
            return Err("query references an unknown case");
        };
        if query.event_time < case.opened_at
            || close_by_case
                .get(query.case_id.as_str())
                .is_some_and(|close| {
                    close
                        .closed_at
                        .is_none_or(|closed_at| query.event_time > closed_at)
                })
        {
            return Err("query outside case lifecycle");
        }
        if query.query_signature.trim().is_empty() {
            return Err("query signature is empty");
        }
        query_family(&query.tables_read)?;
        answer_class(&query.answered_by)?;
        query_rows_by_case
            .entry(query.case_id.as_str())
            .or_default()
            .push(query);
    }

    for (case_id, category_key) in &case_categories {
        let stats = categories
            .get_mut(category_key)
            .expect("category was created with the case");
        if let Some(complaint) = input
            .cases
            .iter()
            .find(|case| case.case_id == **case_id)
            .and_then(|case| case.complaint_id.as_deref())
            .and_then(|complaint_id| complaints_by_id.get(complaint_id))
        {
            match complaint_status_bucket(&complaint.status)? {
                "open_like" => stats.bank_status_open_like += 1,
                "resolved_or_closed" => stats.bank_status_resolved_or_closed += 1,
                _ => unreachable!("status bucketing is closed"),
            }
            match complaint.sla_breached {
                Some(true) => stats.bank_sla_yes += 1,
                Some(false) => stats.bank_sla_no += 1,
                None => stats.bank_sla_unknown += 1,
            }
            if complaint
                .resolution
                .as_deref()
                .is_some_and(|s| !s.trim().is_empty())
            {
                stats.bank_resolution_known += 1;
            } else {
                stats.bank_resolution_missing += 1;
            }
            if let Some(days) = complaint.resolution_days {
                if days < 0 {
                    return Err("negative bank complaint resolution days");
                }
                stats.bank_resolution_days.push(days);
            } else {
                stats.bank_resolution_days_missing += 1;
            }
        }

        if let Some(close) = close_by_case.get(case_id) {
            stats.close_rows += 1;
            match close.resolved {
                Some(true) => stats.resolved += 1,
                Some(false) => stats.unresolved += 1,
                None => stats.resolved_unknown += 1,
            }
            if close
                .resolution_code
                .as_deref()
                .is_some_and(|s| !s.trim().is_empty())
            {
                stats.resolution_code_known += 1;
            } else {
                stats.resolution_code_missing += 1;
            }
            if close.csat.is_some_and(|score| (1..=4).contains(&score)) {
                stats.csat_known += 1;
            } else {
                stats.csat_missing += 1;
            }
        } else {
            stats.resolved_unknown += 1;
            stats.resolution_code_missing += 1;
            stats.csat_missing += 1;
        }

        let Some(queries) = query_rows_by_case.get(case_id) else {
            continue;
        };
        stats.query_bearing_cases += 1;
        let mut case_stats = CaseStats::default();
        for query in queries {
            let family = query_family(&query.tables_read)?;
            *case_stats.families.entry(family).or_default() += 1;
            *case_stats
                .query_signatures
                .entry(query.query_signature.clone())
                .or_default() += 1;
            let family_counts = stats.family_counts.entry(family).or_default();
            family_counts.events += 1;
            family_counts.cases.insert((*case_id).to_owned());
            match answer_class(&query.answered_by)? {
                "freeform" => family_counts.freeform += 1,
                "tool" => family_counts.tool += 1,
                _ => unreachable!("answer classes are closed"),
            }
            match query.sent_to_chat {
                Some(true) => family_counts.sent_yes += 1,
                Some(false) => family_counts.sent_no += 1,
                None => family_counts.sent_unknown += 1,
            }
        }
        if case_stats
            .query_signatures
            .values()
            .any(|count| *count >= 2)
        {
            stats.repeated_signature += 1;
        }
        if case_stats.families.values().any(|count| *count >= 2) {
            stats.repeated_family += 1;
        }
    }

    let link_total = input.cases.len() as u64;
    let linked_total = linked_case_ids.len() as u64;
    let link_coverage = table_envelope(
        vec![json!({
            "source_link_grade": "linked",
            "case_count": link_total,
            "exact_link_count": linked_total,
            "null_link_count": null_link_count,
            "orphan_link_count": orphan_link_count
        })],
        &[link_total, linked_total, null_link_count, orphan_link_count],
        k,
    );

    let query_coverage = json!({
        "no_row_interpretation": "not_observed",
        "queryless_case_rate": Value::Null
    });
    let (e0_close_outcomes, e0_close_coverage) = e0_close_tables(&categories, k);
    let (bank_outcomes, bank_coverage) = bank_complaint_tables(&categories, k);
    let query_family_table = query_family_table(&categories, k);
    let query_repeat_table = query_repeat_table(&categories, k);

    Ok(json!({
        "benchmark": "OPBENCH-lite-R3-4",
        "version": "1",
        "privacy": {"minimum_count": k, "aggregate_only": true, "row_data_included": false},
        "evidence": {
            "source_link_grade": "linked",
            "workflow_link_grade": "not_evaluable",
            "generated_process_data": true,
            "interpretation": "post_hoc_descriptive_no_causal_or_lift_claim"
        },
        "query_coverage": query_coverage,
        "tables": {
            "link_coverage": link_coverage,
            "query_families_by_category": query_family_table,
            "query_repeats_by_category": query_repeat_table,
            "e0_close_outcomes_by_category": e0_close_outcomes,
            "e0_close_coverage_by_category": e0_close_coverage,
            "bank_complaint_outcomes_by_category": bank_outcomes,
            "bank_complaint_coverage_by_category": bank_coverage
        }
    }))
}

fn validate_key(value: &str) -> Result<(), &'static str> {
    if value.trim().is_empty() || value.trim() != value {
        return Err("blank or malformed source key");
    }
    Ok(())
}

fn query_family(tables: &[String]) -> Result<&'static str, &'static str> {
    if tables.is_empty() {
        return Err("query has no recorded table reads");
    }
    let mut unique = HashSet::new();
    for table in tables {
        if !ALLOWED_TABLES.contains(&table.as_str()) {
            return Err("unallowlisted table in query metadata");
        }
        if !unique.insert(table.as_str()) {
            return Err("duplicate table in query metadata");
        }
    }
    if unique.contains("transactions") {
        Ok("transaction_lookup")
    } else if unique.contains("complaints") {
        Ok("complaint_lookup")
    } else if unique.contains("customers") || unique.contains("products") {
        Ok("customer_or_product_context")
    } else {
        Ok("other_allowlisted_read")
    }
}

fn answer_class(value: &str) -> Result<&'static str, &'static str> {
    if value == "freeform" {
        Ok("freeform")
    } else if value
        .strip_prefix("tool:")
        .is_some_and(|tool| !tool.trim().is_empty())
    {
        Ok("tool")
    } else {
        Err("unrecognized answer source")
    }
}

fn complaint_status_bucket(value: &str) -> Result<&'static str, &'static str> {
    match value.trim().to_ascii_lowercase().as_str() {
        "open" | "in process" | "escalated" => Ok("open_like"),
        "resolved" | "closed" => Ok("resolved_or_closed"),
        _ => Err("unknown bank complaint status"),
    }
}

fn category_label(key: &CategoryKey) -> Value {
    json!({"category": key.0, "subcategory": key.1, "source_link_grade": "linked"})
}

fn table_envelope(rows: Vec<Value>, cells: &[u64], k: u64) -> Value {
    if cells.iter().any(|count| *count < k) {
        suppressed_table("leaf_or_complement_below_k")
    } else {
        json!({"suppressed": false, "suppression_reason": null, "rows": rows})
    }
}

fn suppressed_table(reason: &str) -> Value {
    json!({"suppressed": true, "suppression_reason": reason, "rows": []})
}

fn query_family_table(categories: &BTreeMap<CategoryKey, CategoryStats>, k: u64) -> Value {
    let mut rows = Vec::new();
    let mut cells = Vec::new();
    for (category, stats) in categories {
        let family_total: u64 = stats
            .family_counts
            .values()
            .map(|counts| counts.events)
            .sum();
        for (family, counts) in &stats.family_counts {
            let events = counts.events;
            let event_complement = family_total.saturating_sub(events);
            let case_support = counts.cases.len() as u64;
            let case_complement = stats.query_bearing_cases.saturating_sub(case_support);
            let sent_total = counts.sent_yes + counts.sent_no + counts.sent_unknown;
            let mut row = category_label(category);
            row["query_family"] = json!(family);
            row["query_bearing_cases"] = json!(stats.query_bearing_cases);
            row["family_case_support"] = json!(case_support);
            row["family_event_count"] = json!(events);
            row["answered_freeform"] = json!(counts.freeform);
            row["answered_by_tool"] = json!(counts.tool);
            row["sent_to_chat_yes"] = json!(counts.sent_yes);
            row["sent_to_chat_no"] = json!(counts.sent_no);
            row["sent_to_chat_unknown"] = json!(counts.sent_unknown);
            row["sent_to_chat_known"] = json!(sent_total.saturating_sub(counts.sent_unknown));
            rows.push(row);
            cells.extend([
                case_support,
                case_complement,
                events,
                event_complement,
                counts.freeform,
                counts.tool,
                counts.sent_yes,
                counts.sent_no,
                counts.sent_unknown,
                sent_total.saturating_sub(counts.sent_unknown),
            ]);
        }
    }
    if rows.is_empty() {
        return suppressed_table("no_query_rows");
    }
    table_envelope(rows, &cells, k)
}

fn query_repeat_table(categories: &BTreeMap<CategoryKey, CategoryStats>, k: u64) -> Value {
    let mut rows = Vec::new();
    let mut cells = Vec::new();
    for (category, stats) in categories {
        if stats.query_bearing_cases == 0 {
            continue;
        }
        let signature_other = stats
            .query_bearing_cases
            .saturating_sub(stats.repeated_signature);
        let family_other = stats
            .query_bearing_cases
            .saturating_sub(stats.repeated_family);
        let mut row = category_label(category);
        row["query_bearing_cases"] = json!(stats.query_bearing_cases);
        row["repeated_signature_cases"] = json!(stats.repeated_signature);
        row["not_repeated_signature_cases"] = json!(signature_other);
        row["repeated_family_cases"] = json!(stats.repeated_family);
        row["not_repeated_family_cases"] = json!(family_other);
        rows.push(row);
        cells.extend([
            stats.query_bearing_cases,
            stats.repeated_signature,
            signature_other,
            stats.repeated_family,
            family_other,
        ]);
    }
    table_envelope(rows, &cells, k)
}

fn e0_close_tables(categories: &BTreeMap<CategoryKey, CategoryStats>, k: u64) -> (Value, Value) {
    let mut outcome_rows = Vec::new();
    let mut outcome_cells = Vec::new();
    let mut coverage_rows = Vec::new();
    let mut coverage_cells = Vec::new();
    for (category, stats) in categories {
        let close_unknown = stats.cases.saturating_sub(stats.close_rows);
        let mut outcome = category_label(category);
        outcome["resolved"] = json!(stats.resolved);
        outcome["unresolved"] = json!(stats.unresolved);
        outcome["resolution_code_known"] = json!(stats.resolution_code_known);
        outcome["csat_known"] = json!(stats.csat_known);
        outcome_rows.push(outcome);
        outcome_cells.extend([
            stats.resolved,
            stats.unresolved,
            stats.resolution_code_known,
            stats.csat_known,
        ]);

        let mut coverage = category_label(category);
        coverage["close_missing"] = json!(close_unknown);
        coverage["resolved_unknown"] = json!(stats.resolved_unknown);
        coverage["resolution_code_missing"] = json!(stats.resolution_code_missing);
        coverage["csat_missing"] = json!(stats.csat_missing);
        coverage_rows.push(coverage);
        coverage_cells.extend([
            close_unknown,
            stats.close_rows,
            stats.resolved_unknown,
            stats.resolved + stats.unresolved,
            stats.resolution_code_missing,
            stats.resolution_code_known,
            stats.csat_missing,
            stats.csat_known,
        ]);
    }
    let outcomes = table_envelope(outcome_rows, &outcome_cells, k);
    let coverage = table_envelope(coverage_rows, &coverage_cells, k);
    if coverage["suppressed"] == true {
        (
            suppressed_table("unsafe_joint_outcome_coverage"),
            suppressed_table("unsafe_joint_outcome_coverage"),
        )
    } else {
        (outcomes, coverage)
    }
}

fn bank_complaint_tables(
    categories: &BTreeMap<CategoryKey, CategoryStats>,
    k: u64,
) -> (Value, Value) {
    let mut outcome_rows = Vec::new();
    let mut outcome_cells = Vec::new();
    let mut coverage_rows = Vec::new();
    let mut coverage_cells = Vec::new();
    for (category, stats) in categories {
        let days = sorted_median(&stats.bank_resolution_days);
        let mut outcome = category_label(category);
        outcome["status_open_like"] = json!(stats.bank_status_open_like);
        outcome["status_resolved_or_closed"] = json!(stats.bank_status_resolved_or_closed);
        outcome["sla_breached_yes"] = json!(stats.bank_sla_yes);
        outcome["sla_breached_no"] = json!(stats.bank_sla_no);
        outcome["resolution_known"] = json!(stats.bank_resolution_known);
        outcome["resolution_days_median"] = days.map_or(Value::Null, |value| json!(value));
        outcome_rows.push(outcome);
        outcome_cells.extend([
            stats.cases,
            stats.bank_status_open_like,
            stats.bank_status_resolved_or_closed,
            stats.bank_sla_yes,
            stats.bank_sla_no,
            stats.bank_resolution_known,
            stats.bank_resolution_days.len() as u64,
        ]);

        let mut coverage = category_label(category);
        coverage["sla_breached_unknown"] = json!(stats.bank_sla_unknown);
        coverage["resolution_missing"] = json!(stats.bank_resolution_missing);
        coverage["resolution_days_missing"] = json!(stats.bank_resolution_days_missing);
        coverage_rows.push(coverage);
        coverage_cells.extend([
            stats.bank_sla_unknown,
            stats.bank_sla_yes + stats.bank_sla_no,
            stats.bank_resolution_missing,
            stats.bank_resolution_known,
            stats.bank_resolution_days_missing,
            stats.bank_resolution_days.len() as u64,
        ]);
    }
    let outcomes = table_envelope(outcome_rows, &outcome_cells, k);
    let coverage = table_envelope(coverage_rows, &coverage_cells, k);
    if coverage["suppressed"] == true {
        (
            suppressed_table("unsafe_joint_outcome_coverage"),
            suppressed_table("unsafe_joint_outcome_coverage"),
        )
    } else {
        (outcomes, coverage)
    }
}

fn sorted_median(values: &[i64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let middle = sorted.len() / 2;
    let median = if sorted.len().is_multiple_of(2) {
        (sorted[middle - 1] as f64 + sorted[middle] as f64) / 2.0
    } else {
        sorted[middle] as f64
    };
    Some((median * 10.0).round() / 10.0)
}

fn read_projected_batches(path: &Path, columns: &[&str]) -> Result<Vec<RecordBatch>, &'static str> {
    let file = File::open(path).map_err(|_| "could not open an allowlisted E0 table")?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)
        .map_err(|_| "could not read allowlisted E0 table metadata")?;
    for column in columns {
        builder
            .schema()
            .index_of(column)
            .map_err(|_| "allowlisted E0 schema is missing a required field")?;
    }
    let projection = ProjectionMask::columns(builder.parquet_schema(), columns.iter().copied());
    builder
        .with_projection(projection)
        .build()
        .map_err(|_| "could not build projected E0 reader")?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "could not read projected E0 rows")
}

fn string_at(array: &dyn Array, row: usize) -> Result<String, &'static str> {
    if let Some(values) = array.as_any().downcast_ref::<StringArray>() {
        return if values.is_null(row) {
            Err("required allowlisted string is null")
        } else {
            Ok(values.value(row).to_owned())
        };
    }
    if let Some(values) = array.as_any().downcast_ref::<LargeStringArray>() {
        return if values.is_null(row) {
            Err("required allowlisted string is null")
        } else {
            Ok(values.value(row).to_owned())
        };
    }
    Err("allowlisted E0 field has an unsupported string type")
}

fn optional_string_at(array: &dyn Array, row: usize) -> Result<Option<String>, &'static str> {
    if let Some(values) = array.as_any().downcast_ref::<StringArray>() {
        return Ok((!values.is_null(row)).then(|| values.value(row).to_owned()));
    }
    if let Some(values) = array.as_any().downcast_ref::<LargeStringArray>() {
        return Ok((!values.is_null(row)).then(|| values.value(row).to_owned()));
    }
    Err("allowlisted E0 field has an unsupported string type")
}

fn timestamp_at(array: &dyn Array, row: usize) -> Result<Option<i64>, &'static str> {
    if let Some(values) = array.as_any().downcast_ref::<TimestampMicrosecondArray>() {
        return Ok((!values.is_null(row)).then(|| values.value(row)));
    }
    Err("allowlisted E0 field has an unsupported timestamp type")
}

fn bool_at(array: &dyn Array, row: usize) -> Result<Option<bool>, &'static str> {
    let values = array
        .as_any()
        .downcast_ref::<BooleanArray>()
        .ok_or("allowlisted E0 field has an unsupported boolean type")?;
    Ok((!values.is_null(row)).then(|| values.value(row)))
}

fn integer_at(array: &dyn Array, row: usize) -> Result<Option<i64>, &'static str> {
    if let Some(values) = array.as_any().downcast_ref::<Int32Array>() {
        return Ok((!values.is_null(row)).then(|| i64::from(values.value(row))));
    }
    if let Some(values) = array.as_any().downcast_ref::<Int64Array>() {
        return Ok((!values.is_null(row)).then(|| values.value(row)));
    }
    Err("allowlisted E0 field has an unsupported integer type")
}

fn list_strings_at(array: &dyn Array, row: usize) -> Result<Vec<String>, &'static str> {
    if let Some(values) = array.as_any().downcast_ref::<LargeStringArray>() {
        if values.is_null(row) {
            return Err("required query table list is null");
        }
        return serde_json::from_str(values.value(row))
            .map_err(|_| "allowlisted query list has an invalid serialized array");
    }
    if let Some(values) = array.as_any().downcast_ref::<StringArray>() {
        if values.is_null(row) {
            return Err("required query table list is null");
        }
        return serde_json::from_str(values.value(row))
            .map_err(|_| "allowlisted query list has an invalid serialized array");
    }
    let values = if let Some(list) = array.as_any().downcast_ref::<ListArray>() {
        if list.is_null(row) {
            return Err("required query table list is null");
        }
        list.value(row)
    } else if let Some(list) = array.as_any().downcast_ref::<LargeListArray>() {
        if list.is_null(row) {
            return Err("required query table list is null");
        }
        list.value(row)
    } else {
        return Err("allowlisted query list has an unsupported type");
    };
    let mut strings = Vec::with_capacity(values.len());
    for index in 0..values.len() {
        strings.push(string_at(values.as_ref(), index)?);
    }
    Ok(strings)
}

fn column_index(batch: &RecordBatch, name: &str) -> Result<usize, &'static str> {
    batch
        .schema()
        .index_of(name)
        .map_err(|_| "projected E0 batch is missing an allowlisted field")
}

pub fn read_e0_inputs(data_dir: &Path) -> Result<E0Inputs, &'static str> {
    let mut cases = Vec::new();
    for batch in read_projected_batches(
        &data_dir.join("case.parquet"),
        &["case_id", "complaint_id", "opened_at"],
    )? {
        let case_id = column_index(&batch, "case_id")?;
        let complaint_id = column_index(&batch, "complaint_id")?;
        let opened_at = column_index(&batch, "opened_at")?;
        for row in 0..batch.num_rows() {
            cases.push(E0Case {
                case_id: string_at(batch.column(case_id).as_ref(), row)?,
                complaint_id: optional_string_at(batch.column(complaint_id).as_ref(), row)?,
                opened_at: timestamp_at(batch.column(opened_at).as_ref(), row)?
                    .ok_or("E0 case opening timestamp is null")?,
            });
        }
    }

    let mut queries = Vec::new();
    for batch in read_projected_batches(
        &data_dir.join("copilot_query.parquet"),
        &[
            "case_id",
            "event_time",
            "query_signature",
            "tables_read",
            "answered_by",
            "sent_to_chat",
        ],
    )? {
        let case_id = column_index(&batch, "case_id")?;
        let event_time = column_index(&batch, "event_time")?;
        let query_signature = column_index(&batch, "query_signature")?;
        let tables_read = column_index(&batch, "tables_read")?;
        let answered_by = column_index(&batch, "answered_by")?;
        let sent_to_chat = column_index(&batch, "sent_to_chat")?;
        for row in 0..batch.num_rows() {
            queries.push(CopilotQuery {
                case_id: string_at(batch.column(case_id).as_ref(), row)?,
                event_time: timestamp_at(batch.column(event_time).as_ref(), row)?
                    .ok_or("E0 query timestamp is null")?,
                query_signature: string_at(batch.column(query_signature).as_ref(), row)?,
                tables_read: list_strings_at(batch.column(tables_read).as_ref(), row)?,
                answered_by: string_at(batch.column(answered_by).as_ref(), row)?,
                sent_to_chat: bool_at(batch.column(sent_to_chat).as_ref(), row)?,
            });
        }
    }

    let mut closes = Vec::new();
    for batch in read_projected_batches(
        &data_dir.join("case_close.parquet"),
        &[
            "case_id",
            "closed_at",
            "resolved",
            "resolution_code",
            "csat",
        ],
    )? {
        let case_id = column_index(&batch, "case_id")?;
        let closed_at = column_index(&batch, "closed_at")?;
        let resolved = column_index(&batch, "resolved")?;
        let resolution_code = column_index(&batch, "resolution_code")?;
        let csat = column_index(&batch, "csat")?;
        for row in 0..batch.num_rows() {
            closes.push(CaseClose {
                case_id: string_at(batch.column(case_id).as_ref(), row)?,
                closed_at: timestamp_at(batch.column(closed_at).as_ref(), row)?,
                resolved: bool_at(batch.column(resolved).as_ref(), row)?,
                resolution_code: optional_string_at(batch.column(resolution_code).as_ref(), row)?,
                csat: integer_at(batch.column(csat).as_ref(), row)?,
            });
        }
    }

    Ok((cases, queries, closes))
}

pub fn read_bank_complaints(data_root: &Path) -> Result<Vec<BankComplaint>, &'static str> {
    let complaints_root = data_root.join("complaints");
    let mut files = Vec::new();
    collect_csv_files(&complaints_root, &mut files)?;
    files.sort();
    if files.is_empty() {
        return Err("bank complaint source is empty");
    }
    let mut rows = Vec::new();
    for path in files {
        let mut reader = csv::ReaderBuilder::new()
            .has_headers(true)
            .from_path(&path)
            .map_err(|_| "could not read bank complaint source")?;
        let headers = reader
            .headers()
            .map_err(|_| "could not read bank complaint schema")?
            .clone();
        let required = [
            "complaint_id",
            "category",
            "subcategory",
            "status",
            "sla_breached",
            "resolution",
            "resolution_days",
        ];
        let mut indices = Vec::with_capacity(required.len());
        for field in required {
            indices.push(
                headers
                    .iter()
                    .position(|name| name == field)
                    .ok_or("bank complaint schema is missing an allowlisted field")?,
            );
        }
        for record in reader.records() {
            let record = record.map_err(|_| "could not parse bank complaint row")?;
            let get = |index: usize| record.get(index).ok_or("bank complaint row schema drift");
            rows.push(BankComplaint {
                complaint_id: get(indices[0])?.to_owned(),
                category: get(indices[1])?.to_owned(),
                subcategory: get(indices[2])?.to_owned(),
                status: get(indices[3])?.to_owned(),
                sla_breached: parse_optional_bool(get(indices[4])?)?,
                resolution: nonempty(get(indices[5])?),
                resolution_days: parse_optional_integer(get(indices[6])?)?,
            });
        }
    }
    Ok(rows)
}

fn collect_csv_files(root: &Path, files: &mut Vec<PathBuf>) -> Result<(), &'static str> {
    let entries = fs::read_dir(root).map_err(|_| "could not open bank complaints directory")?;
    for entry in entries {
        let entry = entry.map_err(|_| "could not enumerate bank complaints directory")?;
        let path = entry.path();
        if path.is_dir() {
            collect_csv_files(&path, files)?;
        } else if path.extension().is_some_and(|extension| extension == "csv") {
            files.push(path);
        }
    }
    Ok(())
}

fn nonempty(value: &str) -> Option<String> {
    (!value.trim().is_empty()).then(|| value.to_owned())
}

fn parse_optional_bool(value: &str) -> Result<Option<bool>, &'static str> {
    match value.trim().to_ascii_lowercase().as_str() {
        "" | "null" | "na" | "n/a" => Ok(None),
        "true" | "1" | "yes" => Ok(Some(true)),
        "false" | "0" | "no" => Ok(Some(false)),
        _ => Err("bank complaint boolean schema drift"),
    }
}

fn parse_optional_integer(value: &str) -> Result<Option<i64>, &'static str> {
    let value = value.trim();
    if value.is_empty() || value.eq_ignore_ascii_case("null") || value.eq_ignore_ascii_case("na") {
        Ok(None)
    } else {
        value
            .parse()
            .map(Some)
            .map_err(|_| "bank complaint integer schema drift")
    }
}

pub fn digest_inputs(data_root: &Path, e0_data_dir: &Path) -> Result<Value, &'static str> {
    let mut complaint_files = Vec::new();
    collect_csv_files(&data_root.join("complaints"), &mut complaint_files)?;
    complaint_files.sort();
    let mut complaints = Sha256::new();
    for path in complaint_files {
        let bytes = fs::read(&path).map_err(|_| "could not digest bank complaint input")?;
        let relative = path
            .strip_prefix(data_root.join("complaints"))
            .map_err(|_| "could not identify bank complaint partition")?
            .to_string_lossy();
        update_digest_component(&mut complaints, relative.as_bytes(), &bytes);
    }
    let mut digests = serde_json::Map::new();
    digests.insert(
        "bank_complaints_sha256".into(),
        json!(format!("{:x}", complaints.finalize())),
    );
    for table in ["case", "copilot_query", "case_close"] {
        let bytes = fs::read(e0_data_dir.join(format!("{table}.parquet")))
            .map_err(|_| "could not digest allowlisted E0 input")?;
        let mut table_digest = Sha256::new();
        update_digest_component(&mut table_digest, table.as_bytes(), &bytes);
        digests.insert(
            format!("e0_{table}_sha256"),
            json!(format!("{:x}", table_digest.finalize())),
        );
    }
    Ok(Value::Object(digests))
}

fn update_digest_component(hasher: &mut Sha256, name: &[u8], bytes: &[u8]) {
    hasher.update((name.len() as u64).to_be_bytes());
    hasher.update(name);
    hasher.update((bytes.len() as u64).to_be_bytes());
    hasher.update(bytes);
}

pub fn schema_fingerprints(data_root: &Path, e0_data_dir: &Path) -> Result<Value, &'static str> {
    let mut required_csv = vec![
        "complaint_id",
        "category",
        "subcategory",
        "status",
        "sla_breached",
        "resolution",
        "resolution_days",
    ];
    required_csv.sort_unstable();
    let mut csv_hash = Sha256::new();
    for field in required_csv {
        csv_hash.update((field.len() as u64).to_be_bytes());
        csv_hash.update(field.as_bytes());
        csv_hash.update(b":string\n");
    }

    let projections: [(&str, &[&str]); 3] = [
        ("case", &["case_id", "complaint_id", "opened_at"]),
        (
            "copilot_query",
            &[
                "case_id",
                "event_time",
                "query_signature",
                "tables_read",
                "answered_by",
                "sent_to_chat",
            ],
        ),
        (
            "case_close",
            &[
                "case_id",
                "closed_at",
                "resolved",
                "resolution_code",
                "csat",
            ],
        ),
    ];
    let mut e0_hasher = Sha256::new();
    for (table, fields) in projections {
        let file = File::open(e0_data_dir.join(format!("{table}.parquet")))
            .map_err(|_| "could not open allowlisted E0 schema")?;
        let reader = ParquetRecordBatchReaderBuilder::try_new(file)
            .map_err(|_| "could not inspect allowlisted E0 schema")?;
        for field in fields {
            let arrow_field = reader
                .schema()
                .field_with_name(field)
                .map_err(|_| "allowlisted E0 schema is missing a required field")?;
            let schema_item = format!("{table}.{field}:{}", arrow_field.data_type());
            e0_hasher.update((schema_item.len() as u64).to_be_bytes());
            e0_hasher.update(schema_item.as_bytes());
        }
    }

    // Fail the fingerprint if the registered bank fields are absent from every
    // partition; per-file shape validation remains in the CSV reader.
    let mut csv_files = Vec::new();
    collect_csv_files(&data_root.join("complaints"), &mut csv_files)?;
    if csv_files.is_empty() {
        return Err("bank complaint source is empty");
    }
    for path in csv_files {
        let mut reader = csv::ReaderBuilder::new()
            .has_headers(true)
            .from_path(path)
            .map_err(|_| "could not inspect bank complaint schema")?;
        let headers = reader
            .headers()
            .map_err(|_| "could not inspect bank complaint schema")?;
        for required in [
            "complaint_id",
            "category",
            "subcategory",
            "status",
            "sla_breached",
            "resolution",
            "resolution_days",
        ] {
            if !headers.iter().any(|header| header == required) {
                return Err("bank complaint schema is missing an allowlisted field");
            }
        }
    }
    Ok(json!({
        "bank_required_csv_projection_sha256": format!("{:x}", csv_hash.finalize()),
        "e0_required_parquet_projection_sha256": format!("{:x}", e0_hasher.finalize())
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn reads_varchar_array_contract_from_large_utf8_json_projection() {
        let values = LargeStringArray::from(vec![r#"["transactions","complaints"]"#]);
        assert_eq!(
            list_strings_at(&values, 0).expect("valid serialized VARCHAR array"),
            ["transactions", "complaints"]
        );
    }

    #[test]
    fn rejects_non_array_large_utf8_query_table_metadata() {
        let values = LargeStringArray::from(vec!["transactions"]);
        assert_eq!(
            list_strings_at(&values, 0).unwrap_err(),
            "allowlisted query list has an invalid serialized array"
        );
    }

    fn fixture() -> Input {
        let complaints = (0..60)
            .map(|i| BankComplaint {
                complaint_id: format!("SYN-COMPLAINT-{i:03}"),
                category: "Disputes".into(),
                subcategory: "Unauthorized charge".into(),
                status: if i < 30 { "Open" } else { "Resolved" }.into(),
                sla_breached: Some(i < 30),
                resolution: Some("explained".into()),
                resolution_days: Some(4),
            })
            .collect::<Vec<_>>();
        let cases = (0..60)
            .map(|i| E0Case {
                case_id: format!("SYN-CASE-{i:03}"),
                complaint_id: Some(format!("SYN-COMPLAINT-{i:03}")),
                opened_at: 100,
            })
            .collect::<Vec<_>>();
        let mut queries = (0..40)
            .map(|i| CopilotQuery {
                case_id: format!("SYN-CASE-{i:03}"),
                event_time: 101,
                query_signature: format!("OPAQUE-SIGNATURE-{i:03}"),
                tables_read: if i < 20 {
                    vec!["transactions".into()]
                } else {
                    vec!["complaints".into()]
                },
                answered_by: if i < 20 { "freeform" } else { "tool:lookup" }.into(),
                sent_to_chat: Some(i % 2 == 0),
            })
            .collect::<Vec<_>>();
        // Twenty cases repeat a signature; the signature itself must never escape.
        for i in 0..20 {
            queries.push(CopilotQuery {
                case_id: format!("SYN-CASE-{i:03}"),
                event_time: 102,
                query_signature: format!("OPAQUE-SIGNATURE-{i:03}"),
                tables_read: vec!["transactions".into()],
                answered_by: "freeform".into(),
                sent_to_chat: Some(true),
            });
        }
        let closes = (0..60)
            .map(|i| CaseClose {
                case_id: format!("SYN-CASE-{i:03}"),
                closed_at: Some(103),
                resolved: Some(i >= 30),
                resolution_code: Some("explained".into()),
                csat: Some(3),
            })
            .collect();
        Input {
            cases,
            complaints,
            queries,
            closes,
        }
    }

    #[test]
    fn emits_aggregate_only_output_and_keeps_source_grade_distinct_from_workflow_grade() {
        let input = fixture();
        let output = aggregate(&input, 10).expect("valid fixture should aggregate");
        let encoded = output.to_string();
        for forbidden in [
            "SYN-CASE-",
            "SYN-COMPLAINT-",
            "OPAQUE-SIGNATURE-",
            "query_signature",
            "customer_id",
        ] {
            assert!(!encoded.contains(forbidden), "output exposed {forbidden}");
        }
        assert_eq!(output["evidence"]["source_link_grade"], "linked");
        assert_eq!(output["evidence"]["workflow_link_grade"], "not_evaluable");
        assert_eq!(output["evidence"]["generated_process_data"], true);
    }

    #[test]
    fn distinguishes_no_query_row_from_a_verified_zero_query_case() {
        let output = aggregate(&fixture(), 10).expect("valid fixture should aggregate");
        assert_eq!(
            output["query_coverage"]["no_row_interpretation"],
            "not_observed"
        );
        assert_eq!(output["query_coverage"]["queryless_case_rate"], Value::Null);
    }

    #[test]
    fn rejects_duplicate_keys_and_many_to_one_complaint_links() {
        let mut input = fixture();
        input.cases.push(input.cases[0].clone());
        assert_eq!(aggregate(&input, 10).unwrap_err(), "duplicate E0 case key");

        let mut input = fixture();
        input.cases[1].complaint_id = input.cases[0].complaint_id.clone();
        assert_eq!(
            aggregate(&input, 10).unwrap_err(),
            "complaint linked to multiple E0 cases"
        );

        let mut input = fixture();
        input.cases[0].complaint_id = Some("SYN-ORPHAN-COMPLAINT".into());
        assert_eq!(
            aggregate(&input, 10).unwrap_err(),
            "E0 contains incomplete exact complaint linkage"
        );
    }

    #[test]
    fn rejects_queries_outside_case_lifecycle_and_unallowlisted_tables() {
        let mut input = fixture();
        input.queries[0].event_time = 99;
        assert_eq!(
            aggregate(&input, 10).unwrap_err(),
            "query outside case lifecycle"
        );

        let mut input = fixture();
        input.queries[0].tables_read = vec!["secret_table".into()];
        assert_eq!(
            aggregate(&input, 10).unwrap_err(),
            "unallowlisted table in query metadata"
        );
    }

    #[test]
    fn missing_close_outcome_is_not_coerced_to_unresolved() {
        let mut input = fixture();
        for close in input.closes.iter_mut().take(10) {
            close.resolved = None;
        }
        let output = aggregate(&input, 10).expect("missing outcomes remain explicit");
        let outcomes = &output["tables"]["e0_close_outcomes_by_category"];
        assert_eq!(outcomes["suppressed"], true);
        assert_eq!(outcomes["rows"], Value::Array(vec![]));
        let coverage = &output["tables"]["e0_close_coverage_by_category"];
        assert_eq!(coverage["suppressed"], true);
        assert_eq!(coverage["rows"], Value::Array(vec![]));
    }

    #[test]
    fn rejects_queries_when_an_existing_close_has_no_timestamp() {
        let mut input = fixture();
        input.closes[0].closed_at = None;
        assert_eq!(
            aggregate(&input, 10).unwrap_err(),
            "query outside case lifecycle"
        );
    }

    #[test]
    fn rejects_out_of_domain_non_null_csat_as_schema_drift() {
        let mut input = fixture();
        input.closes[0].csat = Some(5);
        assert_eq!(
            aggregate(&input, 10).unwrap_err(),
            "E0 CSAT is outside its registered domain"
        );
    }

    #[test]
    fn bank_tables_suppress_together_when_coverage_has_small_cells() {
        let output = aggregate(&fixture(), 10).expect("valid fixture should aggregate");
        let bank = &output["tables"]["bank_complaint_outcomes_by_category"];
        assert_eq!(bank["suppressed"], true);
        assert_eq!(bank["rows"], Value::Array(vec![]));
        let coverage = &output["tables"]["bank_complaint_coverage_by_category"];
        assert_eq!(coverage["suppressed"], true);
        assert_eq!(coverage["rows"], Value::Array(vec![]));
    }

    #[test]
    fn emits_outcomes_only_when_all_joint_coverage_cells_are_k_safe() {
        let mut input = fixture();
        input.closes.drain(0..10);
        for close in input.closes.iter_mut().take(10) {
            close.resolved = None;
        }
        for close in input.closes.iter_mut().skip(10).take(10) {
            close.resolution_code = None;
        }
        for close in input.closes.iter_mut().skip(20).take(10) {
            close.csat = None;
        }
        let output = aggregate(&input, 10).expect("k-safe outcomes can be released");
        let outcomes = &output["tables"]["e0_close_outcomes_by_category"];
        assert_eq!(outcomes["suppressed"], false);
        let row = &outcomes["rows"][0];
        assert_eq!(row["resolved"], 30);
        assert_eq!(row["unresolved"], 10);
        assert!(row.get("case_count").is_none());
        assert_eq!(
            output["tables"]["e0_close_coverage_by_category"]["suppressed"],
            false
        );
    }

    #[test]
    fn emits_bank_outcomes_only_when_all_joint_coverage_cells_are_k_safe() {
        let mut input = fixture();
        for complaint in input.complaints.iter_mut().take(10) {
            complaint.sla_breached = None;
        }
        for complaint in input.complaints.iter_mut().skip(10).take(10) {
            complaint.resolution = None;
        }
        for complaint in input.complaints.iter_mut().skip(20).take(10) {
            complaint.resolution_days = None;
        }
        let output = aggregate(&input, 10).expect("k-safe bank outcomes can be released");
        let bank = &output["tables"]["bank_complaint_outcomes_by_category"];
        assert_eq!(bank["suppressed"], false);
        let row = &bank["rows"][0];
        assert!(row.get("linked_complaint_count").is_none());
        assert!(row.get("sla_breached_unknown").is_none());
        assert_eq!(
            output["tables"]["bank_complaint_coverage_by_category"]["suppressed"],
            false
        );
    }

    #[test]
    fn a_small_leaf_suppresses_the_entire_table_to_prevent_differencing() {
        let mut input = fixture();
        input.cases[0].complaint_id = None;
        assert_eq!(
            aggregate(&input, 10).unwrap_err(),
            "E0 contains incomplete exact complaint linkage"
        );
    }

    #[test]
    fn digest_components_include_names_and_lengths_to_preserve_file_boundaries() {
        let digest = |components: &[(&[u8], &[u8])]| {
            let mut hasher = Sha256::new();
            for (name, bytes) in components {
                update_digest_component(&mut hasher, name, bytes);
            }
            format!("{:x}", hasher.finalize())
        };
        assert_ne!(digest(&[(b"a", b"bc")]), digest(&[(b"ab", b"c")]));
    }
}

