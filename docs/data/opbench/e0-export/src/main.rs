use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::path::Path;

use arrow_array::{Array, LargeStringArray, StringArray, TimestampMicrosecondArray};
use arrow_schema::{DataType, TimeUnit};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use serde_json::json;
use sha2::{Digest, Sha256};

const AS_OF_UTC: &str = "2026-10-05T00:00:00Z";
const AS_OF_EPOCH_MICROS: i64 = 1_791_158_400_000_000;
const DISCOVERY_CASES: usize = 200;
const EXPECTED_ELIGIBLE_CASES: usize = 2_000;

#[derive(Clone, Debug, PartialEq, Eq)]
struct E0Case {
    internal_case_id: String,
    case_hash: Vec<u8>,
    opened_at_micros: i64,
    complaint_hash: Option<Vec<u8>>,
}

#[derive(Clone, Debug)]
struct E0Query {
    case_hash: Vec<u8>,
    event_time_micros: i64,
    ordinal: u64,
    signature: String,
}

enum CliMode {
    Legacy(String),
    V2 {
        data_dir: String,
        complaint_hashes_path: String,
    },
}

fn parse_args(args: &[String]) -> Result<CliMode, &'static str> {
    match args {
        [data_dir] if !data_dir.is_empty() => Ok(CliMode::Legacy(data_dir.clone())),
        [data_dir, flag, path] if !data_dir.is_empty() && flag == "--v2" && !path.is_empty() => {
            Ok(CliMode::V2 {
                data_dir: data_dir.clone(),
                complaint_hashes_path: path.clone(),
            })
        }
        _ => Err("expected <data-dir> [--v2 <complaint-digest-file>]"),
    }
}

fn field_index(schema: &arrow_schema::Schema, name: &str) -> usize {
    schema.index_of(name).expect("required E0 field is present")
}

fn checked_field_index(schema: &arrow_schema::Schema, name: &str) -> Result<usize, &'static str> {
    schema
        .index_of(name)
        .map_err(|_| "E0 v2 source schema is missing a required column")
}

fn is_supported_string_type(data_type: &DataType) -> bool {
    matches!(data_type, DataType::Utf8 | DataType::LargeUtf8)
}

fn is_utc_microsecond_timestamp(data_type: &DataType) -> bool {
    matches!(
        data_type,
        DataType::Timestamp(TimeUnit::Microsecond, timezone)
            if timezone.as_deref().is_none_or(|value| value == "UTC")
    )
}

fn string_value(array: &dyn Array, row: usize) -> Result<&str, &'static str> {
    if let Some(values) = array.as_any().downcast_ref::<StringArray>() {
        return Ok(values.value(row));
    }
    if let Some(values) = array.as_any().downcast_ref::<LargeStringArray>() {
        return Ok(values.value(row));
    }
    Err("E0 v2 source string column type mismatch")
}

fn read_batches(path: &Path) -> impl Iterator<Item = arrow_array::RecordBatch> {
    let file = File::open(path).expect("open permitted E0 operational table");
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)
        .expect("read permitted E0 operational table metadata");
    builder
        .build()
        .expect("build E0 table reader")
        .map(|batch| batch.expect("read E0 batch"))
}

fn read_batches_v2(path: &Path) -> Result<Vec<arrow_array::RecordBatch>, &'static str> {
    let file = File::open(path).map_err(|_| "could not open E0 v2 source table")?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)
        .map_err(|_| "could not read E0 v2 source table metadata")?;
    builder
        .build()
        .map_err(|_| "could not build E0 v2 source table reader")?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "could not read E0 v2 source table batches")
}

fn case_key(case_id: &str) -> Vec<u8> {
    Sha256::digest(case_id.as_bytes()).to_vec()
}

fn validate_identifier(value: &str) -> Result<&str, &'static str> {
    if value.is_empty() || value.trim() != value {
        return Err("E0 identifier is blank or has surrounding whitespace");
    }
    Ok(value)
}

fn deduplicate_cases(cases: &[E0Case]) -> Result<Vec<E0Case>, &'static str> {
    let mut unique: HashMap<Vec<u8>, E0Case> = HashMap::new();
    for case in cases {
        validate_identifier(&case.internal_case_id)?;
        if case.case_hash.is_empty() || case.complaint_hash.as_ref().is_none_or(Vec::is_empty) {
            return Err("eligible E0 case is missing a required key or complaint linkage");
        }
        match unique.get(&case.case_hash) {
            Some(prior) if prior == case => continue,
            Some(_) => return Err("conflicting duplicate E0 case key"),
            None => {
                unique.insert(case.case_hash.clone(), case.clone());
            }
        }
    }
    Ok(unique.into_values().collect())
}

fn require_expected_case_count(count: usize) -> Result<(), &'static str> {
    if count != EXPECTED_ELIGIBLE_CASES {
        return Err("E0 v2 expected exactly 2000 eligible unique cases at the frozen cutoff");
    }
    Ok(())
}

fn is_at_or_before_cutoff(timestamp_micros: i64) -> bool {
    timestamp_micros <= AS_OF_EPOCH_MICROS
}

fn split_for(case_id: &str) -> &'static str {
    let digest = Sha256::digest(format!("opbench-lite:v1:e0:{case_id}").as_bytes());
    if digest[0] & 1 == 0 {
        "discovery"
    } else {
        "replication"
    }
}

fn prefer_query(candidate: &(i64, u64, String), existing: &(i64, u64, String)) -> bool {
    (candidate.0, candidate.1) < (existing.0, existing.1)
}

fn modal_signature(counts: HashMap<String, usize>) -> Option<String> {
    counts
        .into_iter()
        .min_by(|(sig_a, count_a), (sig_b, count_b)| {
            count_b.cmp(count_a).then_with(|| sig_a.cmp(sig_b))
        })
        .map(|(signature, _)| signature)
}

fn run_legacy(data_dir: &str) {
    let root = Path::new(&data_dir).parent().expect("E0 package root");

    // Validate source provenance and the approved operational-table set through
    // the production adapter. This must not load evaluator labels/timeline/close.
    let config = improvement_engine_source_adapters::PreparationConfig::new(
        "opbench-lite-local",
        AS_OF_UTC,
        DISCOVERY_CASES,
    )
    .expect("valid local preparation configuration");
    improvement_engine_source_adapters::prepare_e0_package(root, &config)
        .expect("E0 package must pass source-adapter validation");

    let case_path = Path::new(&data_dir).join("case.parquet");
    let mut case_batches = read_batches(&case_path);
    let first_case_batch = case_batches.next().expect("case table is non-empty");
    let case_schema = first_case_batch.schema();
    assert_eq!(
        case_schema
            .field(field_index(&case_schema, "case_id"))
            .data_type(),
        &DataType::LargeUtf8
    );
    let case_id_index = field_index(&case_schema, "case_id");
    let mut case_splits: HashMap<Vec<u8>, &'static str> = HashMap::new();
    for batch in std::iter::once(first_case_batch).chain(case_batches) {
        let ids = batch
            .column(case_id_index)
            .as_any()
            .downcast_ref::<LargeStringArray>()
            .expect("case_id is a string");
        for row in 0..batch.num_rows() {
            if ids.is_null(row) {
                continue;
            }
            let id = ids.value(row);
            if id.is_empty() {
                continue;
            }
            case_splits.insert(case_key(id), split_for(id));
        }
    }

    let query_path = Path::new(&data_dir).join("copilot_query.parquet");
    let mut leading: HashMap<Vec<u8>, (i64, u64, String)> = HashMap::new();
    let mut ordinal = 0_u64;
    for batch in read_batches(&query_path) {
        let schema = batch.schema();
        let case_idx = field_index(&schema, "case_id");
        let time_idx = field_index(&schema, "event_time");
        let sig_idx = field_index(&schema, "query_signature");
        assert_eq!(
            schema.field(time_idx).data_type(),
            &DataType::Timestamp(arrow_schema::TimeUnit::Microsecond, None)
        );
        let cases = batch
            .column(case_idx)
            .as_any()
            .downcast_ref::<LargeStringArray>()
            .expect("query case_id is a string");
        let times = batch
            .column(time_idx)
            .as_any()
            .downcast_ref::<TimestampMicrosecondArray>()
            .expect("query event_time is microsecond timestamp");
        let signatures = batch
            .column(sig_idx)
            .as_any()
            .downcast_ref::<LargeStringArray>()
            .expect("query signature is a string");
        for row in 0..batch.num_rows() {
            let current_ordinal = ordinal;
            ordinal += 1;
            if cases.is_null(row) || times.is_null(row) || signatures.is_null(row) {
                continue;
            }
            let key = case_key(cases.value(row));
            if !case_splits.contains_key(&key) {
                continue;
            }
            let signature = signatures.value(row);
            if signature.is_empty() {
                continue;
            }
            let candidate = (times.value(row), current_ordinal, signature.to_owned());
            match leading.get(&key) {
                Some(existing) if !prefer_query(&candidate, existing) => {}
                _ => {
                    leading.insert(key, candidate);
                }
            }
        }
    }

    let mut discovery_signatures: HashMap<String, usize> = HashMap::new();
    for (key, (_, _, signature)) in &leading {
        if case_splits.get(key) == Some(&"discovery") {
            *discovery_signatures.entry(signature.clone()).or_default() += 1;
        }
    }
    let selected = modal_signature(discovery_signatures);

    let mut halves = [(0_u64, 0_u64); 2]; // selected count, query-bearing cases
    if let Some(selected) = selected {
        for (key, (_, _, signature)) in &leading {
            let Some(split) = case_splits.get(key) else {
                continue;
            };
            let index = usize::from(*split == "replication");
            halves[index].1 += 1;
            halves[index].0 += u64::from(signature == &selected);
        }
    }
    // Emit only aggregate sufficient statistics; IDs and signatures never leave this process.
    println!(
        "{}",
        json!({
            "discovery": {"selected": halves[0].0, "total": halves[0].1},
            "replication": {"selected": halves[1].0, "total": halves[1].1}
        })
    );
}

fn aggregate_v2(
    cases: &[E0Case],
    queries: &[E0Query],
    bank_complaint_hashes: &HashSet<Vec<u8>>,
    discovery_size: usize,
) -> Result<serde_json::Value, &'static str> {
    if cases.is_empty()
        || bank_complaint_hashes.is_empty()
        || discovery_size == 0
        || discovery_size >= cases.len()
    {
        return Err("invalid case counts for E0 v2 split");
    }
    let mut ordered = deduplicate_cases(cases)?;
    ordered.sort_by(|left, right| {
        left.opened_at_micros
            .cmp(&right.opened_at_micros)
            .then_with(|| left.internal_case_id.cmp(&right.internal_case_id))
    });
    if discovery_size >= ordered.len() {
        return Err("E0 v2 discovery split exceeds eligible case count");
    }

    let case_times: HashMap<Vec<u8>, i64> = ordered
        .iter()
        .map(|case| (case.case_hash.clone(), case.opened_at_micros))
        .collect();
    let mut first_queries: HashMap<Vec<u8>, (i64, u64, String)> = HashMap::new();
    for query in queries {
        let Some(opened_at) = case_times.get(&query.case_hash) else {
            continue;
        };
        if query.event_time_micros < *opened_at || query.signature.trim().is_empty() {
            continue;
        }
        let candidate = (
            query.event_time_micros,
            query.ordinal,
            query.signature.clone(),
        );
        match first_queries.get(&query.case_hash) {
            Some(existing) if !prefer_query(&candidate, existing) => {}
            _ => {
                first_queries.insert(query.case_hash.clone(), candidate);
            }
        }
    }

    let mut discovery_signatures = HashMap::new();
    for case in ordered.iter().take(discovery_size) {
        if let Some((_, _, signature)) = first_queries.get(&case.case_hash) {
            *discovery_signatures
                .entry(signature.clone())
                .or_insert(0usize) += 1;
        }
    }
    let selected =
        modal_signature(discovery_signatures).ok_or("no discovery query signature available")?;
    let mut counts = [(0u64, 0u64); 2];
    for (index, case) in ordered.iter().enumerate() {
        let split = usize::from(index >= discovery_size);
        counts[split].1 += 1;
        counts[split].0 += u64::from(
            first_queries
                .get(&case.case_hash)
                .is_some_and(|(_, _, signature)| signature == &selected),
        );
    }
    let eligible_cases = ordered.len();
    let complaint_ids_matched = ordered
        .iter()
        .filter(|case| {
            case.complaint_hash
                .as_ref()
                .is_some_and(|digest| bank_complaint_hashes.contains(digest))
        })
        .count();
    if complaint_ids_matched > eligible_cases {
        return Err("E0 complaint linkage numerator exceeds denominator");
    }
    Ok(serde_json::json!({
        "discovery": {"selected": counts[0].0, "total": counts[0].1},
        "replication": {"selected": counts[1].0, "total": counts[1].1},
        "complaint_ids_matched": complaint_ids_matched,
        "eligible_cases": eligible_cases,
    }))
}

fn parse_complaint_hashes(content: &str) -> Result<HashSet<Vec<u8>>, &'static str> {
    if content.is_empty() {
        return Err("bank complaint digest source is empty");
    }
    let mut hashes = HashSet::new();
    for line in content.lines() {
        let bytes = line.as_bytes();
        if bytes.len() != 64 {
            return Err("invalid complaint digest entry");
        }
        let mut digest = Vec::with_capacity(32);
        for pair in bytes.as_chunks::<2>().0 {
            let high = hex_value(pair[0]).ok_or("invalid complaint digest entry")?;
            let low = hex_value(pair[1]).ok_or("invalid complaint digest entry")?;
            digest.push((high << 4) | low);
        }
        hashes.insert(digest);
    }
    if hashes.is_empty() {
        return Err("bank complaint digest source is empty");
    }
    Ok(hashes)
}

fn hex_value(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn read_complaint_hashes(path: &Path) -> Result<HashSet<Vec<u8>>, &'static str> {
    let content = fs::read_to_string(path).map_err(|_| "could not read complaint digest file")?;
    parse_complaint_hashes(&content)
}

fn read_cases_v2(data_dir: &Path) -> Result<Vec<E0Case>, &'static str> {
    let case_path = data_dir.join("case.parquet");
    let batches = read_batches_v2(&case_path)?;
    let mut cases = Vec::new();
    for batch in batches {
        let schema = batch.schema();
        let case_idx = checked_field_index(&schema, "case_id")?;
        let time_idx = checked_field_index(&schema, "opened_at")?;
        let complaint_idx = checked_field_index(&schema, "complaint_id")?;
        if !is_supported_string_type(schema.field(case_idx).data_type())
            || !is_supported_string_type(schema.field(complaint_idx).data_type())
            || !is_utc_microsecond_timestamp(schema.field(time_idx).data_type())
        {
            return Err("E0 v2 case schema mismatch");
        }
        let times = batch
            .column(time_idx)
            .as_any()
            .downcast_ref::<TimestampMicrosecondArray>()
            .ok_or("E0 opened_at type mismatch")?;
        let ids = batch.column(case_idx).as_ref();
        let complaints = batch.column(complaint_idx).as_ref();
        for row in 0..batch.num_rows() {
            if ids.is_null(row) || times.is_null(row) {
                return Err("E0 case has missing case_id or opened_at");
            }
            let case_id = validate_identifier(string_value(ids, row)?)?;
            let opened_at = times.value(row);
            if !is_at_or_before_cutoff(opened_at) {
                continue;
            }
            if complaints.is_null(row) {
                return Err("eligible E0 case is missing complaint linkage");
            }
            let complaint_id = validate_identifier(string_value(complaints, row)?)
                .map_err(|_| "eligible E0 case has invalid complaint linkage")?;
            cases.push(E0Case {
                internal_case_id: case_id.to_owned(),
                case_hash: case_key(case_id),
                opened_at_micros: opened_at,
                complaint_hash: Some(case_key(complaint_id)),
            });
        }
    }
    let unique_cases = deduplicate_cases(&cases)?;
    require_expected_case_count(unique_cases.len())?;
    Ok(unique_cases)
}

fn read_queries_v2(data_dir: &Path, cases: &[E0Case]) -> Result<Vec<E0Query>, &'static str> {
    let known_cases: HashSet<Vec<u8>> = cases.iter().map(|case| case.case_hash.clone()).collect();
    let mut queries = Vec::new();
    let mut ordinal = 0u64;
    for batch in read_batches_v2(&data_dir.join("copilot_query.parquet"))? {
        let schema = batch.schema();
        let case_idx = checked_field_index(&schema, "case_id")?;
        let time_idx = checked_field_index(&schema, "event_time")?;
        let sig_idx = checked_field_index(&schema, "query_signature")?;
        if !is_supported_string_type(schema.field(case_idx).data_type())
            || !is_utc_microsecond_timestamp(schema.field(time_idx).data_type())
            || !is_supported_string_type(schema.field(sig_idx).data_type())
        {
            return Err("E0 v2 copilot_query schema mismatch");
        }
        let times = batch
            .column(time_idx)
            .as_any()
            .downcast_ref::<TimestampMicrosecondArray>()
            .ok_or("E0 query event_time type mismatch")?;
        let case_ids = batch.column(case_idx).as_ref();
        let signatures = batch.column(sig_idx).as_ref();
        for row in 0..batch.num_rows() {
            let current_ordinal = ordinal;
            ordinal += 1;
            if case_ids.is_null(row) || times.is_null(row) || signatures.is_null(row) {
                return Err("E0 query contains null required fields");
            }
            if !is_at_or_before_cutoff(times.value(row)) {
                continue;
            }
            let case_hash = case_key(validate_identifier(string_value(case_ids, row)?)?);
            if !known_cases.contains(&case_hash) {
                continue;
            }
            queries.push(E0Query {
                case_hash,
                event_time_micros: times.value(row),
                ordinal: current_ordinal,
                signature: string_value(signatures, row)?.to_owned(),
            });
        }
    }
    Ok(queries)
}

fn run_v2(data_dir: &str, complaint_hashes_path: &str) -> Result<serde_json::Value, &'static str> {
    let data_path = Path::new(data_dir);
    let root = data_path.parent().ok_or("E0 package root is unavailable")?;
    let config = improvement_engine_source_adapters::PreparationConfig::new(
        "opbench-lite-local",
        AS_OF_UTC,
        DISCOVERY_CASES,
    )
    .map_err(|_| "invalid E0 preparation configuration")?;
    improvement_engine_source_adapters::prepare_e0_package(root, &config)
        .map_err(|_| "E0 package failed source-adapter validation")?;
    let cases = read_cases_v2(data_path)?;
    let queries = read_queries_v2(data_path, &cases)?;
    let bank_complaint_hashes = read_complaint_hashes(Path::new(complaint_hashes_path))?;
    aggregate_v2(&cases, &queries, &bank_complaint_hashes, DISCOVERY_CASES)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match parse_args(&args) {
        Ok(CliMode::Legacy(data_dir)) => run_legacy(&data_dir),
        Ok(CliMode::V2 {
            data_dir,
            complaint_hashes_path,
        }) => match run_v2(&data_dir, &complaint_hashes_path) {
            Ok(result) => println!("{}", result),
            Err(message) => {
                eprintln!("OPBENCH E0 v2 failed safely: {message}");
                std::process::exit(2);
            }
        },
        Err(message) => {
            eprintln!("OPBENCH E0 exporter arguments invalid: {message}");
            std::process::exit(2);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CliMode, E0Case, E0Query, EXPECTED_ELIGIBLE_CASES, aggregate_v2, checked_field_index,
        deduplicate_cases, is_at_or_before_cutoff, modal_signature, parse_args,
        parse_complaint_hashes, prefer_query, read_batches_v2, read_cases_v2,
        require_expected_case_count, split_for, validate_identifier,
    };
    use arrow_array::{ArrayRef, RecordBatch, StringArray, TimestampMicrosecondArray};
    use arrow_schema::{DataType, Field, Schema, TimeUnit};
    use parquet::arrow::ArrowWriter;
    use std::collections::{HashMap, HashSet};
    use std::fs::{self, File};
    use std::path::Path;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    fn temporary_case_dir() -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "opbench-e0-read-cases-{}-{}",
            std::process::id(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).expect("create synthetic test directory");
        path
    }

    fn write_case_parquet(path: &Path, rows: &[(String, i64, String)]) {
        let schema = Arc::new(Schema::new(vec![
            Field::new("case_id", DataType::Utf8, false),
            Field::new(
                "opened_at",
                DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
                false,
            ),
            Field::new("complaint_id", DataType::Utf8, false),
        ]));
        let ids = rows.iter().map(|row| row.0.as_str()).collect::<Vec<_>>();
        let opened_at = rows.iter().map(|row| row.1).collect::<Vec<_>>();
        let complaint_ids = rows.iter().map(|row| row.2.as_str()).collect::<Vec<_>>();
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(StringArray::from(ids)) as ArrayRef,
                Arc::new(TimestampMicrosecondArray::from(opened_at).with_timezone("UTC")),
                Arc::new(StringArray::from(complaint_ids)),
            ],
        )
        .expect("valid synthetic case batch");
        let file = File::create(path).expect("create synthetic case parquet");
        let mut writer = ArrowWriter::try_new(file, schema, None).expect("create parquet writer");
        writer.write(&batch).expect("write synthetic case batch");
        writer.close().expect("close synthetic case parquet");
    }

    #[test]
    fn split_is_stable_and_domain_separated() {
        assert_eq!(split_for("case-17"), split_for("case-17"));
        assert!(matches!(split_for("case-17"), "discovery" | "replication"));
    }

    #[test]
    fn first_query_is_timestamp_then_source_ordinal() {
        let selected = (1_700_000_000, 4, "opaque-a".to_owned());
        assert!(prefer_query(
            &(1_699_999_999, 8, "opaque-b".to_owned()),
            &selected
        ));
        assert!(prefer_query(
            &(1_700_000_000, 3, "opaque-c".to_owned()),
            &selected
        ));
        assert!(!prefer_query(
            &(1_700_000_000, 5, "opaque-d".to_owned()),
            &selected
        ));
    }

    #[test]
    fn modal_ties_resolve_lexically_without_exposing_the_signature() {
        let counts = HashMap::from([("opaque-b".to_owned(), 8), ("opaque-a".to_owned(), 8)]);
        let selected = modal_signature(counts).expect("non-empty discovery signature set");
        assert_eq!(selected, "opaque-a");
        let output = serde_json::json!({"selected": 8, "total": 10}).to_string();
        assert!(!output.contains("opaque-a"));
    }

    #[test]
    fn v2_splits_by_opened_at_uses_all_cases_and_only_emits_counts() {
        // Deliberately reverse storage order: the first two by opened_at form discovery.
        let cases = vec![
            E0Case {
                internal_case_id: "case-4".into(),
                case_hash: vec![4],
                opened_at_micros: 40,
                complaint_hash: Some(vec![14]),
            },
            E0Case {
                internal_case_id: "case-2".into(),
                case_hash: vec![2],
                opened_at_micros: 20,
                complaint_hash: Some(vec![12]),
            },
            E0Case {
                internal_case_id: "case-1".into(),
                case_hash: vec![1],
                opened_at_micros: 10,
                complaint_hash: Some(vec![11]),
            },
            E0Case {
                internal_case_id: "case-3".into(),
                case_hash: vec![3],
                opened_at_micros: 30,
                complaint_hash: Some(vec![13]),
            },
        ];
        let queries = vec![
            E0Query {
                case_hash: vec![1],
                event_time_micros: 12,
                ordinal: 1,
                signature: "private-z".into(),
            },
            E0Query {
                case_hash: vec![1],
                event_time_micros: 11,
                ordinal: 2,
                signature: "private-a".into(),
            },
            E0Query {
                case_hash: vec![2],
                event_time_micros: 21,
                ordinal: 3,
                signature: "private-a".into(),
            },
            E0Query {
                case_hash: vec![3],
                event_time_micros: 31,
                ordinal: 4,
                signature: "private-a".into(),
            },
            // Case 4 intentionally has no query and must still count in holdout's denominator.
            E0Query {
                case_hash: vec![99],
                event_time_micros: 22,
                ordinal: 5,
                signature: "unknown-case".into(),
            },
        ];
        let bank_ids = HashSet::from([vec![11], vec![12], vec![13]]);
        let result =
            aggregate_v2(&cases, &queries, &bank_ids, 2).expect("valid synthetic aggregate");
        assert_eq!(result["discovery"]["selected"], 2);
        assert_eq!(result["discovery"]["total"], 2);
        assert_eq!(result["replication"]["selected"], 1);
        assert_eq!(result["replication"]["total"], 2);
        assert_eq!(result["complaint_ids_matched"], 3);
        assert_eq!(result["eligible_cases"], 4);
        assert_eq!(
            result
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect::<HashSet<_>>(),
            HashSet::from([
                "discovery",
                "replication",
                "complaint_ids_matched",
                "eligible_cases"
            ]),
        );
        let output = result.to_string();
        for private in ["private-a", "private-z", "unknown-case"] {
            assert!(!output.contains(private));
        }
        assert!(!output.contains("case_hash"));
    }

    #[test]
    fn v2_collapses_exact_case_duplicates_rejects_conflicts_and_requires_signature() {
        let case = E0Case {
            internal_case_id: "case-1".into(),
            case_hash: vec![1],
            opened_at_micros: 10,
            complaint_hash: Some(vec![9]),
        };
        let other = E0Case {
            internal_case_id: "case-2".into(),
            case_hash: vec![2],
            opened_at_micros: 20,
            complaint_hash: Some(vec![10]),
        };
        let bank_ids = HashSet::from([vec![9]]);
        let duplicate = vec![case.clone(), case.clone(), other.clone()];
        let query = E0Query {
            case_hash: vec![1],
            event_time_micros: 11,
            ordinal: 1,
            signature: "private".into(),
        };
        let collapsed = aggregate_v2(&duplicate, std::slice::from_ref(&query), &bank_ids, 1)
            .expect("exact duplicates collapse");
        assert_eq!(collapsed["eligible_cases"], 2);
        let conflict = E0Case {
            opened_at_micros: 12,
            ..case.clone()
        };
        assert!(
            aggregate_v2(
                &[case.clone(), conflict, other.clone()],
                &[query],
                &bank_ids,
                1
            )
            .is_err()
        );
        assert!(aggregate_v2(&[case.clone(), other.clone()], &[], &bank_ids, 1).is_err());
        assert!(aggregate_v2(&[case, other], &[], &HashSet::new(), 1).is_err());
    }

    #[test]
    fn v2_rejects_missing_complaint_linkage_instead_of_fabricating_zero() {
        let cases = vec![
            E0Case {
                internal_case_id: "case-1".into(),
                case_hash: vec![1],
                opened_at_micros: 10,
                complaint_hash: Some(vec![9]),
            },
            E0Case {
                internal_case_id: "case-2".into(),
                case_hash: vec![2],
                opened_at_micros: 20,
                complaint_hash: None,
            },
        ];
        let queries = vec![E0Query {
            case_hash: vec![1],
            event_time_micros: 11,
            ordinal: 1,
            signature: "private-signature".into(),
        }];
        assert!(aggregate_v2(&cases, &queries, &HashSet::from([vec![9]]), 1).is_err());
    }

    #[test]
    fn cli_keeps_legacy_mode_and_requires_exact_v2_digest_arguments() {
        assert!(
            matches!(parse_args(&["datos".into()]), Ok(CliMode::Legacy(path)) if path == "datos")
        );
        assert!(matches!(
            parse_args(&["datos".into(), "--v2".into(), "hashes.txt".into()]),
            Ok(CliMode::V2 { data_dir, complaint_hashes_path }) if data_dir == "datos" && complaint_hashes_path == "hashes.txt"
        ));
        assert!(parse_args(&["datos".into(), "--v2".into()]).is_err());
        assert!(parse_args(&["datos".into(), "--unknown".into(), "x".into()]).is_err());
    }

    #[test]
    fn v2_opened_at_ties_use_private_case_id_order_not_storage_or_hash_order() {
        let cases = vec![
            E0Case {
                internal_case_id: "z-case".into(),
                case_hash: vec![1],
                opened_at_micros: 10,
                complaint_hash: Some(vec![11]),
            },
            E0Case {
                internal_case_id: "a-case".into(),
                case_hash: vec![9],
                opened_at_micros: 10,
                complaint_hash: Some(vec![12]),
            },
        ];
        let queries = vec![
            E0Query {
                case_hash: vec![1],
                event_time_micros: 11,
                ordinal: 1,
                signature: "holdout".into(),
            },
            E0Query {
                case_hash: vec![9],
                event_time_micros: 11,
                ordinal: 2,
                signature: "discovery".into(),
            },
        ];
        let result =
            aggregate_v2(&cases, &queries, &HashSet::from([vec![11], vec![12]]), 1).unwrap();
        assert_eq!(result["discovery"]["selected"], 1);
        assert_eq!(result["replication"]["selected"], 0);
    }

    #[test]
    fn v2_population_is_checked_after_case_level_deduplication() {
        let mut cases: Vec<E0Case> = (0..EXPECTED_ELIGIBLE_CASES)
            .map(|index| E0Case {
                internal_case_id: format!("case-{index:04}"),
                case_hash: index.to_le_bytes().to_vec(),
                opened_at_micros: index as i64,
                complaint_hash: Some(vec![1]),
            })
            .collect();
        let duplicate = cases[17].clone();
        cases.push(duplicate);
        let deduped = deduplicate_cases(&cases).expect("exact duplicate collapses");
        assert_eq!(deduped.len(), EXPECTED_ELIGIBLE_CASES);
        assert!(require_expected_case_count(deduped.len()).is_ok());

        cases.remove(18);
        cases.push(cases[18].clone());
        let wrong_population =
            deduplicate_cases(&cases).expect("duplicate is removed before count");
        assert_eq!(wrong_population.len(), EXPECTED_ELIGIBLE_CASES - 1);
        assert!(require_expected_case_count(wrong_population.len()).is_err());
    }

    #[test]
    fn source_ids_are_never_trimmed_and_padded_ids_fail_closed() {
        assert_eq!(validate_identifier("case-1").unwrap(), "case-1");
        assert!(validate_identifier(" case-1").is_err());
        assert!(validate_identifier("case-1 ").is_err());
        assert!(validate_identifier("   ").is_err());
    }

    #[test]
    fn digest_file_parser_requires_nonempty_exact_ascii_sha256_lines() {
        let valid = format!("{}\n", "ab".repeat(32));
        let parsed = parse_complaint_hashes(&valid).expect("valid fixed-size digest");
        assert_eq!(parsed.len(), 1);
        assert!(parse_complaint_hashes("").is_err());
        assert!(parse_complaint_hashes("abc\n").is_err());
        assert!(parse_complaint_hashes(&format!("{}\n", "é".repeat(32))).is_err());
        assert!(read_batches_v2(Path::new("missing-opbench-table.parquet")).is_err());
    }

    #[test]
    fn v2_cutoff_includes_exact_boundary_and_excludes_later_timestamps() {
        assert!(is_at_or_before_cutoff(1_791_158_400_000_000));
        assert!(!is_at_or_before_cutoff(1_791_158_400_000_001));
    }

    #[test]
    fn v2_required_column_lookup_fails_closed_when_column_is_missing() {
        let schema = arrow_schema::Schema::empty();
        assert!(checked_field_index(&schema, "case_id").is_err());
    }

    #[test]
    fn v2_case_reader_accepts_source_adapter_utf8_utc_schema_and_dedupes_before_cap() {
        let directory = temporary_case_dir();
        let case_path = directory.join("case.parquet");
        let mut rows = (0..EXPECTED_ELIGIBLE_CASES)
            .map(|index| {
                (
                    format!("SYNTH-CASE-{index:04}"),
                    index as i64,
                    format!("SYNTH-COMPLAINT-{index:04}"),
                )
            })
            .collect::<Vec<_>>();
        rows.push(rows[17].clone());
        write_case_parquet(&case_path, &rows);

        let cases = read_cases_v2(&directory).expect("2000 unique cases after exact duplicate");
        assert_eq!(cases.len(), EXPECTED_ELIGIBLE_CASES);

        rows[18] = rows[17].clone();
        write_case_parquet(&case_path, &rows);
        assert!(read_cases_v2(&directory).is_err());

        rows[18].1 += 1;
        write_case_parquet(&case_path, &rows);
        assert!(read_cases_v2(&directory).is_err());
        fs::remove_dir_all(directory).expect("remove synthetic test directory");
    }
}

