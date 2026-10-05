//! Finding -> real agent-core artifact mapping (BANK_DATA_AUDIT section 5, ARTIFACT_ANATOMY sections 3 and 6.1), MAP1: data-driven.
//!
//! The mapping table is DATA (`fixtures/mapping_table.json`, label `hypothesis_table`, with provenance and a pointer to the anatomy
//! evidence per candidate) and is validated when it is loaded. A finding (metric, cell dimensions) maps to a ranked LIST of candidate
//! targets, each with a justification (finding cell -> topic -> existing agent or artifact whose scope covers it).
//!
//! Honest reading: a row is a HYPOTHESIS of where to intervene, never a cause. The cells are associations and a contact is not
//! linkable to a complaint; the table says so in its `notice` and every row carries the caveat
//! `mapping_is_a_hypothesis_of_where_to_intervene`.
//!
//! A finding that maps to nothing stays `unlinked` (descriptive): without a mapping there is no proposal. A finding a person owns
//! (`human_owned` entries: the dispute amount policy, the es-only calibration) is never proposed by the Builder. The mapping row is the
//! ONLY place that decides which artifacts a role may name; the model chooses among the targets of the row, it never invents one.
//! Deny-listed kinds (policy values, interrupts, injection rulesets, language detection, tools, `release_settings`, `constructor-chat`
//! and `pulso-*` agents, model profiles) are not representable as targets at all (the loader refuses them).
use crate::finding::Finding;
use serde_json::{Value, json};

pub const MECHANISMS: [&str; 10] = ["draft_next_step", "uncovered_topic", "repeated_lookup", "status_message_gap", "closing_followup", "wording", "none", "missing_tool", "stale_tool_answer", "policy_threshold"];

/// Kinds a target can have. Anything else (policy, tool, flow, model_profile ...) is refused by construction.
pub const TARGET_KINDS: [&str; 4] = ["patch", "new_agent", "link_tool", "tighten_policy"];

/// At most this many candidates of one finding are tried (in rank order, stopping at the first proven one).
pub const MAX_CANDIDATES: usize = 2;

const BUNDLED: &str = include_str!("../fixtures/mapping_table.json");

/// What the engine can announce today. A candidate that cannot be announced now is ranked after one that can.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Caps {
    /// A human-owned admin credential exists for the release settings an evaluation draft of a NEW agent needs (agent-core `put_draft`
    /// refuses them to the engine `builder` role with 403). Off by default: `PULSO_NEW_AGENT_ADMIN=1`.
    pub new_agent_admin: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Target {
    /// `template:t/estado_pqr`, `prompt:p/copiloto` or `new_agent:<donor>`.
    pub target_ref: String,
    /// `patch` (anchored patch of an existing template or prompt) or `new_agent` (closure copy of the donor).
    pub kind: &'static str,
    /// The agent whose behaviour the target changes (existing agent) or the donor of the clone.
    pub agent: &'static str,
    /// Placeholders a template patch may introduce (`{{ facts.pqr.value.status }}`); prompts never take any.
    pub placeholders: Vec<&'static str>,
    /// For `new_agent`: the closed list of slugs the Builder may choose from.
    pub slugs: Vec<&'static str>,
    pub mechanisms: Vec<&'static str>,
    /// 1 = first in the table (after the `when_dims` filter).
    pub rank: usize,
    /// Finding cell -> topic -> existing agent or artifact whose scope covers it: one sentence.
    pub justification: &'static str,
    /// Pointer to the anatomy / audit evidence.
    pub evidence: &'static str,
    /// `suite:<mechanism>` (a regression suite generator exists, `scripts/regression/build_suite.py`) or `none`.
    pub proof_support: &'static str,
    pub announceable_now: bool,
    /// ART2 structured params (`link_tool`: `{tool}`; `tighten_policy`: `{tighten_to, source}`), never free text.
    pub params: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub id: &'static str,
    /// The topic the cell was mapped to (`complaint_followup`, `uncovered_reason`, ...).
    pub topic: &'static str,
    /// Why the topic is covered (or not) by existing agents.
    pub topic_why: &'static str,
    pub covered_by: Vec<&'static str>,
    /// Ranked candidate targets (table order).
    pub targets: Vec<Target>,
    /// `mechanism_proxy | unlinked` (never `same_outcome_linked` from CSV aggregates).
    pub link_grade: &'static str,
    /// Counter-metric the proposal must name (rubric R8).
    pub guardrail: &'static str,
    pub caveats: Vec<&'static str>,
    /// How many candidates the table has for this cell before the cap (`MAX_CANDIDATES`).
    pub candidates_total: usize,
}

/// A finding no Builder may propose for: a person owns it. It becomes a `human_owned` outcome with a short note.
#[derive(Debug, Clone, PartialEq)]
pub struct HumanOwned {
    pub id: &'static str,
    pub owner: &'static str,
    pub evidence: &'static str,
    pub note_es: &'static str,
    pub note_pt: &'static str,
    /// ART2: a policy finding: `{policy, registry_threshold, document_threshold, tighten_to?}`. Present: outcome `policy_hypothesis`
    /// (and `needs_owner_ack` when a tighten-only draft compiles from `tighten_to`); the engine never picks the value.
    pub policy: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Mapped {
    Row(Row),
    HumanOwned(HumanOwned),
    Unmapped,
}

fn leak(s: &str) -> &'static str {
    // The table is loaded once per process (OnceLock) and lives as long as the process: leaking its strings is bounded.
    Box::leak(s.to_string().into_boxed_str())
}

fn strs(x: &Value) -> Vec<String> {
    x.as_array().into_iter().flatten().filter_map(|s| s.as_str().map(str::to_string)).collect()
}

type DimCond = Vec<(String, Vec<String>)>;

fn dim_cond(v: &Value) -> DimCond {
    v.as_object().map(|o| o.iter().map(|(k, x)| (k.clone(), strs(x))).collect()).unwrap_or_default()
}

fn dims_hold(cond: &DimCond, f: &Finding) -> bool {
    cond.iter().all(|(k, vals)| f.dims.get(k).is_some_and(|v| vals.contains(v)))
}

#[derive(Debug, Clone)]
struct Matcher {
    metrics: Vec<String>,
    dims: DimCond,
    dims_present: Vec<String>,
}

impl Matcher {
    fn parse(v: &Value, what: &str) -> Result<Matcher, String> {
        let m = Matcher { metrics: strs(&v["metric"]), dims: dim_cond(&v["dims"]), dims_present: strs(&v["dims_present"]) };
        if m.metrics.is_empty() && m.dims.is_empty() && m.dims_present.is_empty() {
            return Err(format!("{what}: an empty match would match every finding"));
        }
        if m.dims.iter().any(|(_, vals)| vals.is_empty()) {
            return Err(format!("{what}: a dims entry without values"));
        }
        Ok(m)
    }
    fn matches(&self, f: &Finding) -> bool {
        (self.metrics.is_empty() || self.metrics.iter().any(|m| m == &f.metric)) && dims_hold(&self.dims, f) && self.dims_present.iter().all(|k| f.dims.contains_key(k))
    }
}

#[derive(Debug, Clone)]
struct RowDef {
    row: Row,
    matcher: Matcher,
    /// Per candidate (same order as `row.targets`): conditions on the finding's dims (e.g. channel Phone).
    when: Vec<DimCond>,
}

#[derive(Debug, Clone)]
struct HumanDef {
    h: HumanOwned,
    matcher: Matcher,
}

#[derive(Debug, Clone)]
pub struct Table {
    pub label: &'static str,
    pub notice_en: &'static str,
    pub notice_es: &'static str,
    pub notice_pt: &'static str,
    pub provenance: Value,
    rows: Vec<RowDef>,
    human: Vec<HumanDef>,
}

fn need<'a>(v: &'a Value, key: &str, what: &str) -> Result<&'a str, String> {
    v[key].as_str().filter(|s| !s.trim().is_empty()).ok_or_else(|| format!("{what}: missing {key}"))
}

impl Table {
    /// Parses and VALIDATES a mapping table. Every violation is a typed message; the bundled table is parsed at first use and a test
    /// pins that it is valid.
    pub fn parse(v: &Value) -> Result<Table, String> {
        if v["contract"].as_str() != Some("reasoning.mapping_table/1") {
            return Err("unknown mapping table contract".into());
        }
        let topics = v["topics"].as_object().ok_or("table without topics")?;
        let mut rows = vec![];
        for (ri, r) in v["rows"].as_array().ok_or("table without rows")?.iter().enumerate() {
            let id = need(r, "id", &format!("row {ri}"))?;
            let what = format!("row {id}");
            let topic_key = need(r, "topic", &what)?;
            let topic = topics.get(topic_key).ok_or_else(|| format!("{what}: unknown topic {topic_key}"))?;
            let link_grade = need(r, "link_grade", &what)?;
            if !["mechanism_proxy", "unlinked"].contains(&link_grade) {
                return Err(format!("{what}: link_grade {link_grade} is not allowed from aggregates"));
            }
            let caveats: Vec<&'static str> = strs(&r["caveats"]).iter().map(|c| leak(c)).collect();
            if !caveats.contains(&"mapping_is_a_hypothesis_of_where_to_intervene") {
                return Err(format!("{what}: the caveat mapping_is_a_hypothesis_of_where_to_intervene is mandatory"));
            }
            let cands = r["candidates"].as_array().filter(|c| !c.is_empty()).ok_or_else(|| format!("{what}: no candidates"))?;
            let (mut targets, mut when) = (vec![], vec![]);
            for (ci, c) in cands.iter().enumerate() {
                let cw = format!("{what} candidate {}", ci + 1);
                if c["rank"].as_u64() != Some(ci as u64 + 1) {
                    return Err(format!("{cw}: ranks must be 1..n in order"));
                }
                let kind = need(c, "kind", &cw)?;
                if !TARGET_KINDS.contains(&kind) {
                    return Err(format!("{cw}: kind {kind} is not representable (deny-listed)"));
                }
                let target_ref = need(c, "target_ref", &cw)?;
                let (prefix, tid) = target_ref.split_once(':').ok_or_else(|| format!("{cw}: target_ref without kind prefix"))?;
                let denied_ids = ["constructor-chat", "release_settings", "interrupt", "injection", "language_detection", "model_profile", "perfil-generacion", "policy"];
                let prefix_ok = match kind {
                    "patch" => prefix == "template" || prefix == "prompt",
                    "link_tool" => prefix == "tool_link" && tid.split_once('/').is_some_and(|(a, t)| !a.is_empty() && !t.is_empty()),
                    "tighten_policy" => prefix == "policy" && c["params"]["tighten_to"].is_number(),
                    _ => prefix == "new_agent",
                };
                if !prefix_ok {
                    return Err(format!("{cw}: target_ref {target_ref} does not match kind {kind}"));
                }
                if tid.starts_with("pulso-") || denied_ids.iter().any(|d| tid.contains(d)) {
                    return Err(format!("{cw}: {tid} is deny-listed"));
                }
                let mechanisms: Vec<&'static str> = strs(&c["mechanisms"]).iter().map(|m| leak(m)).collect();
                if mechanisms.is_empty() || mechanisms.iter().any(|m| !MECHANISMS.contains(m)) {
                    return Err(format!("{cw}: mechanisms must be a non-empty subset of the closed list"));
                }
                let slugs: Vec<&'static str> = strs(&c["slugs"]).iter().map(|m| leak(m)).collect();
                if kind == "new_agent" && (slugs.is_empty() || slugs.iter().any(|s| s.starts_with("pulso-") || *s == "constructor-chat")) {
                    return Err(format!("{cw}: a new agent needs a closed list of allowed slugs"));
                }
                let proof_support = need(c, "proof_support", &cw)?;
                if proof_support != "none" && !proof_support.starts_with("suite:") {
                    return Err(format!("{cw}: proof_support must be none or suite:<mechanism>"));
                }
                targets.push(Target {
                    target_ref: target_ref.to_string(),
                    kind: match kind {
                        "patch" => "patch",
                        "link_tool" => "link_tool",
                        "tighten_policy" => "tighten_policy",
                        _ => "new_agent",
                    },
                    agent: leak(need(c, "agent", &cw)?),
                    placeholders: strs(&c["placeholders"]).iter().map(|m| leak(m)).collect(),
                    slugs,
                    mechanisms,
                    rank: ci + 1,
                    justification: leak(need(c, "justification", &cw)?),
                    evidence: leak(need(c, "evidence", &cw)?),
                    proof_support: leak(proof_support),
                    announceable_now: c["announceable_now"].as_bool().ok_or_else(|| format!("{cw}: announceable_now missing"))?,
                    params: c["params"].clone(),
                });
                when.push(dim_cond(&c["when_dims"]));
            }
            let total = targets.len();
            rows.push(RowDef {
                row: Row {
                    id: leak(id),
                    topic: leak(topic_key),
                    topic_why: leak(need(topic, "why", &format!("topic {topic_key}"))?),
                    covered_by: strs(&topic["covered_by"]).iter().map(|c| leak(c)).collect(),
                    targets,
                    link_grade: leak(link_grade),
                    guardrail: leak(need(r, "guardrail", &what)?),
                    caveats,
                    candidates_total: total,
                },
                matcher: Matcher::parse(&r["match"], &what)?,
                when,
            });
        }
        let mut human = vec![];
        for h in v["human_owned"].as_array().into_iter().flatten() {
            let id = need(h, "id", "human_owned")?;
            human.push(HumanDef {
                h: HumanOwned {
                    id: leak(id),
                    owner: leak(need(h, "owner", id)?),
                    evidence: leak(need(h, "evidence", id)?),
                    note_es: leak(need(&h["note"], "es", id)?),
                    note_pt: leak(need(&h["note"], "pt", id)?),
                    policy: h["policy"].clone(),
                },
                matcher: Matcher::parse(&h["match"], id)?,
            });
        }
        Ok(Table {
            label: leak(need(v, "label", "table")?),
            notice_en: leak(need(&v["notice"], "en", "notice")?),
            notice_es: leak(need(&v["notice"], "es", "notice")?),
            notice_pt: leak(need(&v["notice"], "pt", "notice")?),
            provenance: v["provenance"].clone(),
            rows,
            human,
        })
    }

    pub fn bundled() -> &'static Table {
        static T: std::sync::OnceLock<Table> = std::sync::OnceLock::new();
        T.get_or_init(|| Table::parse(&serde_json::from_str(BUNDLED).expect("the bundled mapping table is JSON")).expect("the bundled mapping table is valid"))
    }

    /// Human-owned entries are checked FIRST: a person owns the finding even when an artifact row would also match.
    pub fn classify(&self, f: &Finding) -> Mapped {
        if let Some(h) = self.human.iter().find(|h| h.matcher.matches(f)) {
            return Mapped::HumanOwned(h.h.clone());
        }
        let Some(rd) = self.rows.iter().find(|r| r.matcher.matches(f)) else { return Mapped::Unmapped };
        let mut row = rd.row.clone();
        row.targets = rd.row.targets.iter().zip(&rd.when).filter(|(_, w)| dims_hold(w, f)).map(|(t, _)| t.clone()).collect();
        for (n, t) in row.targets.iter_mut().enumerate() {
            t.rank = n + 1;
        }
        row.candidates_total = row.targets.len();
        Mapped::Row(row)
    }

    pub fn row_ids(&self) -> Vec<&'static str> {
        self.rows.iter().map(|r| r.row.id).collect()
    }

    /// Every candidate of every row (for tests and tools that check the table against the catalogue).
    pub fn all_targets(&self) -> Vec<&Target> {
        self.rows.iter().flat_map(|r| r.row.targets.iter()).collect()
    }
}

pub fn map_finding(f: &Finding) -> Option<Row> {
    match Table::bundled().classify(f) {
        Mapped::Row(r) => Some(r),
        _ => None,
    }
}

pub fn human_owned(f: &Finding) -> Option<HumanOwned> {
    match Table::bundled().classify(f) {
        Mapped::HumanOwned(h) => Some(h),
        _ => None,
    }
}

impl Row {
    pub fn target(&self, target_ref: &str) -> Option<&Target> {
        self.targets.iter().find(|t| t.target_ref == target_ref)
    }

    /// Can this candidate be proven and announced with what the engine has now?
    pub fn announceable(t: &Target, caps: Caps) -> bool {
        t.proof_support != "none" && (t.announceable_now || (t.kind == "new_agent" && caps.new_agent_admin))
    }

    /// The candidates in the order the engine tries them: table rank, with the ones that can be announced NOW first (stable), capped at
    /// `MAX_CANDIDATES`.
    pub fn ordered(&self, caps: Caps) -> Vec<&Target> {
        let mut v: Vec<&Target> = self.targets.iter().collect();
        v.sort_by_key(|t| (!Row::announceable(t, caps), t.rank));
        v.truncate(MAX_CANDIDATES);
        v
    }

    /// This row restricted to ONE candidate: what a single attempt (Scout, Builder, compiler) sees.
    pub fn only(&self, target_ref: &str) -> Option<Row> {
        let t = self.target(target_ref)?.clone();
        let mut r = self.clone();
        r.targets = vec![t];
        Some(r)
    }

    /// The honest list of candidates for a record: rank, try order, target, justification, evidence, proof support and whether it was tried.
    pub fn candidate_list(&self, caps: Caps, tried: &[String]) -> Vec<Value> {
        let order = self.ordered(caps);
        self.targets
            .iter()
            .map(|t| {
                let pos = order.iter().position(|o| o.target_ref == t.target_ref);
                let was_tried = tried.contains(&t.target_ref);
                let why_not = if was_tried {
                    ""
                } else if pos.is_none() {
                    "over_the_candidate_cap"
                } else {
                    "an_earlier_candidate_was_proven_or_the_finding_stopped"
                };
                json!({"rank": t.rank, "try_order": pos.map(|p| p + 1), "target_ref": t.target_ref, "kind": t.kind, "agent": t.agent, "justification": t.justification, "evidence": t.evidence,
                       "proof_support": t.proof_support, "announceable_now": Row::announceable(t, caps), "tried": was_tried, "not_tried_because": why_not})
            })
            .collect()
    }
}

/// Why a finding has NO row: a closed, explicit reason (never a failure: an unlinked finding is descriptive and is not counted against
/// the Builder). `(code, why)`.
pub fn unlinked_reason(f: &Finding) -> (&'static str, String) {
    match f.metric.as_str() {
        m if m.starts_with("M6") => (
            "dependency_metric",
            format!("metric {m} (customer satisfaction after a contact) is a dependency metric: it follows how the contact was resolved (M1), no agent-core artifact moves it directly; descriptive, the proposal starts from the M1 findings"),
        ),
        "M10" => ("dependent_on_m1", "metric M10 depends on the unresolved-contact rate (M1) and has no artifact of its own; descriptive, the proposal starts from the M1 findings".to_string()),
        "M8" => ("level_risk_human_owned", "metric M8 is a level risk (sends to customers without consent): a consent and marketing policy question that only a human owner can change, not an agent artifact".to_string()),
        m => ("no_mapping", format!("metric {m} with these dimensions maps to no agent-core artifact: descriptive finding, no proposal")),
    }
}
