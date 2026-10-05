//! Finding -> real agent-core artifact mapping (BANK_DATA_AUDIT section 5, ARTIFACT_ANATOMY sections 3 and 6.1).
//!
//! A finding that maps to nothing stays `unlinked` (descriptive): the spec rule is that without a mapping there is no proposal.
//! The mapping row is the ONLY place that decides which artifacts a role may name; the model chooses among the targets of the
//! row, it never invents one. Deny-listed kinds (policy values, interrupts, injection rulesets, language detection, tools,
//! `release_settings`, `constructor-chat` and `pulso-*` agents, model profiles) are not representable as targets at all.
use crate::finding::Finding;

pub const MECHANISMS: [&str; 5] = ["uncovered_topic", "repeated_lookup", "status_message_gap", "wording", "none"];

/// Kinds a target can have. Anything else (policy, tool, flow, model_profile ...) is refused by construction.
pub const TARGET_KINDS: [&str; 2] = ["patch", "new_agent"];

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
}

#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub id: &'static str,
    pub targets: Vec<Target>,
    /// `mechanism_proxy | unlinked` (never `same_outcome_linked` from CSV aggregates).
    pub link_grade: &'static str,
    /// Counter-metric the proposal must name (rubric R8).
    pub guardrail: &'static str,
    pub caveats: Vec<&'static str>,
}

fn patch(target_ref: &str, agent: &'static str, placeholders: Vec<&'static str>, mechanisms: Vec<&'static str>) -> Target {
    Target { target_ref: target_ref.into(), kind: "patch", agent, placeholders, slugs: vec![], mechanisms }
}

pub fn map_finding(f: &Finding) -> Option<Row> {
    let reason = f.dims.get("reason_category").map(String::as_str);
    let category = f.dims.get("category").map(String::as_str);
    let case_type = f.dims.get("case_type").map(String::as_str);
    match (f.metric.as_str(), reason, category, case_type) {
        ("M1", Some(r @ ("Tecnico" | "Comercial" | "Retencion")), _, _) => {
            let slugs = match r {
                "Tecnico" => vec!["soporte-tecnico", "soporte-app"],
                "Comercial" => vec!["soporte-comercial"],
                _ => vec!["soporte-retencion"],
            };
            Some(Row {
                id: "uncovered_reason",
                targets: vec![Target { target_ref: "new_agent:consultas".into(), kind: "new_agent", agent: "consultas", placeholders: vec![], slugs, mechanisms: vec!["uncovered_topic"] }],
                link_grade: "mechanism_proxy",
                guardrail: "traffic_of_disputas_and_consultas_via_recepcion_unchanged",
                caveats: vec!["no_specialist_in_directory", "contact_and_complaint_not_linkable", "where_not_why", "mostly_phone_contacts"],
            })
        }
        ("M1", Some("Queja"), _, _) => Some(Row {
            id: "complaint_unresolved",
            targets: vec![Target { target_ref: "new_agent:consultas".into(), kind: "new_agent", agent: "consultas", placeholders: vec![], slugs: vec!["intake-quejas"], mechanisms: vec!["uncovered_topic"] }],
            link_grade: "mechanism_proxy",
            guardrail: "traffic_of_disputas_and_consultas_via_recepcion_unchanged",
            caveats: vec!["only_part_of_complaints_are_disputes", "contact_and_complaint_not_linkable", "where_not_why"],
        }),
        ("M4", _, Some(_), _) | ("M5", _, Some(_), _) => Some(Row {
            id: "pqr_status_message",
            targets: vec![patch("template:t/estado_pqr", "consultas", vec!["facts.pqr.value.status"], vec!["status_message_gap", "wording"])],
            link_grade: "unlinked",
            guardrail: "tool_failure_and_low_confidence_escalations_not_worse",
            caveats: vec!["backlog_not_changed_by_a_message", "status_field_shape_unverified_on_real_tool"],
        }),
        // E0-derived advisor-side finding: the same recent-charges lookup repeated inside a dispute case.
        ("E1", _, _, Some("dispute")) => Some(Row {
            id: "copilot_repeated_lookup",
            targets: vec![patch("prompt:p/copiloto", "copiloto-asesor", vec![], vec!["repeated_lookup", "wording"])],
            link_grade: "mechanism_proxy",
            guardrail: "fallback_and_gave_up_rate_not_worse",
            caveats: vec!["e0_is_generated_data", "no_draft_acceptance_data", "advisor_agent_not_natively_evaluable"],
        }),
        _ => None,
    }
}

impl Row {
    pub fn target(&self, target_ref: &str) -> Option<&Target> {
        self.targets.iter().find(|t| t.target_ref == target_ref)
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
