//! Decision dossier (plan item W1-2): everything a supervisor needs to decide a proposal in about 30 seconds, assembled by code
//! from AUDITED AGGREGATES ONLY (the sensor signal, the compiled anchored patch, the REG1 verdict story) and written into the
//! agent-core `docs` fields of the proposal (`description`, `rationale`, `changelog`) plus the proposal `title`, in Spanish with
//! a Portuguese variant. Pure and deterministic: same inputs, same bytes. No model call, no clock, no randomness.
//!
//! Section labels and field names follow the Codex DOSSIER_SPEC (T4): Problema observado / Evidencia y comparación / Que
//! cambiaria / Efecto esperado / Cómo se evaluará / Riesgo / Qué no cambia, plus two engine sections the spec leaves open:
//! `Resultado base vs candidato` (the REG1 verdict story) and `Etiquetas` (honesty labels).
//!
//! Honesty rules enforced here, not left to the caller:
//! * `announce` is true only for a `regression_suite_proven` verdict on a corroborated finding with a real change. A missing
//!   verdict, `non_discriminating`, `not_fixed`, `guard_regressed`, `infra_failed`, `base_only`, `not_exercised` all give
//!   `announce: false` and a dossier that says so in the first line.
//! * The expected effect is always a hypothesis, labelled "no medido"; the claim is "association", never a cause.
//! * The `uncalibrated` label stays unless the caller passes `calibrated: true`. A run with any double is labelled `doubles`
//!   whatever the caller asked for. The rubric score is the structural self-score, never the judge.
//! * Every output string is refused when PII-shaped (an at-sign that is not `id@version`, a digit run of 6 or more, a PII token
//!   marker). Counts are printed with thousands separators so an audited count never forms a long digit run.
//! * Bounded: each section is capped, the description has a hard ceiling below the agent-core limit (4,000), and an
//!   over-long result is a typed error, never a silent truncation of the whole.
use serde_json::{Map, Value, json};

pub const SCHEMA: &str = "dossier/1";
/// agent-core `VersionDocs.description` allows 4,000; the dossier keeps a margin and is meant to be read in 30 seconds.
pub const DESCRIPTION_MAX: usize = 3800;
pub const RATIONALE_MAX: usize = 1500;
pub const CHANGELOG_MAX: usize = 1500;
pub const TITLE_MAX: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Es,
    Pt,
}

impl Lang {
    pub fn key(self) -> &'static str {
        match self {
            Lang::Es => "es",
            Lang::Pt => "pt",
        }
    }
    fn t(self, es: &str, pt: &str) -> String {
        if self == Lang::Es { es.to_string() } else { pt.to_string() }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Runtime {
    Doubles,
    Real,
    LocalModel,
}

impl Runtime {
    pub fn parse(s: &str) -> Option<Runtime> {
        match s {
            "doubles" => Some(Runtime::Doubles),
            "real" => Some(Runtime::Real),
            "local-model" | "local_model" => Some(Runtime::LocalModel),
            _ => None,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Runtime::Doubles => "doubles",
            Runtime::Real => "real",
            Runtime::LocalModel => "local-model",
        }
    }
}

/// Caller-supplied honesty labels. The default is the most conservative one.
#[derive(Debug, Clone)]
pub struct Labels {
    pub runtime: Runtime,
    /// Overrides the rubric found in the reasoning record (structural self-score `(total, max)`).
    pub rubric: Option<(u32, u32)>,
    /// Overrides the judge family derived from the verdict story's `model_policy`.
    pub judge_family: Option<String>,
    /// Only a human-reviewed calibration may set this; the default label is `uncalibrated`.
    pub calibrated: bool,
}

impl Default for Labels {
    fn default() -> Self {
        Labels { runtime: Runtime::Doubles, rubric: None, judge_family: None, calibrated: false }
    }
}

// ---------------------------------------------------------------------------------------------------------------- helpers

fn pct(x: f64) -> String {
    format!("{:.1}", x * 100.0).replace('.', ",")
}

fn pp(x: f64) -> String {
    format!("{:+.1}", x * 100.0).replace('.', ",")
}

/// `117021` -> `117.021` (no digit run of 6 or more can be produced by an audited count).
fn count(n: i64) -> String {
    let s = n.abs().to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push('.');
        }
        out.push(c);
    }
    if n < 0 { format!("-{out}") } else { out }
}

fn pval(p: f64) -> String {
    if p < 0.0001 { "<0,0001".to_string() } else { format!("{p:.4}").replace('.', ",") }
}

fn cap(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(n.saturating_sub(1)).collect();
        t.push('\u{2026}');
        t
    }
}

/// Wilson 95% interval of a proportion (deterministic, closed form).
fn wilson(num: i64, den: i64) -> (f64, f64) {
    if den <= 0 {
        return (0.0, 1.0);
    }
    let (n, p, z) = (den as f64, num as f64 / den as f64, 1.959_963_984_540_054_f64);
    let d = 1.0 + z * z / n;
    let c = p + z * z / (2.0 * n);
    let h = z * (p * (1.0 - p) / n + z * z / (4.0 * n * n)).sqrt();
    (((c - h) / d).max(0.0), ((c + h) / d).min(1.0))
}

fn stage(v: &Value) -> Option<(i64, i64, f64, f64, f64)> {
    Some((v["numerator"].as_i64()?, v["denominator"].as_i64()?, v["rate"].as_f64()?, v["baseline_rate"].as_f64()?, v["diff"].as_f64()?))
}

fn dims_text(dims: &Value) -> String {
    let m: Vec<String> = dims.as_object().map(|o| o.iter().filter_map(|(k, v)| Some(format!("{k}={}", v.as_str()?))).collect()).unwrap_or_default();
    m.join(", ")
}

/// Refuses any PII-shaped string. Same shapes the rest of the engine refuses (`clean_text`, rubric R11): an at-sign that is not an
/// `id@version` reference, a PII token marker, a digit run of 6 or more (evidence refs `ev_<hex>` excluded, a hash can hold one).
pub fn pii_shaped(s: &str) -> Option<&'static str> {
    if s.contains('\u{27e6}') || s.contains('\u{27e7}') {
        return Some("a PII token marker");
    }
    if crate::email_like(s) {
        return Some("an email-like at-sign");
    }
    if crate::rubric::long_digits(s, 6) {
        return Some("a digit run of 6 or more");
    }
    None
}

fn collect(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::String(s) => out.push(s.clone()),
        Value::Array(a) => a.iter().for_each(|x| collect(x, out)),
        Value::Object(m) => m.values().for_each(|x| collect(x, out)),
        _ => {}
    }
}

// ---------------------------------------------------------------------------------------------------------------- sections

fn problem(l: Lang, finding: &Value, level: bool) -> String {
    let metric = finding["metric"].as_str().unwrap_or("?");
    if level {
        l.t(
            &format!("{metric} se mantiene por encima de su umbral preregistrado de forma recurrente (nivel de riesgo, no un contraste contra el resto)."),
            &format!("{metric} permanece acima do límite pre-registrado de forma recorrente (nivel de risco, não um contraste com o resto)."),
        )
        .to_string()
    } else {
        let d = dims_text(&finding["dims"]);
        l.t(
            &format!("{metric} es persistentemente más alto en [{d}] que en su base de comparación; recurre en la ventana de descubrimiento y en la de replicación."),
            &format!("{metric} é persistentemente mais alto em [{d}] do que em sua base de comparação; recorre na janela de descoberta e na de replicação."),
        )
        .to_string()
    }
}

fn evidence(l: Lang, finding: &Value, level: bool) -> String {
    let Some((n, d, rate, base, diff)) = stage(&finding["discovery"]) else {
        return l.t("Sin etapa de descubrimiento en la señal: no hay evidencia numérica.", "Sem etapa de descoberta no sinal: não ha evidencia numérica.").to_string();
    };
    let (lo, hi) = if level {
        (finding["discovery"]["ci95_low"].as_f64().unwrap_or(0.0), finding["discovery"]["ci95_high"].as_f64().unwrap_or(1.0))
    } else {
        wilson(n, d)
    };
    let ci = format!("[{}; {}]", pct(lo), pct(hi));
    let same_channel = finding["dims"]["channel"].is_string();
    let hold = stage(&finding["holdout"]).map(|(hn, hd, hr, hb, _)| {
        l.t(
            &format!("Replicación (holdout): {}/{} = {} % frente a {} %.", count(hn), count(hd), pct(hr), pct(hb)),
            &format!("Replicação (holdout): {}/{} = {} % contra {} %.", count(hn), count(hd), pct(hr), pct(hb)),
        )
        .to_string()
    });
    let r2 = finding["r2"]["status"].as_str().map(|s| l.t(&format!(" Ventanas R2: {s}."), &format!(" Janelas R2: {s}.")).to_string()).unwrap_or_default();
    let p = finding["p_adj"].as_f64().map(|p| format!(" p ajustado {}.", pval(p))).unwrap_or_default();
    let mut s = if level {
        let periods = finding["periods"].as_object().map(|_| {
            l.t(
                &format!(" Períodos sobre el umbral: {}/{}; celdas: {}/{}.", finding["periods"]["above_threshold"], finding["periods"]["total"], finding["level_cells"]["above_threshold"], finding["level_cells"]["total"]),
                &format!(" Períodos acima do límite: {}/{}; células: {}/{}.", finding["periods"]["above_threshold"], finding["periods"]["total"], finding["level_cells"]["above_threshold"], finding["level_cells"]["total"]),
            )
            .to_string()
        });
        l.t(
            &format!("Nivel: {}/{} = {} % de los envíos, umbral {} % (exceso {} pp); IC95 % {ci}.", count(n), count(d), pct(rate), pct(base), pp(diff).trim_start_matches('+')),
            &format!("Nivel: {}/{} = {} % dos envíos, límite {} % (excesso {} pp); IC95 % {ci}.", count(n), count(d), pct(rate), pct(base), pp(diff).trim_start_matches('+')),
        )
        .to_string()
            + &periods.unwrap_or_default()
    } else {
        l.t(
            &format!(
                "Celda: {}/{} = {} % frente a {} % de la base {} ({} pp); IC95 % de la tasa de la celda {ci} (Wilson).",
                count(n), count(d), pct(rate), pct(base), if same_channel { "del mismo canal" } else { "de comparación" }, pp(diff)
            ),
            &format!(
                "Célula: {}/{} = {} % contra {} % da base {} ({} pp); IC95 % da taxa da célula {ci} (Wilson).",
                count(n), count(d), pct(rate), pct(base), if same_channel { "do mesmo canal" } else { "de comparação" }, pp(diff)
            ),
        )
        .to_string()
    };
    if let Some(h) = hold {
        s.push(' ');
        s.push_str(&h);
    }
    s.push_str(&r2);
    s.push_str(&p);
    s.push_str(&l.t(" Es una asociación descriptiva, no una causa.", " É uma associação descritiva, não uma causa."));
    s
}

fn diff_text(l: Lang, proposal: &Value) -> String {
    let entries = proposal["diff"].as_array().cloned().unwrap_or_default();
    if proposal["kind"].as_str() == Some("no_change") || entries.is_empty() {
        return l.t("Sin cambio propuesto.", "Sem mudanca proposta.").to_string();
    }
    let target = proposal["target_ref"].as_str().unwrap_or("?");
    let mut lines: Vec<String> = vec![];
    for e in entries.iter().take(4) {
        if let Some(anchor) = e["anchor_id"].as_str() {
            lines.push(format!(
                "[{}] {} {}: \"{}\" -> \"{}\"",
                e["locale"].as_str().unwrap_or("?"), e["op"].as_str().unwrap_or("?"), anchor, cap(e["anchor_text"].as_str().unwrap_or(""), 70), cap(e["replacement"].as_str().unwrap_or(""), 90)
            ));
        } else {
            lines.push(format!("{} {} v{}", e["kind"].as_str().unwrap_or("?"), e["id"].as_str().unwrap_or("?"), e["version"].as_str().unwrap_or("?")));
        }
    }
    if entries.len() > 4 {
        lines.push(l.t(&format!("(+{} más)", entries.len() - 4), &format!("(+{} mais)", entries.len() - 4)).to_string());
    }
    let head = if proposal["kind"].as_str() == Some("new_agent") {
        l.t(&format!("Nuevo especialista (copia de cierre del donante) en {target}: "), &format!("Novo especialista (cópia de fechamento do doador) em {target}: ")).to_string()
    } else {
        l.t(&format!("Parche anclado sobre {target} ({} caracteres editados de {}): ", proposal["edit_chars"], proposal["edit_budget"]),
            &format!("Patch ancorado em {target} ({} caracteres editados de {}): ", proposal["edit_chars"], proposal["edit_budget"])).to_string()
    };
    format!("{head}{}", lines.join(" | "))
}

struct Verdict {
    announce: bool,
    outcome: String,
    text: String,
}

fn verdict_text(l: Lang, verdict: Option<&Value>) -> Verdict {
    let Some(v) = verdict else {
        return Verdict {
            announce: false,
            outcome: "not_evaluated".into(),
            text: l.t("Sin veredicto de regresión: la propuesta NO fue evaluada contra una suite que falle en la base. No se anuncia.", "Sem veredito de regressão: a proposta NÃO foi avaliada contra uma suite que falhe na base. Não e anunciada.").to_string(),
        };
    };
    let outcome = v["outcome"].as_str().unwrap_or("unknown").to_string();
    let announce = v["announce"].as_bool() == Some(true) && outcome == "regression_suite_proven";
    let per = v["base"]["per_case"].as_object();
    let finding_cases = per.map(|m| m.keys().filter(|k| !k.starts_with("guard-")).count()).unwrap_or(0);
    let guard_cases = per.map(|m| m.keys().filter(|k| k.starts_with("guard-")).count()).unwrap_or(0);
    let base_failed = v["base"]["failed_cases"].as_array().map_or(0, Vec::len);
    let attempts = v["attempts"].as_array().cloned().unwrap_or_default();
    let steps: Vec<String> = attempts
        .iter()
        .enumerate()
        .map(|(i, a)| {
            let failed = a["failed_cases"].as_array().map_or(0, Vec::len);
            let pass = a["verdict"].as_str() == Some("pass");
            if a["verdict"].as_str() == Some("probe_only_pass") { l.t(&format!("intento {} pasó solo por la sonda del arnés", i + 1), &format!("tentativa {} passou só pela sonda do arnês", i + 1)).to_string() }
            else if pass { l.t(&format!("intento {} pasó", i + 1), &format!("tentativa {} passou", i + 1)).to_string() }
            else { l.t(&format!("intento {} falló ({} casos)", i + 1, failed), &format!("tentativa {} falhou ({} casos)", i + 1, failed)).to_string() }
        })
        .collect();
    let gi = v["gate_items"].as_array().cloned().unwrap_or_default();
    let gi_pass = gi.iter().filter(|g| g["passed"].as_bool() == Some(true)).count();
    let gates = if gi.is_empty() { String::new() } else { l.t(&format!(" GateItems: {gi_pass}/{} aprobados.", gi.len()), &format!(" GateItems: {gi_pass}/{} aprovados.", gi.len())).to_string() };
    let absent = v["base"]["verdict"].as_str() == Some("absent");
    let story = if absent {
        l.t(
            &format!("El agente nuevo no existe en la base: sus {finding_cases} casos del hallazgo fallan por ausencia (no medido, {guard_cases} guardas); {}.", if steps.is_empty() { "sin candidato evaluado".to_string() } else { steps.join("; ") }),
            &format!("O agente novo não existe na base: seus {finding_cases} casos do achado falham por ausencia (não medido, {guard_cases} guardas); {}.", if steps.is_empty() { "sem candidato avaliado".to_string() } else { steps.join("; ") }),
        )
    } else {
        l.t(
            &format!("La base falla {base_failed} de {finding_cases} casos del hallazgo ({guard_cases} guardas); {}.", if steps.is_empty() { "sin candidato evaluado".to_string() } else { steps.join("; ") }),
            &format!("A base falha {base_failed} de {finding_cases} casos do achado ({guard_cases} guardas); {}.", if steps.is_empty() { "sem candidato avaliado".to_string() } else { steps.join("; ") }),
        )
    };
    let verdict_line = match outcome.as_str() {
        "regression_suite_proven" => l.t("Suite de regresión probada: falla en la base y pasa con el candidato.", "Suite de regressão comprovada: falha na base e passa com o candidato."),
        "non_discriminating" => l.t("NO DISCRIMINANTE: la base ya pasa todos los casos; la suite no captura el problema y no es una suite de regresión. No se anuncia.", "NÃO DISCRIMINANTE: a base ja passa todos os casos; a suite não captura o problema e não é uma suite de regressão. Não e anunciada."),
        "not_fixed" => l.t("NO CORREGIDO: el candidato sigue fallando casos del hallazgo. No se anuncia.", "NÃO CORRIGIDO: o candidato continua falhando casos do achado. Não e anunciada."),
        "guard_regressed" => l.t("GUARDAS ROTAS: una guarda falla (en la base o en el candidato). No se anuncia.", "GUARDAS QUEBRADAS: uma guarda falha (na base ou no candidato). Não e anunciada."),
        "infra_failed" | "not_exercised" => l.t("NO EVALUADO: la infraestructura no produjo un veredicto. No se anuncia.", "NÃO AVALIADO: a infraestrutura não produziu um veredito. Não e anunciada."),
        "suite_refused" => l.t("SIN SUITE: no se pudo construir una suite de regresión para este hallazgo (evidencia insuficiente o sin mecanismo). No se anuncia.", "SEM SUITE: não foi possivel construir uma suite de regressão para este achado (evidência insuficiente ou sem mecanismo). Não e anunciada."),
        "suite_error" => l.t("NO EVALUADO: la construcción de la suite o el juez fallaron. No se anuncia.", "NÃO AVALIADO: a construção da suite ou o juiz falharam. Não e anunciada."),
        "base_only" => l.t("Solo base: no hay candidato evaluado. No se anuncia.", "Somente base: não ha candidato avaliado. Não e anunciada."),
        "native_not_candidate_bound" => l.t("SIN VÍNCULO NATIVO: agent-core no ejercitó el prompt candidato (una versión idéntica en texto falla lo que la base pasa); solo la sonda del arnés lo midió. No se anuncia.", "SEM VÍNCULO NATIVO: o agent-core não exercitou o prompt candidato (uma versão idêntica em texto falha o que a base passa); so a sonda do arnês o mediu. Não e anunciada."),
        _ => l.t("Veredicto desconocido: no se anuncia.", "Veredito desconhecido: não e anunciada."),
    };
    Verdict { announce, outcome, text: format!("{verdict_line} {story}{gates}") }
}

/// What was measured natively (agent-core scorer), by harness probe, and what was NOT measured, from the closed codes of the verdict
/// story (`coverage`, `native_binding`). Without a story the proposal was not evaluated at all.
fn coverage_text(l: Lang, verdict: Option<&Value>) -> String {
    let Some(v) = verdict else {
        return l.t("Nada medido: no hubo evaluación.", "Nada medido: não houve avaliação.").to_string();
    };
    if !v["coverage"].is_object() {
        return l.t("Cobertura no registrada en este veredicto (anterior a W13): véase el resultado.", "Cobertura não registrada neste veredito (anterior ao W13): veja o resultado.").to_string();
    }
    let cov = &v["coverage"];
    let name = |code: &str| -> String {
        match code {
            "flow_outcome_and_placeholder_render" => l.t("flujo resuelto y marcador renderizado en el motor real", "fluxo resolvido e marcador renderizado no motor real"),
            "flow_outcome" => l.t("flujo verificado y resuelto", "fluxo verificado e resolvido"),
            "response_from_model_path" => l.t("respuesta generada por el modelo (sin plantilla de respaldo)", "resposta gerada pelo modelo (sem template de reserva)"),
            "new_agent_intake_and_handoff" => l.t("el agente nuevo toma el tema, avisa y deriva a una persona sin herramientas", "o agente novo assume o tema, avisa e encaminha a uma pessoa sem ferramentas"),
            "platform_guardrails" => l.t("guardarraíles de plataforma", "guardrails de plataforma"),
            "guards" => l.t("casos guarda", "casos guarda"),
            "state_reflected" => l.t("el texto renderizado refleja el estado", "o texto renderizado reflete o estado"),
            "generated_followup" => l.t("3 muestras reales del modelo nombran el seguimiento y no llevan cifras", "3 amostras reais do modelo citam o acompanhamento e não trazem numeros"),
            "native_wording" => l.t("el texto (el evaluador nativo no lo lee)", "o texto (o avaliador nativo não o le)"),
            "real_customer_effect" => l.t("efecto en clientes reales", "efeito em clientes reais"),
            "candidate_prompt_native" => l.t("el prompt candidato en el evaluador nativo (sin vínculo al candidato)", "o prompt candidato no avaliador nativo (sem vínculo ao candidato)"),
            "base_by_absence" => l.t("la base (el agente no existe)", "a base (o agente não existe)"),
            "routing_recepcion_to_new_agent" => l.t("ruteo de recepción al agente nuevo (el arnés no ejercita directorio ni transferencia; entra al directorio solo con promote humano a prod)", "roteamento da recepção ao agente novo (o arnês não exercita diretório nem transferência; entra no diretório so com promote humano para prod)"),
            "traffic_stealing" => l.t("robo de tráfico a disputas y consultas", "roubo de tráfego de disputas e consultas"),
            "release_settings_assumed" => l.t("SUPUESTO: se evaluó con los ajustes de release del donante (interrupción fraude, injection-rules, lang-es-pt) que fija un admin humano; sin ellos agent-core no evalúa al agente nuevo", "PRESSUPOSTO: avaliou-se com os ajustes de release do doador (interrupção fraude, injection-rules, lang-es-pt) definidos por um admin humano; sem eles o agent-core não avalia o agente novo"),
            other => other.to_string(),
        }
    };
    let list = |key: &str| -> String { cov[key].as_array().into_iter().flatten().filter_map(Value::as_str).map(&name).collect::<Vec<_>>().join("; ") };
    let mut parts = vec![l.t(&format!("Nativo (agent-core): {}.", list("native")), &format!("Nativo (agent-core): {}.", list("native"))).to_string()];
    if cov["harness_probe"].as_array().is_some_and(|a| !a.is_empty()) {
        parts.push(l.t(&format!("Sonda del arnés: {}.", list("harness_probe")), &format!("Sonda do arnês: {}.", list("harness_probe"))).to_string());
    }
    parts.push(l.t(&format!("NO medido: {}.", list("not_measured")), &format!("NÃO medido: {}.", list("not_measured"))).to_string());
    if cov["assumptions"].as_array().is_some_and(|a| !a.is_empty()) {
        parts.push(list("assumptions") + ".");
    }
    parts.join(" ")
}

fn expected_effect(l: Lang, proposal: &Value, level: bool) -> (String, String) {
    let e = &proposal["expected_effect"];
    if !e.is_object() || e.is_null() {
        let none = l.t("Hipótesis no medida y sin estimacion de efecto; no se promete una reduccion.", "Hipótese não medida e sem estimativa de efeito; não se promete reducao.").to_string();
        let how = if level {
            l.t("Es un riesgo de nivel: se vigila el nivel frente al umbral en una ventana nueva; el estimador de resultado diría improved / no_detectable_change / worsened / inconclusive.", "É um risco de nivel: vigia-se o nivel contra o límite em uma janela nova; o estimador de resultado diría improved / no_detectable_change / worsened / inconclusive.")
        } else {
            l.t("Estimador de resultado en una ventana nueva: improved / no_detectable_change / worsened / inconclusive.", "Estimador de resultado em uma janela nova: improved / no_detectable_change / worsened / inconclusive.")
        };
        return (none, how.to_string());
    }
    let cur = e["current_cell_rate"].as_f64().map(pct).unwrap_or_else(|| "?".into());
    let rf = e["reference_rate"].as_f64().map(pct).unwrap_or_else(|| "?".into());
    let gap = e["min_detectable_gap"].as_f64().map(|g| pct(g)).unwrap_or_else(|| "?".into());
    let dir = e["direction"].as_str().unwrap_or("decrease");
    let effect = l.t(
        &format!("Hipótesis direccional ({dir}), no medida: llevar la tasa de la celda ({cur} %) hacia la referencia ({rf} %); mejora mínima buscada {gap} pp. No es una predicción de efecto."),
        &format!("Hipótese direcional ({dir}), não medida: levar a taxa da célula ({cur} %) para a referência ({rf} %); melhora mínima buscada {gap} pp. Não é uma previsão de efeito."),
    );
    let guard = e["guardrail"].as_str().map(|g| cap(g, 90)).unwrap_or_default();
    let how = l.t(
        &format!("Después del lanzamiento, sobre una ventana nueva con el mismo k-anonimato, el estimador de resultado clasifica: improved / no_detectable_change / worsened / inconclusive (improved = baja de al menos {gap} pp hacia la referencia). Guardrail: {guard}. Las pruebas de regresión sintéticas NO miden la tasa real."),
        &format!("Após o lançamento, em uma janela nova com o mesmo k-anonimato, o estimador de resultado classifica: improved / no_detectable_change / worsened / inconclusive (improved = queda de ao menos {gap} pp rumo à referência). Guardrail: {guard}. Os testes de regressão sintéticos NÃO medem a taxa real."),
    );
    (effect.to_string(), how.to_string())
}

fn risks(l: Lang, proposal: &Value, finding: &Value) -> String {
    let unc = proposal["uncertainty"].as_str().map(|u| cap(u, 160)).unwrap_or_default();
    let cascade = proposal["cascade"].as_array().map_or(0, Vec::len);
    let humans = proposal["human_items"].as_array().map_or(0, Vec::len);
    let mut s = String::new();
    if !unc.is_empty() {
        s.push_str(&unc);
        s.push(' ');
    }
    if cascade > 0 {
        s.push_str(&l.t(&format!("Cascada: {cascade} entidad(es) pasan a la siguiente versión parche. "), &format!("Cascata: {cascade} entidade(s) passam para a próxima versão patch. ")));
    }
    if humans > 0 {
        s.push_str(&l.t(&format!("{humans} item(s) de release solo los fija un admin humano. "), &format!("{humans} item(ns) de release so um admin humano define. ")));
    }
    if finding["status"].as_str().is_some_and(|st| st != "corroborated") {
        s.push_str(&l.t("El hallazgo no está corroborado. ", "O achado não está corroborado. "));
    }
    s.push_str(&l.t("Reversión: volver a la release base; solo staging.", "Reversão: voltar a release base; somente staging."));
    s
}

fn unchanged(l: Lang, proposal: &Value) -> String {
    match proposal["kind"].as_str() {
        Some("patch") => l.t(
            "Cláusulas protegidas del texto base intactas (el menú de anclas las excluye y el compilador verifica los marcadores); ningún presupuesto, modelo ni permiso cambia; solo se tocan los textos nombrados.",
            "Cláusulas protegidas do texto base intactas (o menú de âncoras as exclui e o compilador verifica os marcadores); nenhum orçamento, modelo ou permissão muda; so os textos nomeados sao tocados.",
        ),
        Some("new_agent") => l.t(
            "Recepción no cambia (solo ruta a disputas y consultas): llega al agente nuevo tras ajustes de release de un admin y promote humano a prod (seguimiento humano). Sin herramientas ni permisos nuevos.",
            "A recepção não muda (so roteia a disputas e consultas): chega ao agente novo apos ajustes de release de um admin e promote humano para prod (acompanhamento humano). Sem ferramentas nem permissoes novas.",
        ),
        _ => l.t("Nada cambia: no hay cambio propuesto.", "Nada muda: não ha mudanca proposta."),
    }
    .to_string()
}

fn judge_family(verdict: Option<&Value>, labels: &Labels) -> Option<String> {
    if let Some(j) = &labels.judge_family {
        return Some(j.clone());
    }
    let mp = verdict?.get("model_policy")?;
    let family = mp["judge"].as_str()?.split('/').next()?.to_string();
    Some(if mp["judge_rule"].as_str().is_some_and(|r| r.contains("deterministic checks only")) { format!("{family} (not used here: deterministic checks only)") } else { family })
}

fn honesty(l: Lang, runtime: Runtime, rubric: Option<(u32, u32)>, judge: &Option<String>, calibrated: bool, data: &str) -> String {
    let run = match runtime {
        Runtime::Doubles => l.t("dobles (ninguna llamada real a modelo)", "dobles (nenhuma chamada real a modelo)"),
        Runtime::Real => l.t("real (stack local y gateway)", "real (stack local e gateway)"),
        Runtime::LocalModel => l.t("modelo local", "modelo local"),
    };
    let rub = rubric.map(|(t, m)| l.t(&format!("{t}/{m} (autopuntaje estructural, no es el juez)"), &format!("{t}/{m} (autopontuação estrutural, não e o juiz)")).to_string()).unwrap_or_else(|| l.t("no calculado", "não calculado").to_string());
    let j = judge.clone().unwrap_or_else(|| l.t("sin juez", "sem juiz").to_string());
    let cal = if calibrated { "calibrated" } else { "uncalibrated" };
    let data = match data {
        "synthetic" => l.t(" Datos: sintéticos (conteos inventados).", " Dados: sintéticos (contagens inventadas)."),
        "bank_treated" | "bank" => l.t(" Datos: agregados tratados del banco.", " Dados: agregados tratados do banco."),
        "e0_treated" | "e0" => l.t(" Datos: agregados tratados de E0.", " Dados: agregados tratados de E0."),
        _ => String::new(),
    };
    let base = l.t(
        &format!("Ejecución: {run}. Rúbrica: {rub}. Familia del juez: {j}. Calibración: {cal}. Sensor: claude-standin. Reclamo: asociación, no causa."),
        &format!("Execução: {run}. Rúbrica: {rub}. Família do juiz: {j}. Calibração: {cal}. Sensor: claude-standin. Alegação: associação, não causa."),
    )
    .to_string();
    base + &data
}

// ---------------------------------------------------------------------------------------------------------------- build

/// Assembles the dossier. `finding` is one signal of a `steps_cli cells` report (contrast or `level_risk`); `proposal` is the
/// compiled proposal JSON (`Compiled::to_json`) or the whole reasoning record that carries it under `proposal`; `verdict` is the
/// REG1 `reg1.verdict_story/1` JSON (absent: the proposal was not evaluated and is not announced).
pub fn build(finding: &Value, proposal: &Value, verdict: Option<&Value>, labels: &Labels) -> Result<Value, String> {
    let record = proposal;
    let proposal = if record["proposal"].is_object() { &record["proposal"] } else { record };
    if !proposal["kind"].is_string() {
        return Err("the proposal has no kind (expected a compiled proposal)".into());
    }
    if !finding["discovery"].is_object() {
        return Err("the finding has no discovery stage".into());
    }
    let level = finding["type"].as_str() == Some("level_risk");
    let finding_ok = finding["status"].as_str().is_none_or(|s| s == "corroborated");
    let has_change = proposal["kind"].as_str() != Some("no_change");
    let doubles = record["doubles"].as_array().is_some_and(|d| !d.is_empty());
    let runtime = if doubles { Runtime::Doubles } else { labels.runtime };
    let rubric = labels.rubric.or_else(|| Some((record["rubric"]["total"].as_u64()? as u32, record["rubric"]["max"].as_u64()? as u32)));
    let judge = judge_family(verdict, labels);

    let vd_es = verdict_text(Lang::Es, verdict);
    let announce = vd_es.announce && finding_ok && has_change;
    let reason_key = if announce {
        "announce"
    } else if verdict.is_none() {
        "not_evaluated"
    } else if !vd_es.announce {
        vd_es.outcome.as_str()
    } else if !finding_ok {
        "finding_not_corroborated"
    } else {
        "no_change"
    };

    let mut out = Map::new();
    for l in [Lang::Es, Lang::Pt] {
        let vd = verdict_text(l, verdict);
        let (effect, how) = expected_effect(l, proposal, level);
        let decision = if announce {
            l.t("DECISIÓN: proponer a supervisión (suite de regresión probada).", "DECISÃO: propor a supervisão (suite de regressão comprovada).")
        } else {
            l.t("DECISIÓN: NO SE ANUNCIA; queda como borrador interno.", "DECISÃO: NÃO E ANUNCIADA; fica como rascunho interno.")
        };
        let sec = json!({
            "problem": problem(l, finding, level),
            "evidence": evidence(l, finding, level),
            "diff": diff_text(l, proposal),
            "result": vd.text,
            "expected_effect": effect,
            "measurement": how,
            "risks": risks(l, proposal, finding),
            "unchanged": unchanged(l, proposal),
            "coverage": coverage_text(l, verdict),
            "honesty": honesty(l, runtime, rubric, &judge, labels.calibrated, finding["source"].as_str().unwrap_or("")),
        });
        let s = |k: &str| sec[k].as_str().unwrap_or("").to_string();
        let labels_l = [
            ("problem", l.t("Problema observado", "Problema observado"), 220),
            ("evidence", l.t("Evidencia y comparación", "Evidência e comparação"), 440),
            ("diff", l.t("Qué cambiaría", "O que mudaria"), 400),
            ("result", l.t("Resultado base vs candidato", "Resultado base vs candidato"), 400),
            ("coverage", l.t("Qué se midió", "O que foi medido"), 560),
            ("expected_effect", l.t("Efecto esperado", "Efeito esperado"), 240),
            ("measurement", l.t("Cómo se evaluará", "Como será avaliado"), 260),
            ("risks", l.t("Riesgo", "Risco"), 300),
            ("unchanged", l.t("Qué no cambia", "O que não muda"), 200),
            ("honesty", l.t("Etiquetas", "Rótulos"), 300),
        ];
        let mut description = decision.to_string();
        for (k, label, max) in labels_l {
            description.push_str(&format!("\n\n**{label}.** {}", cap(&s(k), max)));
        }
        if description.chars().count() > DESCRIPTION_MAX {
            return Err(format!("the description is longer than {DESCRIPTION_MAX} characters"));
        }
        let rationale = cap(&format!("{} {}", proposal["rationale"].as_str().unwrap_or(""), l.t("Hipótesis de intervención sobre una asociación; debe superar la evaluación antes de cualquier aprobación.", "Hipótese de intervenção sobre uma associação; deve passar na avaliação antes de qualquer aprovação.")).trim().to_string(), RATIONALE_MAX);
        let changelog = cap(&l.t(&format!("Solo propuesta, no aprobada ni publicada. {}", cap(&s("diff"), 600)), &format!("Somente proposta, não aprovada nem publicada. {}", cap(&s("diff"), 600))), CHANGELOG_MAX);
        let metric = finding["metric"].as_str().unwrap_or("?");
        let title = cap(&format!("{} - {}{}: {}", proposal["target_ref"].as_str().unwrap_or("?"), metric, if level { l.t(" (nivel)", " (nível)") } else { String::new() }, l.t("propuesta de cambio", "proposta de mudança")), TITLE_MAX);
        out.insert(l.key().to_string(), json!({"title": title, "description": description, "rationale": rationale, "changelog": changelog, "sections": sec}));
    }

    let mut strings = vec![];
    collect(&Value::Object(out.clone()), &mut strings);
    for s in &strings {
        if let Some(why) = pii_shaped(s) {
            return Err(format!("dossier refused: a text contains {why}"));
        }
    }
    let es = out.remove("es").unwrap_or(Value::Null);
    let pt = out.remove("pt").unwrap_or(Value::Null);
    Ok(json!({
        "schema": SCHEMA,
        "announce": announce,
        "announce_reason": reason_key,
        "outcome": vd_es.outcome,
        "finding_kind": if level { "level_risk" } else { "contrast" },
        "honesty": {"runtime": runtime.as_str(), "rubric": rubric.map(|(t, m)| json!({"total": t, "max": m, "scope": "structural self-score, not the judge"})),
                    "judge_family": judge, "calibration": if labels.calibrated { "calibrated" } else { "uncalibrated" },
                    "claim": "association", "sensor": "claude-standin", "effect": "not_measured"},
        "refs": {"finding_id": finding["finding_id"], "evidence_ref": proposal["expected_effect"]["evidence_ref"], "target_ref": proposal["target_ref"], "base_digest": proposal["base_digest"]},
        "limits": {"description_max": DESCRIPTION_MAX, "rationale_max": RATIONALE_MAX, "changelog_max": CHANGELOG_MAX, "title_max": TITLE_MAX},
        "es": es,
        "pt": pt,
    }))
}

/// JSON Schema of the dossier object (the contract the registry-writer integration consumes).
pub fn schema() -> Value {
    let lang = json!({"type": "object", "required": ["title", "description", "rationale", "changelog", "sections"], "additionalProperties": false,
        "properties": {"title": {"type": "string", "maxLength": TITLE_MAX}, "description": {"type": "string", "maxLength": DESCRIPTION_MAX},
                       "rationale": {"type": "string", "maxLength": RATIONALE_MAX}, "changelog": {"type": "string", "maxLength": CHANGELOG_MAX},
                       "sections": {"type": "object", "required": ["problem", "evidence", "diff", "result", "coverage", "expected_effect", "measurement", "risks", "unchanged", "honesty"],
                                    "additionalProperties": {"type": "string"}}}});
    json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "$id": "dossier/1", "type": "object",
        "required": ["schema", "announce", "announce_reason", "outcome", "finding_kind", "honesty", "refs", "limits", "es", "pt"],
        "properties": {"schema": {"const": SCHEMA}, "announce": {"type": "boolean"}, "announce_reason": {"type": "string"}, "outcome": {"type": "string"},
                       "finding_kind": {"enum": ["contrast", "level_risk"]},
                       "honesty": {"type": "object", "required": ["runtime", "calibration", "claim", "sensor", "effect"],
                                   "properties": {"runtime": {"enum": ["doubles", "real", "local-model"]}, "calibration": {"enum": ["uncalibrated", "calibrated"]}, "claim": {"const": "association"}, "effect": {"const": "not_measured"}}},
                       "es": lang, "pt": lang}})
}

/// Plain-text rendering of one language (used by the CLI and `docs/dev/DOSSIER_EXAMPLES.md`).
pub fn render_markdown(d: &Value, l: Lang) -> String {
    let x = &d[l.key()];
    format!(
        "### {}\n\n{}\n\n- rationale: {}\n- changelog: {}\n- announce: `{}` ({})\n",
        x["title"].as_str().unwrap_or(""), x["description"].as_str().unwrap_or(""), x["rationale"].as_str().unwrap_or(""), x["changelog"].as_str().unwrap_or(""), d["announce"], d["announce_reason"].as_str().unwrap_or("")
    )
}
