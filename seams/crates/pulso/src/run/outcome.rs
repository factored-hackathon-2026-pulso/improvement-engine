//! OUT1: the outcome step of `pulso run` (plan item W2-1, the last beat: did it work?).
//!
//! Triggered by an `outcome` trigger of type `release.published|promoted|revoked` (`POST /internal/v1/automation/triggers`). The
//! release event carries `release_id` and `proposal_id`; the proposal is mapped to the finding(s) the value loop announced it for
//! (the readable record `<work>/value-loop/<job>.json`: `delivery.proposal_id`, `metric`, `dims`). For each finding the step builds
//! a small aggregate table (the finding's cell and its same-metric sibling controls; the pre months from the pre cell tables, the
//! post months from the post cell tables), runs the configured estimator as a SUBPROCESS over the JSON CLI contract
//! (`docs/dev/OUTCOME_STEP.md`), parses its verdict and keeps a verdict card.
//!
//! Honesty rules, enforced here and tested:
//! - the card never claims success unless the estimator said `improved` AND the interval supports it (effect and upper bound below
//!   zero; every metric of the cell tables is "higher is worse"); anything else is downgraded to `inconclusive`;
//! - `inconclusive` (underpowered included) is a valid, expected result and is reported as such;
//! - the estimator is a descriptive pre/post comparison against sibling cells: "association, not cause". It is never "caused";
//! - on the historical bank data the post period is a PSEUDO-release (no release happened): the card says so;
//! - any failure of the estimator (missing, crash, timeout, bad output, unknown vocabulary) is a closed `inconclusive`, never a
//!   verdict, and never a failure of the job;
//! - only aggregates and ids reach the estimator and the card: no rows, no free text.
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub const CONTRACT: &str = "outcome-card/out1-0";
pub const ESTIMATOR_CONTRACT: &str = "pulso.outcome.v1";
pub const DEFAULT_CMD: &str = "python -m scripts.aggregate.outcome";
const VOCABULARY: [&str; 4] = ["improved", "no_detectable_change", "worsened", "inconclusive"];
/// Same order as the estimator's control dimensions: the first one present in the cell's dims is the one that varies.
const CONTROL_DIMENSIONS: [&str; 3] = ["reason_category", "category", "channel"];

type Dims = BTreeMap<String, String>;

/// What an outcome trigger says (whitelisted ids only; see TRIGGERS.md).
#[derive(Debug, Clone, PartialEq)]
pub struct Trigger {
    pub event_type: String,
    pub release_id: Option<String>,
    pub proposal_id: Option<String>,
    pub agent: Option<String>,
    /// `event.at` (RFC 3339): its `YYYY-MM` is the release period unless a pseudo-release is configured.
    pub event_at: Option<String>,
}

impl Trigger {
    /// `None` unless the view is an `outcome` trigger of a `release.*` event type.
    pub fn from_view(view: &Value) -> Option<Trigger> {
        let ty = view["event_type"].as_str()?;
        if view["kind"] != "outcome" || !matches!(ty, "release.published" | "release.promoted" | "release.revoked") {
            return None;
        }
        let s = |k: &str| view["subject"][k].as_str().map(str::to_string);
        Some(Trigger { event_type: ty.to_string(), release_id: s("release_id").or_else(|| s("release")), proposal_id: s("proposal_id"), agent: s("agent"), event_at: view["event_at"].as_str().map(str::to_string) })
    }
}

/// The trigger the keyed job `trigger:<key>` was admitted for, read back from the automation audit run of the in-process store.
/// (Audit events are in memory: after a restart this is `None` and the job falls back to its previous behaviour.)
pub fn find_trigger(store: &debug_api::Store, job_key: &str) -> Option<Trigger> {
    let key = job_key.strip_prefix("trigger:")?;
    let evs = store.events_after("automation-audit", 0, 100_000);
    evs.iter().find(|e| e["kind"] == "automation_trigger_received" && e["entity_ref"]["id"] == key).and_then(|e| Trigger::from_view(&e["data"]))
}

/// The estimator behind the JSON CLI contract.
pub trait Estimator: Send + Sync {
    /// `Ok(json)` = the parsed stdout of a successful call. `Err(code)` = a closed failure code.
    fn estimate(&self, cells: &Path, treated: &Path, release: &str, window_months: u32) -> Result<Value, String>;
    fn label(&self) -> String;
}

/// `<cmd...> --cells <ndjson> --treated <json> --release-date YYYY-MM --window-months N`; stdout = one JSON object.
pub struct Subprocess {
    pub cmd: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub timeout: Duration,
}

impl Estimator for Subprocess {
    fn label(&self) -> String {
        self.cmd.join(" ")
    }
    fn estimate(&self, cells: &Path, treated: &Path, release: &str, window_months: u32) -> Result<Value, String> {
        let (exe, args) = self.cmd.split_first().ok_or("estimator_not_configured")?;
        let mut c = Command::new(exe);
        c.args(args).arg("--cells").arg(cells).arg("--treated").arg(treated).arg("--release-date").arg(release).arg("--window-months").arg(window_months.to_string());
        if let Some(d) = &self.cwd {
            c.current_dir(d);
        }
        let mut child = c.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().map_err(|_| "estimator_not_runnable".to_string())?;
        let out = child.stdout.take().ok_or("estimator_no_stdout")?;
        let reader = std::thread::spawn(move || {
            let mut s = String::new();
            let _ = out.take(8 << 20).read_to_string(&mut s);
            s
        });
        let start = Instant::now();
        let status = loop {
            match child.try_wait() {
                Ok(Some(s)) => break s,
                Ok(None) if start.elapsed() > self.timeout => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err("estimator_timeout".into());
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(20)),
                Err(_) => return Err("estimator_wait_failed".into()),
            }
        };
        let text = reader.join().unwrap_or_default();
        if !status.success() {
            return Err(format!("estimator_exit_{}", status.code().map_or("signal".to_string(), |c| c.to_string())));
        }
        serde_json::from_str(&text).map_err(|_| "estimator_output_not_json".to_string())
    }
}

pub struct OutcomeStep {
    pub work: PathBuf,
    pub pre_cells: PathBuf,
    /// `None` = the pre tables are also the post tables (historical replay: a PSEUDO-release).
    pub post_cells: Option<PathBuf>,
    /// `YYYY-MM` that replaces the release period of the event (historical data). Implies `period_kind = pseudo_release_historical`.
    pub pseudo_release: Option<String>,
    pub window_months: u32,
    /// `PULSO_OUTCOME_DATA_LABEL` (e.g. `synthetic`): travels on every card and in its caveats.
    pub data_label: Option<String>,
    pub estimator: Arc<dyn Estimator>,
}

pub fn valid_period(v: &str) -> bool {
    v.len() == 7 && v.is_ascii() && v.as_bytes()[4] == b'-' && v[..4].bytes().all(|b| b.is_ascii_digit()) && v[5..].bytes().all(|b| b.is_ascii_digit()) && (1..=12).contains(&v[5..].parse::<u32>().unwrap_or(0))
}

fn ordinal(p: &str) -> i64 {
    p[..4].parse::<i64>().unwrap_or(0) * 12 + p[5..].parse::<i64>().unwrap_or(1) - 1
}

fn shift(p: &str, n: i64) -> String {
    let o = ordinal(p) + n;
    format!("{:04}-{:02}", o.div_euclid(12), o.rem_euclid(12) + 1)
}

fn slug(v: &str) -> String {
    v.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') { c } else { '_' }).take(100).collect()
}

fn dims_of(v: &Value) -> Option<Dims> {
    v.as_object().filter(|o| !o.is_empty()).map(|o| o.iter().filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string()))).collect())
}

/// Same-metric cells that differ from the treated cell only in its first control dimension (same channel, other reasons).
fn is_sibling(metric: &str, dims: &Dims, row: &Value) -> bool {
    if row["metric"] != metric {
        return false;
    }
    let Some(rd) = dims_of(&row["dims"]) else { return false };
    let Some(cd) = CONTROL_DIMENSIONS.iter().find(|d| dims.contains_key(**d)) else { return false };
    rd.keys().eq(dims.keys()) && rd.iter().all(|(k, v)| if k == cd { dims[k] != *v } else { dims[k] == *v })
}

fn read_rows(p: &Path) -> Result<Vec<Value>, String> {
    let t = std::fs::read_to_string(p).map_err(|_| "cells_unreadable".to_string())?;
    t.lines().filter(|l| !l.trim().is_empty()).map(|l| serde_json::from_str::<Value>(l).map_err(|_| "cells_not_ndjson".to_string())).collect()
}

/// The aggregate table handed to the estimator: the treated cell and its sibling controls, `window` months before the release from
/// the pre tables and `window` months after it from the post tables. The release month itself is in neither window.
pub fn build_table(pre: &[Value], post: &[Value], metric: &str, dims: &Dims, release: &str, window: u32) -> Vec<Value> {
    let w = i64::from(window);
    let pre_months: BTreeSet<String> = (1..=w).map(|n| shift(release, -n)).collect();
    let post_months: BTreeSet<String> = (1..=w).map(|n| shift(release, n)).collect();
    let keep = |r: &Value, months: &BTreeSet<String>| {
        r["period"].as_str().is_some_and(|p| months.contains(p)) && ((r["metric"] == metric && dims_of(&r["dims"]).as_ref() == Some(dims)) || is_sibling(metric, dims, r))
    };
    let mut out: Vec<Value> = pre.iter().filter(|r| keep(r, &pre_months)).cloned().collect();
    out.extend(post.iter().filter(|r| keep(r, &post_months)).cloned());
    out.sort_by_key(|r| (r["metric"].to_string(), r["dims"].to_string(), r["half"].to_string(), r["period"].to_string()));
    out
}

fn num(v: &Value) -> Option<f64> {
    v.as_f64().filter(|x| x.is_finite())
}

/// A parsed estimator answer for the treated cell, already held to the closed vocabulary and to the success rule.
#[derive(Debug, Clone, PartialEq)]
pub struct Verdict {
    pub status: String,
    pub reason: Option<String>,
    pub effect_pp: Option<f64>,
    pub interval_pp: Option<[f64; 2]>,
    pub n_pre: u64,
    pub n_post: u64,
    pub control_dimension: Option<String>,
    pub controls: Vec<Value>,
    pub uncertainty_method: Option<String>,
    pub downgraded_from: Option<String>,
}

fn inconclusive(reason: &str) -> Verdict {
    Verdict { status: "inconclusive".into(), reason: Some(reason.into()), effect_pp: None, interval_pp: None, n_pre: 0, n_post: 0, control_dimension: None, controls: vec![], uncertainty_method: None, downgraded_from: None }
}

/// Reads the treated cell out of the estimator's output (`cells[]` of `pulso.outcome.v1`; Codex T1's cell shape).
pub fn parse_verdict(out: &Value, metric: &str, dims: &Dims) -> Verdict {
    if out["contract"] != ESTIMATOR_CONTRACT {
        return inconclusive("estimator_contract_mismatch");
    }
    let matches: Vec<&Value> = out["cells"].as_array().into_iter().flatten().filter(|c| c["metric"] == metric && dims_of(&c["dims"]).as_ref() == Some(dims)).collect();
    let [cell] = matches.as_slice() else { return inconclusive(if matches.is_empty() { "treated_cell_not_in_estimator_output" } else { "treated_cell_ambiguous_in_estimator_output" }) };
    let Some(status) = cell["status"].as_str().filter(|s| VOCABULARY.contains(s)) else { return inconclusive("estimator_status_outside_vocabulary") };
    let interval = cell["interval_family_adjusted_pp"].as_array().filter(|a| a.len() == 2).and_then(|a| Some([num(&a[0])?, num(&a[1])?]));
    let mut v = Verdict {
        status: status.to_string(),
        reason: cell["reason"].as_str().map(str::to_string),
        effect_pp: num(&cell["effect_pp"]),
        interval_pp: interval,
        n_pre: cell["n_pre"].as_u64().unwrap_or(0),
        n_post: cell["n_post"].as_u64().unwrap_or(0),
        control_dimension: cell["control_dimension"].as_str().map(str::to_string),
        controls: cell["control_siblings"].as_array().cloned().unwrap_or_default(),
        uncertainty_method: cell["uncertainty_method"].as_str().map(str::to_string),
        downgraded_from: None,
    };
    // A directional claim must be backed by the numbers. Every metric is "higher is worse": improved = effect and the whole
    // interval below zero; worsened = above zero.
    let backed = match status {
        "improved" => v.effect_pp.is_some_and(|e| e < 0.0) && v.interval_pp.is_some_and(|i| i[0] <= i[1] && i[1] < 0.0),
        "worsened" => v.effect_pp.is_some_and(|e| e > 0.0) && v.interval_pp.is_some_and(|i| i[0] <= i[1] && i[0] > 0.0),
        _ => true,
    };
    if !backed {
        v.downgraded_from = Some(status.to_string());
        v.status = "inconclusive".into();
        v.reason = Some(format!("{status}_not_supported_by_interval"));
    }
    v
}

fn metric_label(m: &str) -> (&'static str, &'static str) {
    match m {
        "M1" => ("la tasa de contactos sin resolver", "a taxa de contatos nao resolvidos"),
        "M2" => ("la proporcion de quejas entre los contactos", "a proporcao de reclamacoes entre os contatos"),
        "M3" => ("la proporcion de quejas entre los contactos sin resolver", "a proporcao de reclamacoes entre os contatos nao resolvidos"),
        "M4" => ("la tasa de PQR abiertas", "a taxa de PQR abertas"),
        "M5" => ("la tasa de PQR fuera de plazo", "a taxa de PQR fora do prazo"),
        "M6" => ("la tasa de encuestas con puntaje bajo", "a taxa de pesquisas com nota baixa"),
        _ => ("la tasa de la metrica", "a taxa da metrica"),
    }
}

fn reason_texts(reason: &str) -> (&'static str, &'static str) {
    if reason.starts_with("underpowered") {
        ("muestra insuficiente (poder estadistico bajo) para detectar un cambio en la ventana observada", "amostra insuficiente (poder estatistico baixo) para detectar uma mudanca na janela observada")
    } else if reason.starts_with("half_directions") {
        ("las dos mitades de clientes no coinciden en direccion o intervalo", "as duas metades de clientes nao coincidem em direcao ou intervalo")
    } else if reason.starts_with("incomplete") || reason.starts_with("no_published") {
        ("faltan meses o celdas de control en la ventana", "faltam meses ou celulas de controle na janela")
    } else {
        ("el estimador no pudo concluir con la evidencia disponible", "o estimador nao conseguiu concluir com a evidencia disponivel")
    }
}

fn f1(x: f64) -> String {
    format!("{x:.1}")
}

/// Short dossier-style text for the platform, Spanish and Portuguese. A success sentence exists only for `improved`.
pub fn dossier(v: &Verdict, metric: &str, cell_label: &str, release_id: &str, pseudo: bool) -> Value {
    let (m_es, m_pt) = metric_label(metric);
    let span = match (v.effect_pp, v.interval_pp) {
        (Some(e), Some(i)) => Some((f1(e.abs()), f1(i[0]), f1(i[1]))),
        _ => None,
    };
    let ns = format!("n previo {} / posterior {}", v.n_pre, v.n_post);
    let ns_pt = format!("n anterior {} / posterior {}", v.n_pre, v.n_post);
    let (es, pt) = match (v.status.as_str(), &span) {
        ("improved", Some((e, lo, hi))) => (
            format!("Resultado de la version {release_id} en {cell_label}: {m_es} bajo {e} pp frente a las celdas de control (intervalo {lo} a {hi} pp; {ns}). Es una asociacion observada, no una prueba de causa."),
            format!("Resultado da versao {release_id} em {cell_label}: {m_pt} caiu {e} pp em relacao as celulas de controle (intervalo {lo} a {hi} pp; {ns_pt}). E uma associacao observada, nao uma prova de causa."),
        ),
        ("worsened", Some((e, lo, hi))) => (
            format!("Resultado de la version {release_id} en {cell_label}: {m_es} subio {e} pp frente a las celdas de control (intervalo {lo} a {hi} pp; {ns}). Conviene revisar la version. Es una asociacion, no una prueba de causa."),
            format!("Resultado da versao {release_id} em {cell_label}: {m_pt} subiu {e} pp em relacao as celulas de controle (intervalo {lo} a {hi} pp; {ns_pt}). Convem revisar a versao. E uma associacao, nao uma prova de causa."),
        ),
        ("no_detectable_change", _) => (
            format!("Resultado de la version {release_id} en {cell_label}: no se detecta un cambio material en {m_es} frente a las celdas de control ({ns}). No se afirma ningun efecto."),
            format!("Resultado da versao {release_id} em {cell_label}: nao se detecta mudanca material em {m_pt} em relacao as celulas de controle ({ns_pt}). Nenhum efeito e afirmado."),
        ),
        _ => {
            let (r_es, r_pt) = reason_texts(v.reason.as_deref().unwrap_or(""));
            (
                format!("Resultado de la version {release_id} en {cell_label}: sin conclusion, {r_es}. No se afirma ningun efecto, ni a favor ni en contra."),
                format!("Resultado da versao {release_id} em {cell_label}: sem conclusao, {r_pt}. Nenhum efeito e afirmado, nem a favor nem contra."),
            )
        }
    };
    if pseudo {
        return json!({"es": format!("{es} Datos historicos: la fecha de version es un pseudo-lanzamiento, no hubo cambio real."), "pt": format!("{pt} Dados historicos: a data da versao e um pseudo-lancamento, nao houve mudanca real.")});
    }
    json!({"es": es, "pt": pt})
}

fn power_note(v: &Verdict) -> String {
    if v.reason.as_deref().is_some_and(|r| r.starts_with("underpowered")) {
        return "inconclusive: underpowered. The treated or control window has less support than the pre-registered minimum; no effect can be claimed or excluded.".into();
    }
    match v.interval_pp {
        Some(i) => format!("interval half-width {:.2} pp over n {} before / {} after; a change smaller than that cannot be told from noise here.", (i[1] - i[0]) / 2.0, v.n_pre, v.n_post),
        None => format!("no interval: {}.", v.reason.as_deref().unwrap_or("no estimate")),
    }
}

impl OutcomeStep {
    /// `PULSO_OUTCOME_PRE_CELLS` (default `PULSO_CELLS_NDJSON`) turns the step on; `PULSO_OUTCOME=off` turns it off.
    /// `PULSO_OUTCOME_POST_CELLS`, `PULSO_OUTCOME_PSEUDO_RELEASE` (YYYY-MM), `PULSO_OUTCOME_WINDOW_MONTHS` (1..=12, default 3),
    /// `PULSO_OUTCOME_CMD` (default `python -m scripts.aggregate.outcome`), `PULSO_OUTCOME_CWD`, `PULSO_OUTCOME_TIMEOUT_SECS` (default 120).
    pub fn from_lookup(get: &dyn Fn(&str) -> Option<String>, work: Option<&Path>) -> Result<Option<OutcomeStep>, String> {
        let on = |k: &str| get(k).filter(|v| !v.is_empty());
        if matches!(on("PULSO_OUTCOME").as_deref(), Some("off" | "0" | "false")) {
            return Ok(None);
        }
        let Some(pre) = on("PULSO_OUTCOME_PRE_CELLS").or_else(|| on("PULSO_CELLS_NDJSON")) else { return Ok(None) };
        let work = work.ok_or("PULSO_WORK_DIR is required with the outcome step")?.to_path_buf();
        let pseudo = on("PULSO_OUTCOME_PSEUDO_RELEASE");
        if pseudo.as_deref().is_some_and(|p| !valid_period(p)) {
            return Err("PULSO_OUTCOME_PSEUDO_RELEASE is not YYYY-MM".into());
        }
        let window_months = match on("PULSO_OUTCOME_WINDOW_MONTHS") {
            None => 3,
            Some(v) => v.parse::<u32>().ok().filter(|n| (1..=12).contains(n)).ok_or("PULSO_OUTCOME_WINDOW_MONTHS is not 1..=12")?,
        };
        let cmd: Vec<String> = on("PULSO_OUTCOME_CMD").unwrap_or_else(|| DEFAULT_CMD.into()).split_whitespace().map(str::to_string).collect();
        let timeout = Duration::from_secs(on("PULSO_OUTCOME_TIMEOUT_SECS").and_then(|v| v.parse().ok()).filter(|n| *n >= 1).unwrap_or(120));
        Ok(Some(OutcomeStep {
            work,
            pre_cells: PathBuf::from(pre),
            post_cells: on("PULSO_OUTCOME_POST_CELLS").map(PathBuf::from),
            pseudo_release: pseudo,
            window_months,
            data_label: on("PULSO_OUTCOME_DATA_LABEL").map(|l| slug(&l)),
            estimator: Arc::new(Subprocess { cmd, cwd: on("PULSO_OUTCOME_CWD").map(PathBuf::from), timeout }),
        }))
    }

    fn dir(&self) -> PathBuf {
        self.work.join("outcome")
    }

    /// Findings the proposal was announced for, with the value-loop record file each came from.
    fn findings_for(&self, proposal_id: &str) -> Vec<(PathBuf, usize, Value)> {
        let mut out = vec![];
        let Ok(rd) = std::fs::read_dir(self.work.join("value-loop")) else { return out };
        let mut files: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().is_some_and(|x| x == "json")).collect();
        files.sort();
        for p in files {
            let Some(doc) = std::fs::read_to_string(&p).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) else { continue };
            for (i, f) in doc["findings"].as_array().into_iter().flatten().enumerate() {
                if f["delivery"]["proposal_id"] == proposal_id {
                    out.push((p.clone(), i, f.clone()));
                }
            }
        }
        out
    }

    /// Handles one `release.*` trigger. Idempotent per release id: a stored card is returned as is (no second estimator call).
    /// `Err` only for a store that cannot be written; every other failure is a closed outcome inside the returned record.
    pub fn run(&self, t: &Trigger) -> Result<Value, String> {
        let Some(release_id) = t.release_id.as_deref() else {
            return Ok(json!({"contract": CONTRACT, "state": "unlinked", "reason": "release_id_missing", "event_type": t.event_type, "cards": [], "success_claimed": false}));
        };
        std::fs::create_dir_all(self.dir()).map_err(|e| format!("outcome dir: {e}"))?;
        let card_path = self.dir().join(format!("{}.json", slug(release_id)));
        if let Some(mut prev) = std::fs::read_to_string(&card_path).ok().and_then(|s| serde_json::from_str::<Value>(&s).ok()) {
            if t.event_type == "release.revoked" && prev["revoked"] != true {
                prev["revoked"] = json!(true);
                self.write(&card_path, &prev)?;
            }
            prev["replay"] = json!(true);
            return Ok(prev);
        }
        let mut doc = json!({"contract": CONTRACT, "release_id": release_id, "proposal_id": t.proposal_id, "agent": t.agent, "event_type": t.event_type,
            "association_not_cause": true, "success_claimed": false, "replay": false, "cards": []});
        if t.event_type == "release.revoked" {
            doc["state"] = json!("revoked_not_measured");
            doc["revoked"] = json!(true);
            self.write(&card_path, &doc)?;
            return Ok(doc);
        }
        let Some(proposal_id) = t.proposal_id.as_deref() else {
            doc["state"] = json!("unlinked");
            doc["reason"] = json!("proposal_id_missing");
            self.write(&card_path, &doc)?;
            return Ok(doc);
        };
        let found = self.findings_for(proposal_id);
        if found.is_empty() {
            doc["state"] = json!("unlinked");
            doc["reason"] = json!("no_finding_announced_for_proposal");
            self.write(&card_path, &doc)?;
            return Ok(doc);
        }
        let pseudo = self.pseudo_release.is_some();
        let release = self.pseudo_release.clone().or_else(|| t.event_at.as_deref().filter(|a| a.is_ascii() && a.len() >= 7 && valid_period(&a[..7])).map(|a| a[..7].to_string()));
        let pre = read_rows(&self.pre_cells);
        let post = self.post_cells.as_ref().map(|p| read_rows(p));
        let mut cards = vec![];
        for (n, (file, idx, f)) in found.iter().enumerate() {
            let card = self.card(n, f, release.as_deref(), pseudo, &pre, post.as_ref().unwrap_or(&pre), release_id);
            self.patch_finding(file, *idx, json!({"verdict": card["verdict"], "release_id": release_id, "card": format!("outcome/{}.json", slug(release_id))}));
            cards.push(card);
        }
        doc["state"] = json!("measured");
        doc["success_claimed"] = json!(cards.iter().any(|c| c["success_claimed"] == true));
        doc["cards"] = json!(cards);
        self.write(&card_path, &doc)?;
        Ok(doc)
    }

    fn card(&self, n: usize, f: &Value, release: Option<&str>, pseudo: bool, pre: &Result<Vec<Value>, String>, post: &Result<Vec<Value>, String>, release_id: &str) -> Value {
        let metric = f["metric"].as_str().unwrap_or("").to_string();
        let dims = dims_of(&f["dims"]);
        let mut card = json!({"finding": {"evidence_ref": f["evidence_ref"], "metric": metric, "dims": f["dims"]},
            "period_kind": if pseudo { "pseudo_release_historical" } else { "live_release_event" }, "success_claimed": false});
        let (Some(release), Some(dims)) = (release, dims.as_ref()) else {
            let why = if release.is_none() { "release_period_unknown" } else { "finding_record_without_cell" };
            self.fill(&mut card, &inconclusive(why), &metric, dims.as_ref(), release_id, pseudo);
            return card;
        };
        card["window"] = json!({"release_period": release, "months_each_side": self.window_months, "release_month_excluded": true});
        let (pre_rows, post_rows) = match (pre, post) {
            (Ok(a), Ok(b)) => (a, b),
            (Err(e), _) | (_, Err(e)) => {
                self.fill(&mut card, &inconclusive(e), &metric, Some(dims), release_id, pseudo);
                return card;
            }
        };
        let table = build_table(pre_rows, post_rows, &metric, dims, release, self.window_months);
        let tag = format!("{}.{n}", slug(release_id));
        let (cells_path, treated_path) = (self.dir().join(format!("{tag}.cells.ndjson")), self.dir().join(format!("{tag}.treated.json")));
        let body: String = table.iter().map(|r| format!("{r}\n")).collect();
        let written = std::fs::write(&cells_path, body).and_then(|()| std::fs::write(&treated_path, json!({"metric": metric, "dims": dims}).to_string()));
        let v = if written.is_err() {
            inconclusive("outcome_dir_not_writable")
        } else {
            match self.estimator.estimate(&cells_path, &treated_path, release, self.window_months) {
                Ok(out) => parse_verdict(&out, &metric, dims),
                Err(code) => inconclusive(&format!("estimator_failed:{code}")),
            }
        };
        card["table"] = json!({"rows": table.len(), "pre_from": self.pre_cells.file_name().and_then(|s| s.to_str()),
            "post_from": self.post_cells.as_ref().map_or("same tables as pre (pseudo-release)".to_string(), |p| p.file_name().and_then(|s| s.to_str()).unwrap_or("?").to_string())});
        card["estimator"] = json!({"command": self.estimator.label(), "contract": ESTIMATOR_CONTRACT});
        self.fill(&mut card, &v, &metric, Some(dims), release_id, pseudo);
        card
    }

    fn fill(&self, card: &mut Value, v: &Verdict, metric: &str, dims: Option<&Dims>, release_id: &str, pseudo: bool) {
        let label = dims.map_or("celda desconocida".to_string(), |d| d.values().cloned().collect::<Vec<_>>().join(" / "));
        let underpowered = v.reason.as_deref().is_some_and(|r| r.starts_with("underpowered"));
        let mut caveats: Vec<String> = vec![
            "association, not cause: a descriptive pre/post comparison against sibling cells; nothing was randomized and nothing here shows the release caused the change".to_string(),
            "one release, one cell, a short window: other changes in the same months are not excluded".to_string(),
        ];
        if let Some(l) = &self.data_label {
            caveats.push(format!("data label: {l}. A synthetic label means the numbers were planted to exercise the pipeline: it is not a result about any release"));
            card["data_label"] = json!(l);
        }
        if pseudo {
            caveats.push("historical data: the release date is a PSEUDO-release (no release happened); the post period is only the months after an arbitrary date".into());
        }
        if v.uncertainty_method.as_deref().is_some_and(|m| m.contains("naive")) {
            caveats.push("the interval treats contacts as independent (not adjusted for repeat customers): it is narrower than it should be".into());
        }
        if underpowered {
            caveats.push("inconclusive: underpowered is a valid and expected result for small cells; it is not a failure of the release".into());
        }
        if let Some(from) = &v.downgraded_from {
            caveats.push(format!("the estimator said {from} but its numbers do not support it: reported as inconclusive"));
        }
        card["verdict"] = json!(v.status);
        card["reason"] = json!(v.reason);
        card["underpowered"] = json!(underpowered);
        card["effect_pp"] = json!(v.effect_pp);
        card["interval_pp"] = json!(v.interval_pp);
        card["n_pre"] = json!(v.n_pre);
        card["n_post"] = json!(v.n_post);
        card["controls"] = json!({"kind": "same-metric sibling cells (same channel, other reasons); the customer-hash halves are replication cohorts that must agree, not controls", "dimension": v.control_dimension, "cells": v.controls});
        card["power_note"] = json!(power_note(v));
        card["caveats"] = json!(caveats);
        let mut d = dossier(v, metric, &label, release_id, pseudo);
        if self.data_label.as_deref().is_some_and(|l| l.contains("synthetic")) {
            d["es"] = json!(format!("{} DATOS SINTETICOS: efecto plantado para probar el flujo, no es un resultado real.", d["es"].as_str().unwrap_or("")));
            d["pt"] = json!(format!("{} DADOS SINTETICOS: efeito plantado para testar o fluxo, nao e um resultado real.", d["pt"].as_str().unwrap_or("")));
        }
        card["dossier"] = d;
        card["success_claimed"] = json!(v.status == "improved");
    }

    fn write(&self, path: &Path, v: &Value) -> Result<(), String> {
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(v).unwrap_or_default()).and_then(|()| std::fs::rename(&tmp, path)).map_err(|e| format!("outcome card: {e}"))
    }

    /// The card summary on the finding record (the readable copy of the value loop; the job-store record is immutable).
    fn patch_finding(&self, file: &Path, idx: usize, summary: Value) {
        let Some(mut doc) = std::fs::read_to_string(file).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) else { return };
        if let Some(f) = doc["findings"].get_mut(idx) {
            f["outcome_card"] = summary;
            let _ = self.write(file, &doc);
        }
    }
}
