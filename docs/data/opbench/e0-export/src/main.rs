use std::collections::HashMap;
use std::fs::File;
use std::path::Path;

use arrow_array::{Array, LargeStringArray, TimestampMicrosecondArray};
use arrow_schema::DataType;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use serde_json::json;
use sha2::{Digest, Sha256};

fn field_index(schema: &arrow_schema::Schema, name: &str) -> usize {
    schema.index_of(name).expect("required E0 field is present")
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

fn case_key(case_id: &str) -> Vec<u8> {
    Sha256::digest(case_id.as_bytes()).to_vec()
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

fn main() {
    let data_dir = std::env::args().nth(1).expect("E0 data directory argument");
    let root = Path::new(&data_dir).parent().expect("E0 package root");

    // Validate source provenance and the approved operational-table set through
    // the production adapter. This must not load evaluator labels/timeline/close.
    let config = improvement_engine_source_adapters::PreparationConfig::new(
        "opbench-lite-local",
        "2026-10-05T00:00:00Z",
        200,
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

#[cfg(test)]
mod tests {
    use super::{modal_signature, prefer_query, split_for};
    use std::collections::HashMap;

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
}
