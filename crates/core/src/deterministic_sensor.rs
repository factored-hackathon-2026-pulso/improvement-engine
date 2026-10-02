//! Deterministic, descriptive signals over sealed U08 lab results.
//!
//! The sensor performs no source access or query execution. It verifies the
//! lab receipts supplied by its caller, accounts explicitly for missing values,
//! and emits a reproducible descriptive metric—not a causal assertion.

use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

use crate::ArtifactReference;
use crate::local_lab::{QueryReceipt, QueryResult};

/// A bounded boolean-equality rate. Values other than the true/false tokens
/// are counted as missing, never coerced to an outcome.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct BooleanRateSpec {
    pub metric_id: String,
    pub field: String,
    pub positive_value: String,
}

impl BooleanRateSpec {
    pub fn new(
        metric_id: impl Into<String>,
        field: impl Into<String>,
        positive_value: impl Into<String>,
    ) -> Result<Self, SensorError> {
        let spec = Self {
            metric_id: metric_id.into(),
            field: field.into(),
            positive_value: positive_value.into(),
        };
        if !valid_identifier(&spec.metric_id)
            || !valid_identifier(&spec.field)
            || !matches!(spec.positive_value.as_str(), "true" | "false")
        {
            return Err(SensorError::InvalidSpec);
        }
        Ok(spec)
    }
}

/// A descriptive signal with counts, coverage and the receipts that prove the
/// observed rows. It deliberately contains no causal claim or opportunity score.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DeterministicSignal {
    pub metric_id: String,
    pub numerator: u64,
    pub denominator: u64,
    pub missing: u64,
    pub coverage_basis_points: u16,
    pub source_snapshot_ref: ArtifactReference,
    pub tenant_id: String,
    pub grant_id: String,
    pub authority_ref: String,
    pub source_contract_digest: String,
    pub source_digest: String,
    pub transform_digest: String,
    pub cutoff_unix_seconds: u64,
    pub query_receipts: Vec<QueryReceipt>,
    pub digest: String,
}

impl DeterministicSignal {
    #[must_use]
    pub fn has_valid_digest(&self) -> bool {
        let mut unsigned = self.clone();
        let expected = std::mem::take(&mut unsigned.digest);
        expected == digest_of(&unsigned)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SensorError {
    InvalidSpec,
    NoEvidence,
    ReceiptIntegrityDenied,
    MetricFieldMissing,
}

/// Stateless verifier and calculator for U12's first deterministic metric.
pub struct DeterministicSensor;

impl DeterministicSensor {
    pub fn measure(
        spec: &BooleanRateSpec,
        results: &[QueryResult],
    ) -> Result<DeterministicSignal, SensorError> {
        let first = results.first().ok_or(SensorError::NoEvidence)?;
        let baseline = first.receipt();
        if !baseline.has_valid_digest() {
            return Err(SensorError::ReceiptIntegrityDenied);
        }
        let mut numerator = 0_u64;
        let mut denominator = 0_u64;
        let mut missing = 0_u64;
        let mut query_receipts = Vec::with_capacity(results.len());
        let mut receipt_digests = BTreeSet::new();
        for (expected_sequence, result) in (1_u64..).zip(results) {
            let receipt = result.receipt();
            if !receipt.has_valid_digest()
                || !receipt.binds_rows(result.rows())
                || !receipt_digests.insert(receipt.digest.clone())
                || receipt.sequence != expected_sequence
                || receipt.session_id != baseline.session_id
                || receipt.run_id != baseline.run_id
                || receipt.tenant_id != baseline.tenant_id
                || receipt.grant_id != baseline.grant_id
                || receipt.authority_ref != baseline.authority_ref
                || receipt.source_snapshot_ref != baseline.source_snapshot_ref
                || receipt.source_contract_digest != baseline.source_contract_digest
                || receipt.source_digest != baseline.source_digest
                || receipt.transform_digest != baseline.transform_digest
                || receipt.source_approval_id != baseline.source_approval_id
                || receipt.cutoff_unix_seconds != baseline.cutoff_unix_seconds
            {
                return Err(SensorError::ReceiptIntegrityDenied);
            }
            for row in result.rows() {
                let Some(value) = row.get(&spec.field) else {
                    return Err(SensorError::MetricFieldMissing);
                };
                match value.as_str() {
                    "true" | "false" => {
                        denominator += 1;
                        if value == &spec.positive_value {
                            numerator += 1;
                        }
                    }
                    _ => missing += 1,
                }
            }
            query_receipts.push(receipt.clone());
        }
        let total = denominator + missing;
        let coverage_basis_points = denominator
            .checked_mul(10_000)
            .and_then(|scaled| scaled.checked_div(total))
            .unwrap_or(0) as u16;
        let mut signal = DeterministicSignal {
            metric_id: spec.metric_id.clone(),
            numerator,
            denominator,
            missing,
            coverage_basis_points,
            source_snapshot_ref: baseline.source_snapshot_ref.clone(),
            tenant_id: baseline.tenant_id.clone(),
            grant_id: baseline.grant_id.clone(),
            authority_ref: baseline.authority_ref.clone(),
            source_contract_digest: baseline.source_contract_digest.clone(),
            source_digest: baseline.source_digest.clone(),
            transform_digest: baseline.transform_digest.clone(),
            cutoff_unix_seconds: baseline.cutoff_unix_seconds,
            query_receipts,
            digest: String::new(),
        };
        signal.digest = digest_of(&signal);
        Ok(signal)
    }
}

fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'_'))
}

fn digest_of<T: Serialize>(value: &T) -> String {
    let bytes = serde_json::to_vec(value).expect("sensor evidence is serializable");
    format!("sha256:{:x}", Sha256::digest(bytes))
}
