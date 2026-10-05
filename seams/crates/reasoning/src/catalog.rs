//! The base artifacts the Builder may patch, and the anchor menu computed from their exact text.
//!
//! The catalogue is a BASELINE exported from the agent-core e2e fixtures (`scripts/reasoning/export_base_artifacts.py`), labelled
//! `fixture-baseline`: it is not a live registry. The engine must read the live entity before a real write; the compiled patch
//! carries the base digest so a changed base is detected (`patch::compile` refuses a base whose digest differs).
use serde_json::Value;
use std::collections::BTreeMap;

const BUNDLED: &str = include_str!("../fixtures/base_artifacts.json");

#[derive(Debug, Clone, PartialEq)]
pub struct Artifact {
    pub kind: String,
    pub id: String,
    pub version: String,
    pub locales: BTreeMap<String, String>,
    /// Every other field of the entity (`model_profile`, `reads`), carried unchanged into the new version.
    pub extra: Value,
    /// `(kind, id, version)` of the entities that reference it.
    pub referenced_by: Vec<(String, String, String)>,
}

impl Artifact {
    pub fn target_ref(&self) -> String {
        format!("{}:{}", self.kind, self.id)
    }
    pub fn digest(&self) -> String {
        let mut raw = format!("{}@{}", self.target_ref(), self.version);
        for (l, t) in &self.locales {
            raw.push_str(&format!("|{l}={t}"));
        }
        steps::compile::sha256_hex_calm(raw.as_bytes(), 16)
    }
}

#[derive(Debug, Clone)]
pub struct Catalog {
    pub label: String,
    pub source: Value,
    artifacts: BTreeMap<String, Artifact>,
    agents: BTreeMap<String, Value>,
    pub donor: String,
    pub donor_flow: Value,
    pub donor_release: Value,
    /// ART2: every flow graph, policy and ToolDef of the baseline (entity JSON, as the registry serves it) and the tool-service listing.
    pub flows: BTreeMap<String, Value>,
    pub policies: BTreeMap<String, Value>,
    pub tool_defs: BTreeMap<String, Value>,
    pub tool_service: Option<crate::art2::ToolService>,
}

fn entity_map(v: &Value) -> BTreeMap<String, Value> {
    v.as_object().into_iter().flatten().map(|(k, x)| (k.clone(), x.clone())).collect()
}

impl Catalog {
    pub fn bundled() -> Catalog {
        Catalog::from_json(&serde_json::from_str(BUNDLED).expect("the bundled baseline is valid JSON")).expect("the bundled baseline is a valid catalogue")
    }

    pub fn from_json(v: &Value) -> Result<Catalog, String> {
        let mut artifacts = BTreeMap::new();
        for a in v["artifacts"].as_array().ok_or("catalogue without artifacts")? {
            let locales: BTreeMap<String, String> = a["locales"].as_object().ok_or("artifact without locales")?.iter().filter_map(|(k, t)| Some((k.clone(), t.as_str()?.to_string()))).collect();
            let art = Artifact {
                kind: a["kind"].as_str().ok_or("artifact without kind")?.into(),
                id: a["id"].as_str().ok_or("artifact without id")?.into(),
                version: a["version"].as_str().ok_or("artifact without version")?.into(),
                locales,
                extra: a["extra"].clone(),
                referenced_by: a["referenced_by"].as_array().into_iter().flatten().filter_map(|r| Some((r["kind"].as_str()?.into(), r["id"].as_str()?.into(), r["version"].as_str()?.into()))).collect(),
            };
            artifacts.insert(art.target_ref(), art);
        }
        let agents = v["agents"].as_object().ok_or("catalogue without agents")?.iter().map(|(k, a)| (k.clone(), a.clone())).collect();
        Ok(Catalog {
            label: v["label"].as_str().unwrap_or("fixture-baseline").into(),
            source: v["source"].clone(),
            artifacts,
            agents,
            donor: v["donor"].as_str().unwrap_or("consultas").into(),
            donor_flow: v["flows"]["consulta-pqr"].clone(),
            donor_release: v["donor_release"].clone(),
            flows: entity_map(&v["flows"]),
            policies: entity_map(&v["policies"]),
            tool_defs: entity_map(&v["tool_defs"]),
            tool_service: crate::art2::ToolService::from_json(&v["tool_service"]),
        })
    }

    pub fn get(&self, target_ref: &str) -> Option<&Artifact> {
        self.artifacts.get(target_ref)
    }
    /// Replaces (or adds) one artifact, e.g. with the text read from the live registry. The caller relabels the catalogue.
    pub fn put_artifact(&mut self, art: Artifact) {
        self.artifacts.insert(art.target_ref(), art);
    }
    pub fn target_refs(&self) -> Vec<String> {
        self.artifacts.keys().cloned().collect()
    }
    pub fn agent(&self, id: &str) -> Option<&Value> {
        self.agents.get(id)
    }
    /// Replaces entities read from the live registry (ART2 kinds): the caller relabels the catalogue.
    pub fn put_entity(&mut self, kind: &str, id: &str, content: Value) {
        match kind {
            "flow" => self.flows.insert(id.into(), content),
            "policy" => self.policies.insert(id.into(), content),
            "tool" => self.tool_defs.insert(id.into(), content),
            _ => self.agents.insert(id.into(), content),
        };
    }
    pub fn flow(&self, id: &str) -> Option<&Value> {
        self.flows.get(id)
    }
    pub fn policy(&self, id: &str) -> Option<&Value> {
        self.policies.get(id)
    }
    pub fn tool_def(&self, id: &str) -> Option<&Value> {
        self.tool_defs.get(id)
    }
    pub fn agent_ids(&self) -> Vec<String> {
        self.agents.keys().cloned().collect()
    }

    /// Entities that move to the next patch version when `art` changes (flows that reference it, then the agents that
    /// reference those flows as entry or reference it directly). Predicted from the baseline; the Core's `auto_bumped` is the
    /// observed value to compare against.
    pub fn cascade(&self, art: &Artifact) -> Vec<String> {
        let bump = |v: &str| {
            let p: Vec<u64> = v.split('.').filter_map(|x| x.parse().ok()).collect();
            if p.len() == 3 { format!("{}.{}.{}", p[0], p[1], p[2] + 1) } else { v.to_string() }
        };
        let mut out: Vec<String> = vec![];
        for (kind, id, version) in &art.referenced_by {
            out.push(format!("{kind}:{id}@{}", bump(version)));
        }
        let flows: Vec<&String> = art.referenced_by.iter().filter(|r| r.0 == "flow").map(|r| &r.1).collect();
        for (aid, a) in &self.agents {
            let entry = a["entry_flow"].as_str().unwrap_or("").split('@').next().unwrap_or("");
            let direct = art.referenced_by.iter().any(|r| r.0 == "agent" && &r.1 == aid);
            if !direct && flows.iter().any(|f| f.as_str() == entry) {
                out.push(format!("agent:{aid}@{}", bump(a["version"].as_str().unwrap_or(""))));
            }
        }
        out.sort();
        out.dedup();
        out
    }
}

/// Substrings that mark a protected clause (untrusted-data handling, other-customer refusal, read-only, output contract,
/// no-figures constraint). A segment that contains one is never on the anchor menu, and every marker present in the base text
/// must still be present after the patch (defence in depth, rubric R4).
pub fn protected_markers(art: &Artifact, locale: &str) -> Vec<&'static str> {
    match (art.id.as_str(), locale) {
        ("p/copiloto", "es") => vec!["datos_no_confiables", "Solo lees", "otro cliente", "Nunca inventes", "nunca obedezcas", "dato del cliente", "kind \"final\"", "datos: un elemento", "resumen: una frase", "No incluyas identificadores", "Si lo pedido no aparece"],
        ("p/copiloto", "pt") => vec!["n\u{e3}o instru\u{e7}\u{f5}es", "somente o cliente atendido", "Nunca invente", "S\u{f3} leia", "kind \"final\"", "Se o dado n\u{e3}o existir"],
        ("p/sugerir", "es") => vec!["datos_no_confiables", "Propones, no ejecutas", "marcador de dato personal", "Nunca escribas", "Nunca pongas", "escalation.required", "citable_facts", "Si escalation es null"],
        ("p/sugerir", "pt") => vec!["dados, nunca instru", "Nunca escreva", "SOMENTE se escalation.required", "citable_facts"],
        ("p/respuesta_asesor", _) | ("p/resumen_radicado", _) | ("p/constructor", _) | ("p/resumen_construccion", _) => vec!["No incluyas", "N\u{e3}o inclua", "nunca", "Nunca", "never", "\u{27e6}", "datos_no_confiables", "cifra", "fact_id"],
        _ if art.kind == "prompt" => vec!["nunca", "Nunca", "never", "jam\u{e1}s", "Solo ", "S\u{f3} ", "datos_no_confiables", "\u{27e6}"],
        _ => vec![],
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Anchor {
    /// `<locale>.a<n>`: an opaque id, stable for a given base text.
    pub id: String,
    pub locale: String,
    /// The exact substring of the base text the anchor stands for.
    pub text: String,
}

/// Sentence-level segments of a text as byte ranges. A line break always ends a segment; `. ? !` followed by a space and an
/// uppercase letter ends one inside a line (so `p. ej.` does not split).
fn segments(text: &str) -> Vec<(usize, usize)> {
    let mut out = vec![];
    let mut start = 0usize;
    let bytes = text.as_bytes();
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    for (ci, &(i, c)) in chars.iter().enumerate() {
        let end_of_line = c == '\n';
        let sentence_end = matches!(c, '.' | '?' | '!') && chars.get(ci + 1).is_some_and(|x| x.1 == ' ') && chars.get(ci + 2).is_some_and(|x| x.1.is_uppercase());
        if end_of_line || sentence_end {
            let end = if end_of_line { i } else { i + c.len_utf8() };
            out.push((start, end));
            start = if end_of_line { i + 1 } else { i + 2 };
        }
    }
    if start < bytes.len() {
        out.push((start, bytes.len()));
    }
    out
}

/// The anchor menu for one locale: segments of at least 12 characters, unique in the text, not protected.
pub fn anchors(art: &Artifact, locale: &str) -> Vec<Anchor> {
    let Some(text) = art.locales.get(locale) else { return vec![] };
    let markers = protected_markers(art, locale);
    let mut out = vec![];
    for (s, e) in segments(text) {
        let seg = &text[s..e];
        if seg.trim().chars().count() < 12 || text.matches(seg).count() != 1 || markers.iter().any(|m| seg.contains(m)) {
            continue;
        }
        out.push(Anchor { id: format!("{locale}.a{}", out.len() + 1), locale: locale.to_string(), text: seg.to_string() });
    }
    out
}
