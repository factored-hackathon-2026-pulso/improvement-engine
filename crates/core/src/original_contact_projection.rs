//! Privacy-preserving aggregates over the original contacts and complaints CSVs.
//!
//! This projection intentionally has no row-level output. It never reads IDs,
//! narratives, descriptions, transcript text, customer or agent attributes.

use std::{
    collections::{BTreeMap, BTreeSet},
    io::Read,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SupportStatus {
    Supported,
    Unsupported { missing_fields: Vec<&'static str> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ContactCategory {
    Complaint,
    Transactional,
    Technical,
    GeneralInquiry,
    Product,
    Account,
    Card,
    Loan,
    Other,
    Unclassified,
}

impl ContactCategory {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Complaint => "complaint",
            Self::Transactional => "transactional",
            Self::Technical => "technical",
            Self::GeneralInquiry => "general_inquiry",
            Self::Product => "product",
            Self::Account => "account",
            Self::Card => "card",
            Self::Loan => "loan",
            Self::Other => "other",
            Self::Unclassified => "unclassified",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Channel {
    Phone,
    Web,
    Chat,
    Email,
    Branch,
    MobileApp,
    Other,
}

impl Channel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Phone => "phone",
            Self::Web => "web",
            Self::Chat => "chat",
            Self::Email => "email",
            Self::Branch => "branch",
            Self::MobileApp => "mobile_app",
            Self::Other => "other",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ContactAggregate {
    pub period: String,
    pub category: ContactCategory,
    pub channel: Channel,
    pub contact_count: u64,
    pub resolved_known: u64,
    pub resolved_yes: u64,
    pub followup_known: u64,
    pub followup_yes: u64,
    pub escalated_known: u64,
    pub escalated_yes: u64,
    pub duration_mean_seconds: Option<f64>,
    pub wait_mean_seconds: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ComplaintAggregate {
    pub period: String,
    pub category: ContactCategory,
    pub channel: Channel,
    pub complaint_count: u64,
    pub sla_known: u64,
    pub sla_breached_yes: u64,
    pub first_response_mean_days: Option<f64>,
    pub resolution_days_mean: Option<f64>,
    pub satisfaction_mean: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Projection<T> {
    pub status: SupportStatus,
    pub rows_seen: u64,
    pub rows_rejected: u64,
    pub available_metrics: Vec<&'static str>,
    pub missing_metrics: Vec<&'static str>,
    pub aggregates: Vec<T>,
}

#[derive(Debug, Default)]
struct ContactAccum {
    count: u64,
    resolved_known: u64,
    resolved_yes: u64,
    followup_known: u64,
    followup_yes: u64,
    escalated_known: u64,
    escalated_yes: u64,
    duration_sum: f64,
    duration_count: u64,
    wait_sum: f64,
    wait_count: u64,
}

#[derive(Debug, Default)]
struct ComplaintAccum {
    count: u64,
    sla_known: u64,
    sla_yes: u64,
    first_response_sum: f64,
    first_response_count: u64,
    resolution_sum: f64,
    resolution_count: u64,
    satisfaction_sum: f64,
    satisfaction_count: u64,
}

pub fn project_contact_csvs<R: Read>(
    readers: impl IntoIterator<Item = R>,
) -> Projection<ContactAggregate> {
    let mut aggregates: BTreeMap<(String, ContactCategory, Channel), ContactAccum> =
        BTreeMap::new();
    let mut rows_seen = 0;
    let mut rows_rejected = 0;
    let mut missing = Vec::new();
    let mut saw_partition = false;
    let mut partition_metrics = Vec::new();

    for input in readers {
        saw_partition = true;
        let mut reader = csv::ReaderBuilder::new().flexible(true).from_reader(input);
        let headers = match reader.headers() {
            Ok(headers) => header_positions(headers),
            Err(_) => {
                missing.extend([
                    "interaction_date",
                    "reason_category|contact_reason",
                    "channel",
                ]);
                continue;
            }
        };
        let date_idx = index(&headers, &["interaction_date"]);
        let category_idx = index(&headers, &["reason_category", "contact_reason"]);
        let channel_idx = index(&headers, &["channel"]);
        let mut current_metrics = BTreeSet::new();
        for (name, found) in [
            ("interaction_date", date_idx.is_some()),
            ("reason_category|contact_reason", category_idx.is_some()),
            ("channel", channel_idx.is_some()),
        ] {
            if !found {
                missing.push(name);
            }
        }
        if date_idx.is_none() || category_idx.is_none() || channel_idx.is_none() {
            continue;
        }

        let resolved_idx = index(&headers, &["was_resolved"]);
        let followup_idx = index(&headers, &["requires_followup"]);
        let escalated_idx = index(&headers, &["was_escalated"]);
        let duration_idx = index(&headers, &["duration_seconds"]);
        let wait_idx = index(&headers, &["wait_time_seconds"]);
        for (name, found) in [
            ("was_resolved", resolved_idx.is_some()),
            ("requires_followup", followup_idx.is_some()),
            ("was_escalated", escalated_idx.is_some()),
            ("duration_seconds", duration_idx.is_some()),
            ("wait_time_seconds", wait_idx.is_some()),
        ] {
            if found {
                current_metrics.insert(name);
            }
        }
        partition_metrics.push(current_metrics);

        for record in reader.records() {
            rows_seen += 1;
            let Ok(record) = record else {
                rows_rejected += 1;
                continue;
            };
            let Some(period) = date_period(value(&record, date_idx)) else {
                rows_rejected += 1;
                continue;
            };
            let category = normalize_category(value(&record, category_idx));
            let channel = normalize_channel(value(&record, channel_idx));
            let row = aggregates.entry((period, category, channel)).or_default();
            row.count += 1;
            add_bool(
                value(&record, resolved_idx),
                &mut row.resolved_known,
                &mut row.resolved_yes,
            );
            add_bool(
                value(&record, followup_idx),
                &mut row.followup_known,
                &mut row.followup_yes,
            );
            add_bool(
                value(&record, escalated_idx),
                &mut row.escalated_known,
                &mut row.escalated_yes,
            );
            add_number(
                value(&record, duration_idx),
                &mut row.duration_sum,
                &mut row.duration_count,
                None,
            );
            add_number(
                value(&record, wait_idx),
                &mut row.wait_sum,
                &mut row.wait_count,
                None,
            );
        }
    }

    let metrics = [
        "was_resolved",
        "requires_followup",
        "was_escalated",
        "duration_seconds",
        "wait_time_seconds",
    ];
    let available_metrics = metrics
        .iter()
        .copied()
        .filter(|m| {
            !partition_metrics.is_empty()
                && partition_metrics.iter().all(|fields| fields.contains(m))
        })
        .collect::<Vec<_>>();
    let missing_metrics = metrics
        .iter()
        .copied()
        .filter(|m| {
            partition_metrics.is_empty()
                || partition_metrics.iter().any(|fields| !fields.contains(m))
        })
        .collect::<Vec<_>>();
    let status = support_status(saw_partition, missing, !available_metrics.is_empty());
    let projected = if status == SupportStatus::Supported {
        aggregates
            .into_iter()
            .map(|((period, category, channel), row)| ContactAggregate {
                period,
                category,
                channel,
                contact_count: row.count,
                resolved_known: row.resolved_known,
                resolved_yes: row.resolved_yes,
                followup_known: row.followup_known,
                followup_yes: row.followup_yes,
                escalated_known: row.escalated_known,
                escalated_yes: row.escalated_yes,
                duration_mean_seconds: mean(row.duration_sum, row.duration_count),
                wait_mean_seconds: mean(row.wait_sum, row.wait_count),
            })
            .collect()
    } else {
        Vec::new()
    };
    Projection {
        status,
        rows_seen,
        rows_rejected,
        available_metrics,
        missing_metrics,
        aggregates: projected,
    }
}

pub fn project_complaint_csvs<R: Read>(
    readers: impl IntoIterator<Item = R>,
) -> Projection<ComplaintAggregate> {
    let mut aggregates: BTreeMap<(String, ContactCategory, Channel), ComplaintAccum> =
        BTreeMap::new();
    let mut rows_seen = 0;
    let mut rows_rejected = 0;
    let mut missing = Vec::new();
    let mut saw_partition = false;
    let mut partition_metrics = Vec::new();

    for input in readers {
        saw_partition = true;
        let mut reader = csv::ReaderBuilder::new().flexible(true).from_reader(input);
        let headers = match reader.headers() {
            Ok(headers) => header_positions(headers),
            Err(_) => {
                missing.extend(["creation_date", "category", "reception_channel"]);
                continue;
            }
        };
        let date_idx = index(&headers, &["creation_date"]);
        let category_idx = index(&headers, &["category"]);
        let channel_idx = index(&headers, &["reception_channel"]);
        let mut current_metrics = BTreeSet::new();
        for (name, found) in [
            ("creation_date", date_idx.is_some()),
            ("category", category_idx.is_some()),
            ("reception_channel", channel_idx.is_some()),
        ] {
            if !found {
                missing.push(name);
            }
        }
        if date_idx.is_none() || category_idx.is_none() || channel_idx.is_none() {
            continue;
        }

        let sla_idx = index(&headers, &["sla_breached"]);
        let first_response_idx = index(&headers, &["first_response_date"]);
        let resolution_idx = index(&headers, &["resolution_days"]);
        let satisfaction_idx = index(&headers, &["resolution_satisfaction"]);
        for (name, found) in [
            ("sla_breached", sla_idx.is_some()),
            ("first_response_date", first_response_idx.is_some()),
            ("resolution_days", resolution_idx.is_some()),
            ("resolution_satisfaction", satisfaction_idx.is_some()),
        ] {
            if found {
                current_metrics.insert(name);
            }
        }
        partition_metrics.push(current_metrics);

        for record in reader.records() {
            rows_seen += 1;
            let Ok(record) = record else {
                rows_rejected += 1;
                continue;
            };
            let Some(period) = date_period(value(&record, date_idx)) else {
                rows_rejected += 1;
                continue;
            };
            let category = normalize_category(value(&record, category_idx));
            let channel = normalize_channel(value(&record, channel_idx));
            let row = aggregates.entry((period, category, channel)).or_default();
            row.count += 1;
            add_bool(
                value(&record, sla_idx),
                &mut row.sla_known,
                &mut row.sla_yes,
            );
            if let (Some(created), Some(response)) = (
                date_ordinal(value(&record, date_idx)),
                date_ordinal(value(&record, first_response_idx)),
            ) {
                if response >= created {
                    row.first_response_sum += (response - created) as f64;
                    row.first_response_count += 1;
                }
            }
            add_number(
                value(&record, resolution_idx),
                &mut row.resolution_sum,
                &mut row.resolution_count,
                None,
            );
            // Satisfaction is accepted only on the common 1..=5 survey scale.
            add_number(
                value(&record, satisfaction_idx),
                &mut row.satisfaction_sum,
                &mut row.satisfaction_count,
                Some((1.0, 5.0)),
            );
        }
    }

    let metrics = [
        "sla_breached",
        "first_response_date",
        "resolution_days",
        "resolution_satisfaction",
    ];
    let available_metrics = metrics
        .iter()
        .copied()
        .filter(|m| {
            !partition_metrics.is_empty()
                && partition_metrics.iter().all(|fields| fields.contains(m))
        })
        .collect::<Vec<_>>();
    let missing_metrics = metrics
        .iter()
        .copied()
        .filter(|m| {
            partition_metrics.is_empty()
                || partition_metrics.iter().any(|fields| !fields.contains(m))
        })
        .collect::<Vec<_>>();
    let status = support_status(saw_partition, missing, !available_metrics.is_empty());
    let projected = if status == SupportStatus::Supported {
        aggregates
            .into_iter()
            .map(|((period, category, channel), row)| ComplaintAggregate {
                period,
                category,
                channel,
                complaint_count: row.count,
                sla_known: row.sla_known,
                sla_breached_yes: row.sla_yes,
                first_response_mean_days: mean(row.first_response_sum, row.first_response_count),
                resolution_days_mean: mean(row.resolution_sum, row.resolution_count),
                satisfaction_mean: mean(row.satisfaction_sum, row.satisfaction_count),
            })
            .collect()
    } else {
        Vec::new()
    };
    Projection {
        status,
        rows_seen,
        rows_rejected,
        available_metrics,
        missing_metrics,
        aggregates: projected,
    }
}

fn support_status(
    saw_partition: bool,
    missing: Vec<&'static str>,
    has_metric: bool,
) -> SupportStatus {
    let mut missing = missing;
    missing.sort_unstable();
    missing.dedup();
    if !saw_partition || !missing.is_empty() || !has_metric {
        if !saw_partition {
            missing.push("csv_partition");
        }
        if !has_metric {
            missing.push("supported_metric_field");
        }
        SupportStatus::Unsupported {
            missing_fields: missing,
        }
    } else {
        SupportStatus::Supported
    }
}

fn header_positions(headers: &csv::StringRecord) -> BTreeMap<String, usize> {
    headers
        .iter()
        .enumerate()
        .map(|(i, name)| (name.trim().to_ascii_lowercase(), i))
        .collect()
}
fn index(headers: &BTreeMap<String, usize>, options: &[&str]) -> Option<usize> {
    options.iter().find_map(|name| headers.get(*name).copied())
}
fn value<'a>(record: &'a csv::StringRecord, index: Option<usize>) -> Option<&'a str> {
    index
        .and_then(|index| record.get(index))
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn normalize_category(value: Option<&str>) -> ContactCategory {
    let Some(value) = value else {
        return ContactCategory::Unclassified;
    };
    match value.trim().to_ascii_lowercase().as_str() {
        "queja" | "reclamo" | "complaint" => ContactCategory::Complaint,
        "transaccional" | "transaccional " | "transactional" | "transaction" => {
            ContactCategory::Transactional
        }
        "tecnico" | "técnico" | "technical" | "technical issue" => ContactCategory::Technical,
        "consulta general" | "consulta_general" | "general inquiry" | "general_inquiry" => {
            ContactCategory::GeneralInquiry
        }
        "producto" | "product" => ContactCategory::Product,
        "cuenta" | "account" => ContactCategory::Account,
        "tarjeta" | "card" => ContactCategory::Card,
        "prestamo" | "préstamo" | "loan" | "credit" => ContactCategory::Loan,
        "otro" | "other" => ContactCategory::Other,
        _ => ContactCategory::Unclassified,
    }
}

fn normalize_channel(value: Option<&str>) -> Channel {
    match value.unwrap_or("").trim().to_ascii_lowercase().as_str() {
        "phone" | "telefono" | "teléfono" | "call" => Channel::Phone,
        "web" | "website" => Channel::Web,
        "chat" | "live chat" => Channel::Chat,
        "email" | "correo" => Channel::Email,
        "branch" | "sucursal" | "in-person" => Channel::Branch,
        "mobile app" | "mobile_app" | "app" => Channel::MobileApp,
        _ => Channel::Other,
    }
}

fn date_period(value: Option<&str>) -> Option<String> {
    let value = value?;
    let date = value.get(..10)?;
    date_ordinal(Some(date))?;
    Some(date[..7].to_owned())
}

fn date_ordinal(value: Option<&str>) -> Option<i64> {
    let value = value?;
    let date = value.get(..10)?;
    let bytes = date.as_bytes();
    if bytes.len() != 10
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || !bytes
            .iter()
            .enumerate()
            .all(|(i, b)| i == 4 || i == 7 || b.is_ascii_digit())
    {
        return None;
    }
    let year = date.get(..4)?.parse::<i64>().ok()?;
    let month = date.get(5..7)?.parse::<i64>().ok()?;
    let day = date.get(8..10)?.parse::<i64>().ok()?;
    if !(1..=12).contains(&month) || day < 1 {
        return None;
    }
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let month_days = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    if day > month_days[(month - 1) as usize] {
        return None;
    }
    let adjusted_year = year - i64::from(month <= 2);
    let era = if adjusted_year >= 0 {
        adjusted_year
    } else {
        adjusted_year - 399
    } / 400;
    let year_of_era = adjusted_year - era * 400;
    let adjusted_month = month + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * adjusted_month + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    Some(era * 146097 + day_of_era)
}

fn parse_bool(value: Option<&str>) -> Option<bool> {
    match value?.trim().to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" => Some(true),
        "false" | "0" | "no" => Some(false),
        _ => None,
    }
}
fn add_bool(value: Option<&str>, known: &mut u64, yes: &mut u64) {
    if let Some(value) = parse_bool(value) {
        *known += 1;
        if value {
            *yes += 1;
        }
    }
}
fn add_number(value: Option<&str>, sum: &mut f64, count: &mut u64, bounds: Option<(f64, f64)>) {
    let Some(value) = value
        .and_then(|v| v.parse::<f64>().ok())
        .filter(|n| n.is_finite() && *n >= 0.0)
    else {
        return;
    };
    if bounds.is_some_and(|(min, max)| value < min || value > max) {
        return;
    }
    *sum += value;
    *count += 1;
}
fn mean(sum: f64, count: u64) -> Option<f64> {
    (count > 0).then(|| sum / count as f64)
}
