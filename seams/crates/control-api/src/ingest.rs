//! Platform observation ingest (spec 32, contract `pulso-observations-2`): validation, ACK, quarantine.
//!
//! Wire rules mirror the Python double (`platform-sim/ingest_fixture`): strict batch model, JCS `batch_digest`,
//! `Idempotency-Key == batch_digest`, `202` only after commit, `200` replays the committed receipt, fast_poll CAS on
//! `expected_cursor_revision` (stale -> `409` before any write), rescan never moves the checkpoint, event identity
//! `(tenant, source, kind, level, native_event_id)` with `409` on a digest conflict.
//!
//! What the double does not do and this server does (the exporter does it on its own side in the double's world):
//! * **type quarantine** (catalog 1.2.0, embedded read-only): admitted types are accepted; denied, planned and unknown
//!   types (`release.*`, `team.*`...) are counted and quarantined as metadata, never the payload, and never fail the batch.
//!   A quarantined row still counts for sequence contiguity (the checkpoint is acked past it).
//! * **gap quarantine**: a hole in the domain `source_sequence` of a fast_poll batch (inside it, before it against the
//!   checkpoint, or up to the declared `to_seq`) that no `gap_suspected` finding in the same batch declares quarantines the
//!   WHOLE batch: nothing enters the ledger, the checkpoint stays, the receipt says `disposition: quarantined`.
//!   Declared holes are acked and kept as `open_gaps`; `late` rows inside a hole close it.
//! Core-event continuity (per-run engine sequences) is not checked here.
use crate::app::{App, Req, Resp, error, is_hex64, json_resp};
use crate::auth::Expect;
use core_client::canon::{jcs, sha256_hex};
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::sync::OnceLock;

pub const MAX_BATCH_BYTES: usize = 512 * 1024;
pub const MAX_BATCH_EVENTS: usize = 500;
const FINDING_CODES: [&str; 8] = ["bad_row", "denied_event_type", "unknown_event_type", "late_event", "gap_suspected", "turn_sequence_gap", "capability_profile", "dimension_snapshot"];

/// The catalog the platform source policy digests; embedded read-only so the two can never drift silently.
const CATALOG: &str = include_str!("../../../../platform-contract/event-catalog.json");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    Admitted,
    Denied,
    Planned,
    Unknown,
}

impl Class {
    fn name(self) -> &'static str {
        match self {
            Class::Admitted => "admitted",
            Class::Denied => "denied",
            Class::Planned => "planned",
            Class::Unknown => "unknown",
        }
    }
}

struct Catalog {
    by_type: BTreeMap<String, Class>,
    planned_prefixes: Vec<String>,
}

fn catalog() -> &'static Catalog {
    static C: OnceLock<Catalog> = OnceLock::new();
    C.get_or_init(|| {
        let v: Value = serde_json::from_str(CATALOG).expect("event-catalog.json");
        let by_type = v["event_types"]
            .as_array()
            .expect("event_types")
            .iter()
            .map(|e| {
                let class = match e["status"].as_str() {
                    Some("admitted") => Class::Admitted,
                    Some("denied") => Class::Denied,
                    _ => Class::Planned,
                };
                (e["event_type"].as_str().expect("event_type").to_string(), class)
            })
            .collect();
        let planned_prefixes = v["planned_prefixes"].as_array().into_iter().flatten().filter_map(|p| p.as_str().map(str::to_string)).collect();
        Catalog { by_type, planned_prefixes }
    })
}

/// Catalog 1.2.0 rules: listed types by status; announced prefixes are planned; everything else (including the legacy
/// `exporter.` prefix of 1.0.0) is unknown.
pub fn classify(event_type: &str) -> Class {
    let c = catalog();
    c.by_type.get(event_type).copied().unwrap_or_else(|| if c.planned_prefixes.iter().any(|p| event_type.starts_with(p.as_str())) { Class::Planned } else { Class::Unknown })
}

// ---- strict wire model (no extra keys, closed enums) -----------------------------------------------------------------
fn exact(m: &Map<String, Value>, required: &[&str], optional: &[&str]) -> bool {
    required.iter().all(|k| m.contains_key(*k)) && m.keys().all(|k| required.contains(&k.as_str()) || optional.contains(&k.as_str()))
}
fn is_ref(v: &Value) -> bool {
    v.as_object().is_some_and(|o| o.len() == 3 && ["id", "digest", "media_type"].iter().all(|k| o.get(*k).is_some_and(Value::is_string)))
}
fn ref_or_null(v: &Value) -> bool {
    v.is_null() || is_ref(v)
}
fn str_map_or_null(v: &Value) -> bool {
    v.is_null() || v.as_object().is_some_and(|o| o.values().all(Value::is_string))
}
fn int_or_null(v: &Value) -> bool {
    v.is_null() || v.is_i64() || v.is_u64()
}
fn one_of(v: &Value, set: &[&str]) -> bool {
    v.as_str().is_some_and(|s| set.contains(&s))
}

fn valid_event(e: &Value) -> bool {
    let Some(m) = e.as_object() else { return false };
    let req = ["kind", "level", "source_event", "native_event_id", "source_event_digest", "source_event_ref", "source_schema_ref", "source_run_ref", "source_sequence", "observed_at", "trace_refs", "coverage_marker"];
    exact(m, &req, &["episode_ref", "goal_ref", "layer_mapping_ref"])
        && one_of(&m["kind"], &["core_event", "platform_event"])
        && (m["level"].is_null() || one_of(&m["level"], &["engine_event", "outbound_event"]))
        && (m["source_event"].is_null() || m["source_event"].is_object())
        && m["native_event_id"].is_string()
        && m["source_event_digest"].is_string()
        && ref_or_null(&m["source_event_ref"])
        && is_ref(&m["source_schema_ref"])
        && (m["source_run_ref"].is_null() || m["source_run_ref"].is_string())
        && int_or_null(&m["source_sequence"])
        && m["observed_at"].is_string()
        && m["trace_refs"].as_array().is_some_and(|a| a.iter().all(is_ref))
        && (m["coverage_marker"].is_null() || one_of(&m["coverage_marker"], &["complete_run", "open_run", "gap", "late"]))
        && ["episode_ref", "goal_ref"].iter().all(|k| m.get(*k).is_none_or(str_map_or_null))
        && m.get("layer_mapping_ref").is_none_or(ref_or_null)
}

fn valid_receipt(r: &Value) -> bool {
    let Some(m) = r.as_object() else { return false };
    let req = ["run_id", "source_schema_ref", "source_artifact_ref", "verified_through_seq", "chain_head_hash", "source_artifact_digest", "check_result", "verifier_agent_core_sha", "verifier_contract_version", "checked_at"];
    exact(m, &req, &[])
        && ["run_id", "chain_head_hash", "source_artifact_digest", "verifier_agent_core_sha", "verifier_contract_version", "checked_at"].iter().all(|k| m[*k].is_string())
        && is_ref(&m["source_schema_ref"])
        && is_ref(&m["source_artifact_ref"])
        && m["verified_through_seq"].is_i64()
        && m["check_result"].is_object()
}

/// Strict batch model; fills the defaulted optional keys with null so the digest recompute matches the double's `model_dump`.
fn validate_batch(v: Value) -> Option<Map<String, Value>> {
    let Value::Object(mut m) = v else { return None };
    let req = ["contract_version", "source_id", "tenant_id", "partition", "scan_mode", "expected_cursor_revision", "from_seq", "to_seq", "cursor", "events", "verification_receipts", "batch_digest"];
    if !exact(&m, &req, &["cut_ref"]) {
        return None;
    }
    let ok = m["contract_version"] == "pulso-observations-2"
        && ["source_id", "tenant_id", "partition", "cursor", "batch_digest"].iter().all(|k| m[*k].is_string())
        && one_of(&m["scan_mode"], &["fast_poll", "rescan"])
        && ["expected_cursor_revision", "from_seq", "to_seq"].iter().all(|k| int_or_null(&m[*k]))
        && m.get("cut_ref").is_none_or(ref_or_null)
        && m["events"].as_array().is_some_and(|a| a.iter().all(valid_event))
        && m["verification_receipts"].as_array().is_some_and(|a| a.iter().all(valid_receipt));
    if !ok {
        return None;
    }
    m.entry("cut_ref").or_insert(Value::Null);
    for e in m["events"].as_array_mut().into_iter().flatten() {
        let o = e.as_object_mut().expect("validated");
        for k in ["episode_ref", "goal_ref", "layer_mapping_ref"] {
            o.entry(k).or_insert(Value::Null);
        }
    }
    Some(m)
}

/// Storage key of several free-text parts: a JSON array, so no part can forge another by containing the separator.
fn jkey(parts: &[&str]) -> String {
    serde_json::to_string(parts).expect("strings serialise")
}

fn cursor_ok(c: &str) -> bool {
    (1..=128).contains(&c.len()) && c.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b':' | b'-'))
}

// ---- classification of one observation ------------------------------------------------------------------------------
enum Kind {
    /// Domain row of the platform event log (the only kind with source-sequence continuity).
    Domain { event_type: String },
    Finding { code: String },
    /// Anything else (core events, refs without inline source_event): accepted as is.
    Other,
}

fn kind_of(e: &Value) -> Kind {
    if e["kind"] != "platform_event" {
        return Kind::Other;
    }
    let Some(se) = e["source_event"].as_object() else { return Kind::Other };
    match se.get("kind").and_then(Value::as_str) {
        Some("exporter_finding") => Kind::Finding { code: se.get("finding_code").and_then(Value::as_str).unwrap_or_default().to_string() },
        _ => Kind::Domain { event_type: se.get("event_type").and_then(Value::as_str).unwrap_or_default().to_string() },
    }
}

fn is_late(e: &Value) -> bool {
    e["coverage_marker"] == "late"
}

/// Holes in the domain sequence: `prev` is the checkpoint's last contiguous sequence (None before the first batch).
fn holes(prev: Option<i64>, from: Option<i64>, to: Option<i64>, seqs: &[i64]) -> Vec<(i64, i64)> {
    let mut out = Vec::new();
    let mut expected = match prev {
        Some(p) => p + 1,
        None => from.or(seqs.first().copied()).unwrap_or(1),
    };
    for &s in seqs {
        if s > expected {
            out.push((expected, s - 1));
        }
        expected = expected.max(s + 1);
    }
    if let Some(t) = to
        && t >= expected
    {
        out.push((expected, t));
    }
    out
}

fn pairs(v: &Value) -> Vec<(i64, i64)> {
    v.as_array().into_iter().flatten().filter_map(|p| Some((p[0].as_i64()?, p[1].as_i64()?))).collect()
}
fn pairs_json(p: &[(i64, i64)]) -> Value {
    Value::Array(p.iter().map(|(a, b)| json!([a, b])).collect())
}

/// Remove `seq` from the open gaps (a late row arrived).
fn close_gap(gaps: &mut Vec<(i64, i64)>, seq: i64) {
    let mut out = Vec::new();
    for &(a, b) in gaps.iter() {
        if seq < a || seq > b {
            out.push((a, b));
            continue;
        }
        if a < seq {
            out.push((a, seq - 1));
        }
        if seq < b {
            out.push((seq + 1, b));
        }
    }
    *gaps = out;
}

impl App {
    pub(crate) fn ingest_claims(&self, r: &Req, scope: &str, purpose: Option<&'static str>) -> Result<Value, Resp> {
        let claims = self.authn(&self.control, r, &Expect { aud: "control-api", scope: Some(scope), purpose }).map_err(|d| error(d.status, d.reason))?;
        if let Some((sub, t)) = &self.cfg.upload_pin
            && (claims["sub"].as_str() != Some(sub) || claims["tenant_id"].as_str() != Some(t))
        {
            return Err(error(403, "binding_or_tenant_denied"));
        }
        Ok(claims)
    }

    fn resolve_ref(&self, tenant: &str, rf: &Value) -> Option<Value> {
        self.store.get_artifact(tenant, rf["id"].as_str()?).filter(|env| env["artifact"] == *rf)
    }

    /// Raw NDJSON behind a ref: a single artifact, or an immutable manifest of chunk refs.
    fn material(&self, tenant: &str, rf: &Value) -> Option<Vec<u8>> {
        let env = self.resolve_ref(tenant, rf)?;
        if rf["media_type"] == "application/x-ndjson" {
            return Some(env["content"].as_str()?.as_bytes().to_vec());
        }
        let mut out = Vec::new();
        for ch in env["content"]["chunks"].as_array().into_iter().flatten() {
            out.extend(self.material(tenant, &ch["ref"])?);
        }
        Some(out)
    }

    fn receipts_ok(&self, tenant: &str, receipts: &[Value]) -> Result<(), &'static str> {
        for rc in receipts {
            let mat = self.material(tenant, &rc["source_artifact_ref"]);
            let (Some(mat), Some(_)) = (mat, self.resolve_ref(tenant, &rc["source_schema_ref"])) else { return Err("unresolvable_artifact_ref") };
            let text = String::from_utf8_lossy(&mat).into_owned();
            let lines: Vec<&str> = text.lines().collect();
            let through = rc["verified_through_seq"].as_i64().unwrap_or(-1);
            let head = usize::try_from(through).ok().and_then(|i| lines.get(i)).and_then(|l| serde_json::from_str::<Value>(l).ok()).and_then(|v| v["hash"].as_str().map(str::to_string));
            let sha = rc["verifier_agent_core_sha"].as_str().unwrap_or_default();
            let sha_ok = sha.len() == 40 && sha.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
            if head.is_none() || sha256_hex(&mat) != rc["source_artifact_digest"].as_str().unwrap_or_default() || head.as_deref() != rc["chain_head_hash"].as_str() || lines.len() as i64 != through + 1 || !sha_ok {
                return Err("receipt_material_mismatch");
            }
        }
        Ok(())
    }

    // ---- POST /internal/v1/platform/observations -------------------------------------------------------------------
    pub(crate) fn observations(&self, r: &Req) -> Resp {
        let claims = match self.ingest_claims(r, "observations", Some("platform_observations")) {
            Ok(c) => c,
            Err(e) => return e,
        };
        let tenant = claims["tenant_id"].as_str().unwrap_or_default().to_string();
        if let Some(code) = self.fault("ingest").and_then(|m| m.parse::<u16>().ok()) {
            return error(code, "injected");
        }
        if r.body.len() > MAX_BATCH_BYTES {
            return error(413, "batch_too_large");
        }
        let Some(mut b) = serde_json::from_slice::<Value>(&r.body).ok().and_then(validate_batch) else { return error(422, "schema_invalid") };
        let digest = b.remove("batch_digest").and_then(|d| d.as_str().map(str::to_string)).unwrap_or_default();
        let canonical = jcs(&Value::Object(b.clone())).ok().map(|c| sha256_hex(c.as_bytes()));
        if !is_hex64(&digest) || canonical.as_deref() != Some(digest.as_str()) {
            return error(422, "batch_digest_mismatch");
        }
        if r.headers.get("idempotency-key").map(String::as_str) != Some(digest.as_str()) {
            return error(422, "idempotency_key_mismatch");
        }
        let events = b["events"].as_array().cloned().unwrap_or_default();
        if events.len() > MAX_BATCH_EVENTS || !cursor_ok(b["cursor"].as_str().unwrap_or_default()) {
            return error(422, "limits");
        }
        for ev in &events {
            if (ev["kind"] == "core_event") != !ev["level"].is_null() {
                return error(422, "level_required_for_core_event");
            }
            if ev["source_event"].is_null() && ev["source_event_ref"].is_null() {
                return error(422, "source_event_ref_required");
            }
            if !is_hex64(ev["source_event_digest"].as_str().unwrap_or_default()) {
                return error(422, "digest_format");
            }
            if ev["level"] == "engine_event" && (ev["source_run_ref"].is_null() || ev["source_sequence"].is_null()) {
                return error(422, "audit_run_and_sequence_required");
            }
        }
        if b["tenant_id"].as_str() != Some(tenant.as_str()) {
            return error(403, "cross_tenant");
        }
        for ev in &events {
            let refs = [Some(&ev["source_schema_ref"]), Some(&ev["source_event_ref"]).filter(|v| !v.is_null())];
            if refs.into_iter().flatten().any(|rf| self.resolve_ref(&tenant, rf).is_none()) {
                return error(422, "unresolvable_artifact_ref");
            }
        }
        let receipts: Vec<Value> = b["verification_receipts"].as_array().cloned().unwrap_or_default();
        if let Err(c) = self.receipts_ok(&tenant, &receipts) {
            return error(422, c);
        }
        let fast = b["scan_mode"] == "fast_poll";
        if fast && b["expected_cursor_revision"].is_null() {
            return error(422, "expected_cursor_revision_required");
        }
        if !fast && !b["expected_cursor_revision"].is_null() {
            return error(422, "rescan_requires_null_revision");
        }

        let source = b["source_id"].as_str().unwrap_or_default().to_string();
        let partition = b["partition"].as_str().unwrap_or_default().to_string();
        let ckey = jkey(&[&source, &partition]);
        let _w = self.write_lock.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(prior) = self.store.get_doc("ingest_receipt", &tenant, &digest) {
            return json_resp(200, prior["replay"].clone()); // committed earlier: never re-apply
        }
        let mut cur = self.store.get_doc("ingest_cursor", &tenant, &ckey).unwrap_or_else(|| json!({"cursor": null, "revision": 0, "last_batch_digest": null, "last_seq": null, "open_gaps": []}));
        if fast && b["expected_cursor_revision"] != cur["revision"] {
            return error(409, "stale_cursor_revision"); // before any write
        }
        let ledger_id = |ev: &Value| jkey(&[&source, ev["kind"].as_str().unwrap_or_default(), ev["level"].as_str().unwrap_or("-"), ev["native_event_id"].as_str().unwrap_or_default()]);
        for ev in &events {
            // all-or-nothing: a digest conflict anywhere refuses the whole batch
            if let Some(prev) = self.store.get_doc("ingest_ledger", &tenant, &ledger_id(ev))
                && prev != ev["source_event_digest"]
            {
                return error(409, "source_digest_conflict");
            }
        }

        // ---- classify, then look for undeclared holes in the domain sequence --------------------------------------
        let mut quarantined: Vec<(usize, String, Class)> = Vec::new(); // (event index, type, class)
        let mut seqs: Vec<i64> = Vec::new();
        let mut declared: Vec<(i64, i64)> = Vec::new();
        for (i, ev) in events.iter().enumerate() {
            match kind_of(ev) {
                Kind::Domain { event_type } => {
                    if !is_late(ev)
                        && let Some(s) = ev["source_sequence"].as_i64()
                    {
                        seqs.push(s);
                    }
                    let class = classify(&event_type);
                    if class != Class::Admitted {
                        quarantined.push((i, event_type, class));
                    }
                }
                Kind::Finding { code } => {
                    if !FINDING_CODES.contains(&code.as_str()) {
                        quarantined.push((i, format!("exporter_finding:{code}"), Class::Unknown));
                    } else if code == "gap_suspected" {
                        let d = &ev["source_event"]["details"];
                        if let (Some(a), Some(z)) = (d["from_sequence"].as_i64(), d["to_sequence"].as_i64()) {
                            declared.push((a, z));
                        }
                    }
                }
                Kind::Other => {}
            }
        }
        seqs.sort_unstable();
        seqs.dedup();
        let prev = cur["last_seq"].as_i64();
        seqs.retain(|s| prev.is_none_or(|p| *s > p));
        let found = if fast { holes(prev, b["from_seq"].as_i64(), b["to_seq"].as_i64(), &seqs) } else { Vec::new() };
        let uncovered: Vec<(i64, i64)> = found.iter().copied().filter(|(a, z)| !declared.iter().any(|(da, dz)| da <= a && dz >= z)).collect();

        let mut receipt = json!({"batch_digest": digest, "accepted_event_count": 0, "duplicate_event_count": 0, "quarantined_event_count": 0,
                                 "late_event_count": 0, "unknown_event_types": {}, "quarantined_event_types": {}, "disposition": "accepted",
                                 "checkpoint_advanced": false, "current_cursor": cur["cursor"], "cursor_revision": cur["revision"]});
        if !uncovered.is_empty() {
            receipt["disposition"] = json!("quarantined");
            receipt["quarantine_reason"] = json!("sequence_gap");
            receipt["gaps"] = pairs_json(&uncovered);
            receipt["quarantined_event_count"] = json!(events.len());
            let record = json!({"classification": "batch", "event_type": null, "reason": "sequence_gap", "gaps": pairs_json(&uncovered),
                                "source_id": source, "batch_digest": digest, "event_count": events.len()});
            self.store.put_doc("ingest_quarantine", &tenant, &jkey(&[&source, "batch", &digest]), record);
            return self.commit_receipt(&tenant, &digest, receipt, 202);
        }

        // ---- accept: ledger, quarantine metadata, cursor ---------------------------------------------------------
        let (mut accepted, mut dup, mut late_n) = (0i64, 0i64, 0i64);
        let mut gaps = pairs(&cur["open_gaps"]);
        let skip: std::collections::HashSet<usize> = quarantined.iter().map(|(i, _, _)| *i).collect();
        for (i, ev) in events.iter().enumerate() {
            if skip.contains(&i) {
                continue;
            }
            let id = ledger_id(ev);
            if self.store.get_doc("ingest_ledger", &tenant, &id).is_some() {
                dup += 1;
                continue;
            }
            self.store.put_doc("ingest_ledger", &tenant, &id, ev["source_event_digest"].clone());
            accepted += 1;
            if is_late(ev) {
                late_n += 1;
                if let Some(s) = ev["source_sequence"].as_i64() {
                    close_gap(&mut gaps, s);
                }
            }
        }
        let mut by_type: BTreeMap<String, i64> = BTreeMap::new();
        let mut unknown: BTreeMap<String, i64> = BTreeMap::new();
        for (i, event_type, class) in &quarantined {
            let ev = &events[*i];
            *by_type.entry(event_type.clone()).or_default() += 1;
            if *class == Class::Unknown {
                *unknown.entry(event_type.clone()).or_default() += 1;
            }
            // metadata only: the (possibly sensitive) payload is never stored
            let record = json!({"classification": class.name(), "event_type": event_type, "reason": "event_type_not_admitted", "source_id": source,
                                "native_event_id": ev["native_event_id"], "source_event_digest": ev["source_event_digest"],
                                "source_sequence": ev["source_sequence"], "batch_digest": digest});
            self.store.put_doc("ingest_quarantine", &tenant, &jkey(&[&source, ev["native_event_id"].as_str().unwrap_or_default(), &digest]), record);
        }
        let mut advanced = false;
        if fast {
            gaps.extend(found.iter().copied()); // every hole is declared here (uncovered ones returned above)
            gaps.sort_unstable();
            gaps.dedup();
            let top = seqs.iter().copied().chain(found.iter().map(|h| h.1)).max();
            cur["last_seq"] = json!(top.max(prev));
            cur = json!({"cursor": b["cursor"], "revision": cur["revision"].as_i64().unwrap_or(0) + 1, "last_batch_digest": digest, "last_seq": cur["last_seq"], "open_gaps": []});
            advanced = true;
        }
        cur["open_gaps"] = pairs_json(&gaps);
        self.store.put_doc("ingest_cursor", &tenant, &ckey, cur.clone());
        receipt["accepted_event_count"] = json!(accepted);
        receipt["duplicate_event_count"] = json!(dup);
        receipt["quarantined_event_count"] = json!(quarantined.len());
        receipt["late_event_count"] = json!(late_n);
        receipt["unknown_event_types"] = json!(unknown);
        receipt["quarantined_event_types"] = json!(by_type);
        receipt["checkpoint_advanced"] = json!(advanced);
        receipt["current_cursor"] = cur["cursor"].clone();
        receipt["cursor_revision"] = cur["revision"].clone();
        self.commit_receipt(&tenant, &digest, receipt, 202)
    }

    /// Stores the receipt and its replay variant (nothing re-applied, `accepted` folded into `duplicate`), answers `status`.
    fn commit_receipt(&self, tenant: &str, digest: &str, receipt: Value, status: u16) -> Resp {
        let mut replay = receipt.clone();
        let n = receipt["accepted_event_count"].as_i64().unwrap_or(0) + receipt["duplicate_event_count"].as_i64().unwrap_or(0);
        replay["checkpoint_advanced"] = json!(false);
        replay["accepted_event_count"] = json!(0);
        replay["duplicate_event_count"] = json!(n);
        self.store.put_doc("ingest_receipt", tenant, digest, json!({"first": receipt, "replay": replay}));
        json_resp(status, receipt)
    }

    // ---- GET /internal/v1/platform/exporters/{source}/partitions/{partition}/cursor ---------------------------------
    pub(crate) fn cursor_get(&self, r: &Req) -> Resp {
        let claims = match self.ingest_claims(r, "observations", None) {
            Ok(c) => c,
            Err(e) => return e,
        };
        let rest = r.path.strip_prefix("/internal/v1/platform/exporters/").unwrap_or_default();
        let parts: Vec<&str> = rest.split('/').collect();
        let [source, "partitions", partition, "cursor"] = parts[..] else { return crate::app::code(404, "not_found") };
        if partition.is_empty() || partition.len() > 64 || !partition.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b':' | b'-')) {
            return error(404, "not_found");
        }
        let tenant = claims["tenant_id"].as_str().unwrap_or_default();
        let cur = self.store.get_doc("ingest_cursor", tenant, &jkey(&[source, partition])).unwrap_or_else(|| json!({"cursor": null, "revision": 0, "last_batch_digest": null, "open_gaps": []}));
        json_resp(200, json!({"contract_version": "pulso-observations-2", "source_id": source, "partition": partition, "cursor": cur["cursor"],
                              "cursor_revision": cur["revision"], "last_batch_digest": cur["last_batch_digest"], "coverage": "partial", "cut_ref": null,
                              "updated_at": null, "open_gaps": cur["open_gaps"]}))
    }

    // ---- GET /internal/v1/platform/quarantine ------------------------------------------------------------------------
    pub(crate) fn quarantine_list(&self, r: &Req) -> Resp {
        let claims = match self.ingest_claims(r, "observations", None) {
            Ok(c) => c,
            Err(e) => return e,
        };
        let mut items: Vec<Value> = self.store.list_docs("ingest_quarantine", claims["tenant_id"].as_str().unwrap_or_default()).into_iter().map(|(_, v)| v).collect();
        items.sort_by_key(|v| (v["event_type"].as_str().unwrap_or_default().to_string(), v["native_event_id"].as_str().unwrap_or_default().to_string()));
        json_resp(200, json!({"items": items}))
    }
}
