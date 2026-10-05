//! Decision dossier (W1-2): golden dossiers from audited aggregates, determinism, ES/PT variants, length bound, PII refusal and
//! honesty labels that are never overstated.
use reasoning::dossier::{DESCRIPTION_MAX, Labels, Runtime, build, schema};
use serde_json::{Value, json};

fn story(name: &str) -> Value {
    let p = format!("{}/../../../scripts/regression/results/{name}.json", env!("CARGO_MANIFEST_DIR"));
    serde_json::from_str(&std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{p}: {e}"))).unwrap()
}

/// Queja x Phone: the audited rates (56.1 % against the 16.6 % same-channel baseline); counts are fixture-scale.
fn queja_phone() -> Value {
    json!({"metric": "M4", "dims": {"category": "Queja", "channel": "Phone", "locale": "es"}, "status": "corroborated", "reason": "replicated_in_holdout",
           "direction": "up", "claim": "association", "finding_id": "finding_1",
           "discovery": {"numerator": 56100, "denominator": 100000, "rate": 0.561, "baseline_rate": 0.166, "diff": 0.395, "p": 0.0},
           "holdout": {"numerator": 5520, "denominator": 10000, "rate": 0.552, "baseline_rate": 0.166, "diff": 0.386, "p": 0.0},
           "r2": {"status": "replicated"}, "p_adj": 0.00001})
}

/// M8 level risk: 874,417 of 1,746,801 sends (50.06 %) go to non-consenting customers; the pre-registered threshold is 10 %.
fn m8_level() -> Value {
    json!({"metric": "M8", "type": "level_risk", "class": "risk", "dims": {}, "status": "corroborated", "reason": "replicated_in_holdout", "direction": "up", "claim": "association",
           "discovery": {"numerator": 874417, "denominator": 1746801, "rate": 0.50058, "baseline_rate": 0.10, "diff": 0.40058, "ci95_low": 0.4994, "ci95_high": 0.5017, "p": 0.0},
           "holdout": {"numerator": 437000, "denominator": 873000, "rate": 0.5006, "baseline_rate": 0.10, "diff": 0.4006, "ci95_low": 0.499, "ci95_high": 0.502, "p": 0.0},
           "r2": {"status": "replicated"}, "periods": {"total": 4, "above_threshold": 4}, "level_cells": {"total": 6, "above_threshold": 6}, "p_adj": 0.0})
}

fn patch_proposal() -> Value {
    json!({"kind": "patch", "target_ref": "template:t/estado_pqr", "base_digest": "ffbb5234aa00bb11",
           "diff": [{"locale": "es", "anchor_id": "a1", "op": "replace", "anchor_text": "Ya consulte tu PQR.", "replacement": "Tu PQR esta {{ estado }}."},
                    {"locale": "pt", "anchor_id": "a1", "op": "replace", "anchor_text": "Ja consultei sua solicitacao.", "replacement": "Sua solicitacao esta {{ estado }}."}],
           "cascade": ["flow:consultas"], "edit_chars": 74, "edit_budget": 240, "human_items": [],
           "expected_effect": {"metric_id": "m4", "direction": "decrease", "current_cell_rate": 0.56, "reference_rate": 0.17, "min_detectable_gap": 0.2,
                               "success_if": "the cell rate falls", "guardrail": "no more escalations to a human than the base", "link_grade": "plausible", "evidence_ref": "ev_0123456789abcdef"},
           "rationale": "The status message never says the state of the request.", "uncertainty": "The data says where the problem is, not why."})
}

fn no_change() -> Value {
    json!({"kind": "no_change", "target_ref": "template:t/estado_pqr", "diff": [], "changes": [], "rationale": "No safe change.", "uncertainty": "Unclear."})
}

fn labels() -> Labels {
    Labels { runtime: Runtime::Real, ..Labels::default() }
}

fn es(d: &Value) -> &str {
    d["es"]["description"].as_str().unwrap()
}

#[test]
fn golden_queja_phone_dossier_is_understandable_and_announced_when_the_regression_suite_is_proven() {
    let d = build(&queja_phone(), &patch_proposal(), Some(&story("story_template_estado_pqr")), &labels()).unwrap();
    assert_eq!(d["schema"], "dossier/1");
    assert_eq!(d["announce"], true);
    let t = es(&d);
    for needle in ["56,1 %", "16,6 %", "del mismo canal", "+39,5 pp", "56.100/100.000", "IC95 %", "Replicación (holdout)", "Ventanas R2: replicated", "no una causa", "Qué cambiaría", "Qué no cambia", "Riesgo", "Cómo se evaluará",
                   "Tu PQR esta {{ estado }}", "improved / no_detectable_change / worsened / inconclusive", "falla 8 de 8 casos", "intento 1", "GateItems", "uncalibrated", "no medida", "Reversión"] {
        assert!(t.contains(needle), "missing {needle:?} in:\n{t}");
    }
    assert!(t.starts_with("DECISIÓN: proponer a supervisión"), "{t}");
    assert_eq!(d["honesty"]["calibration"], "uncalibrated");
    assert_eq!(d["honesty"]["effect"], "not_measured");
    assert_eq!(d["refs"]["evidence_ref"], "ev_0123456789abcdef");
    assert!(d["es"]["title"].as_str().unwrap().contains("template:t/estado_pqr"));
    assert!(d["es"]["changelog"].as_str().unwrap().starts_with("Solo propuesta, no aprobada ni publicada"));
}

#[test]
fn golden_m8_level_risk_dossier_states_level_and_threshold_not_a_contrast() {
    let d = build(&m8_level(), &no_change(), None, &labels()).unwrap();
    let t = es(&d);
    assert_eq!(d["finding_kind"], "level_risk");
    for needle in ["874.417/1.746.801", "50,1 %", "umbral 10,0 %", "IC95 %", "Períodos sobre el umbral: 4/4", "no una causa", "nivel de riesgo"] {
        assert!(t.contains(needle), "missing {needle:?} in:\n{t}");
    }
    assert!(!t.contains("Wilson"), "a level is not a vs-rest contrast");
    assert_eq!(d["announce"], false, "no verdict, no change: not announced");
    assert!(t.starts_with("DECISIÓN: NO SE ANUNCIA"));
}

#[test]
fn portuguese_variant_is_complete_and_differs_from_spanish() {
    let d = build(&queja_phone(), &patch_proposal(), Some(&story("story_template_estado_pqr")), &labels()).unwrap();
    let (e, p) = (es(&d), d["pt"]["description"].as_str().unwrap());
    assert_ne!(e, p);
    for needle in ["DECISÃO", "Problema observado", "Evidência e comparação", "O que mudaria", "Efeito esperado", "Como será avaliado", "Risco", "O que não muda", "da base do mesmo canal", "Rótulos", "uncalibrated", "não medida"] {
        assert!(p.contains(needle), "missing {needle:?} in:\n{p}");
    }
    for k in ["title", "rationale", "changelog"] {
        assert_ne!(d["es"][k], d["pt"][k], "{k}");
    }
    assert_eq!(d["es"]["sections"].as_object().unwrap().len(), d["pt"]["sections"].as_object().unwrap().len());
}

#[test]
fn output_is_deterministic_and_matches_the_json_schema_required_keys() {
    let a = build(&queja_phone(), &patch_proposal(), Some(&story("story_template_estado_pqr")), &labels()).unwrap();
    let b = build(&queja_phone(), &patch_proposal(), Some(&story("story_template_estado_pqr")), &labels()).unwrap();
    assert_eq!(serde_json::to_string(&a).unwrap(), serde_json::to_string(&b).unwrap());
    let s = schema();
    for k in s["required"].as_array().unwrap() {
        assert!(a.get(k.as_str().unwrap()).is_some(), "{k}");
    }
    for l in ["es", "pt"] {
        for k in s["properties"][l]["required"].as_array().unwrap() {
            assert!(a[l].get(k.as_str().unwrap()).is_some(), "{l}.{k}");
        }
        assert!(a[l]["description"].as_str().unwrap().chars().count() <= s["properties"][l]["properties"]["description"]["maxLength"].as_u64().unwrap() as usize);
    }
}

#[test]
fn length_is_bounded_even_for_a_huge_diff_and_long_texts() {
    let mut p = patch_proposal();
    let long = "texto largo ".repeat(200);
    p["diff"] = Value::Array((0..30).map(|_| json!({"locale": "es", "anchor_id": "a1", "op": "replace", "anchor_text": long, "replacement": long})).collect());
    p["rationale"] = json!(long);
    p["uncertainty"] = json!(long);
    let d = build(&queja_phone(), &p, Some(&story("story_template_estado_pqr")), &labels()).unwrap();
    for l in ["es", "pt"] {
        assert!(d[l]["description"].as_str().unwrap().chars().count() <= DESCRIPTION_MAX);
        assert!(d[l]["rationale"].as_str().unwrap().chars().count() <= 1500);
        assert!(d[l]["changelog"].as_str().unwrap().chars().count() <= 1500);
        assert!(d[l]["title"].as_str().unwrap().chars().count() <= 200);
    }
    assert!(d["es"]["sections"]["diff"].as_str().unwrap().contains("(+26 más)"));
}

#[test]
fn pii_shaped_text_is_refused_not_scrubbed() {
    for bad in ["escribe a ana@example.com", "llama al 3001234567", "token \u{27e6}PII_1\u{27e7}"] {
        let mut p = patch_proposal();
        p["diff"][0]["replacement"] = json!(bad);
        let e = build(&queja_phone(), &p, None, &labels()).unwrap_err();
        assert!(e.contains("refused"), "{bad}: {e}");
    }
    // an `id@version` reference and a 5-digit run are not PII shapes
    let mut ok = patch_proposal();
    ok["diff"][0]["replacement"] = json!("usa obtener_pqr@1 con codigo 12345");
    assert!(build(&queja_phone(), &ok, None, &labels()).is_ok());
}

#[test]
fn a_non_discriminating_verdict_is_never_announced_and_the_dossier_says_so() {
    let d = build(&queja_phone(), &patch_proposal(), Some(&story("story_prompt_native_only_non_discriminating")), &labels()).unwrap();
    assert_eq!((d["announce"].clone(), d["announce_reason"].as_str().unwrap(), d["outcome"].as_str().unwrap()), (json!(false), "non_discriminating", "non_discriminating"));
    assert!(es(&d).starts_with("DECISIÓN: NO SE ANUNCIA"));
    assert!(es(&d).contains("NO DISCRIMINANTE"));
    assert!(d["pt"]["description"].as_str().unwrap().contains("NÃO DISCRIMINANTE"));
    assert!(!es(&d).contains("probada"));
}

#[test]
fn a_not_fixed_verdict_is_never_announced_and_the_dossier_says_so() {
    for name in ["story_template_estado_pqr_negative_noop", "story_prompt_resumen_radicado_negative_noop"] {
        let d = build(&queja_phone(), &patch_proposal(), Some(&story(name)), &labels()).unwrap();
        assert_eq!(d["announce"], false, "{name}");
        assert_eq!(d["announce_reason"], "not_fixed");
        assert!(es(&d).contains("NO CORREGIDO") && d["pt"]["description"].as_str().unwrap().contains("NÃO CORRIGIDO"));
    }
}

#[test]
fn announce_needs_a_proven_verdict_a_corroborated_finding_and_a_real_change() {
    let proven = story("story_prompt_resumen_radicado");
    assert_eq!(build(&queja_phone(), &patch_proposal(), Some(&proven), &labels()).unwrap()["announce"], true);
    assert_eq!(build(&queja_phone(), &no_change(), Some(&proven), &labels()).unwrap()["announce"], false);
    let mut refuted = queja_phone();
    refuted["status"] = json!("refuted");
    assert_eq!(build(&refuted, &patch_proposal(), Some(&proven), &labels()).unwrap()["announce"], false);
    // a tampered story whose flag says announce but whose outcome is not proven stays unannounced
    let mut forged = story("story_template_estado_pqr_negative_noop");
    forged["announce"] = json!(true);
    assert_eq!(build(&queja_phone(), &patch_proposal(), Some(&forged), &labels()).unwrap()["announce"], false);
    // the story of the two attempts is told
    let t = build(&queja_phone(), &patch_proposal(), Some(&proven), &labels()).unwrap();
    assert!(es(&t).contains("intento 1 falló (6 casos); intento 2 pasó"), "{}", es(&t));
}

#[test]
fn honesty_labels_default_to_the_conservative_ones_and_doubles_cannot_be_overridden() {
    let d = build(&queja_phone(), &patch_proposal(), Some(&story("story_template_estado_pqr")), &Labels::default()).unwrap();
    assert_eq!(d["honesty"]["runtime"], "doubles");
    assert!(es(&d).contains("dobles"));
    let rec = json!({"proposal": patch_proposal(), "doubles": [{"label": "scripted"}], "rubric": {"total": 20, "max": 24}});
    let d = build(&queja_phone(), &rec, Some(&story("story_template_estado_pqr")), &labels()).unwrap();
    assert_eq!(d["honesty"]["runtime"], "doubles", "a record with doubles is never labelled real");
    assert_eq!(d["honesty"]["rubric"]["total"], 20);
    assert!(es(&d).contains("20/24 (autopuntaje estructural, no es el juez)"));
    assert!(es(&d).contains("Familia del juez: z-ai (not used here: deterministic checks only)"));
    let cal = build(&queja_phone(), &patch_proposal(), None, &Labels { calibrated: true, ..labels() }).unwrap();
    assert_eq!(cal["honesty"]["calibration"], "calibrated");
}

#[test]
fn the_expected_effect_is_never_stated_as_a_prediction_or_a_cause() {
    let d = build(&queja_phone(), &patch_proposal(), Some(&story("story_template_estado_pqr")), &labels()).unwrap();
    let all = format!("{}{}", es(&d), d["pt"]["description"]);
    for forbidden in ["mejorará", "reducirá", "causa de", "melhorará", "reduzirá", "garantiza"] {
        assert!(!all.contains(forbidden), "{forbidden}");
    }
    assert!(es(&d).contains("No es una predicción de efecto"));
}

const PLATFORM_TITLE_MAX: usize = 120;

fn long_target(p: &mut Value) {
    p["target_ref"] = json!("template:t/estado_pqr_con_un_nombre_extraordinariamente_largo_para_la_plantilla_de_estado_de_solicitudes_del_cliente");
}

#[test]
fn a_long_title_is_cut_at_a_word_boundary_to_the_platform_cap_and_the_full_text_stays_in_the_description() {
    let mut finding = queja_phone();
    finding["metric"] = json!("M4 tasa de escalamiento a humano en consultas de estado de solicitud con demora prolongada");
    let mut p = patch_proposal();
    long_target(&mut p);
    let d = build(&finding, &p, Some(&story("story_template_estado_pqr")), &labels()).unwrap();
    for l in ["es", "pt"] {
        let title = d[l]["title"].as_str().unwrap();
        assert!(title.chars().count() <= PLATFORM_TITLE_MAX, "{title}");
        assert!(title.ends_with('\u{2026}'), "a cut title says so: {title}");
        let stem = title.trim_end_matches('\u{2026}');
        assert!(!stem.ends_with(' ') && !stem.ends_with('-') && !stem.ends_with(':'), "{title}");
        let full = d[l]["title_full"].as_str().unwrap();
        assert!(full.chars().count() > PLATFORM_TITLE_MAX && full.starts_with(stem), "{full}");
        assert!(full[stem.len()..].starts_with(' '), "not a word boundary: {title:?} / {full:?}");
        let desc = d[l]["description"].as_str().unwrap();
        assert!(desc.lines().next().unwrap().starts_with("DECIS"), "the first line is kept: {desc}");
        assert!(desc.contains(full), "the full title stays in the description");
    }
    assert!(d["es"]["title_full"].as_str().unwrap().contains("propuesta"));
    assert!(d["pt"]["title_full"].as_str().unwrap().contains("proposta"));
}

#[test]
fn a_short_title_is_untouched_and_has_no_full_title() {
    let d = build(&queja_phone(), &patch_proposal(), Some(&story("story_template_estado_pqr")), &labels()).unwrap();
    for l in ["es", "pt"] {
        assert!(d[l]["title"].as_str().unwrap().chars().count() <= PLATFORM_TITLE_MAX);
        assert!(!d[l]["title"].as_str().unwrap().ends_with('\u{2026}'));
        assert!(d[l]["title_full"].is_null());
    }
}

#[test]
fn a_single_huge_word_title_is_still_capped() {
    let mut p = patch_proposal();
    p["target_ref"] = json!(format!("template:t/{}", "x".repeat(300)));
    let d = build(&queja_phone(), &p, None, &labels()).unwrap();
    assert!(d["es"]["title"].as_str().unwrap().chars().count() <= PLATFORM_TITLE_MAX);
}

#[test]
fn description_reads_as_plain_labelled_lines_without_markdown_for_the_spa() {
    let d = build(&queja_phone(), &patch_proposal(), Some(&story("story_template_estado_pqr")), &labels()).unwrap();
    for l in ["es", "pt"] {
        for k in ["description", "rationale", "changelog", "title"] {
            let t = d[l][k].as_str().unwrap();
            for md in ["**", "__", "##", "```", "\n|", "| --"] {
                assert!(!t.contains(md), "{l}.{k} has markdown {md:?}:\n{t}");
            }
        }
        let desc = d[l]["description"].as_str().unwrap();
        let lines: Vec<&str> = desc.lines().filter(|x| !x.is_empty()).collect();
        assert!(lines.len() >= 11, "decision line plus one labelled line per section: {}", lines.len());
        for line in &lines[1..] {
            let (label, rest) = line.split_once(": ").unwrap_or_else(|| panic!("not a labelled line: {line}"));
            assert!(label.chars().count() <= 30 && !rest.is_empty(), "{line}");
        }
    }
    assert!(es(&d).contains("\nProblema observado: "));
}

#[test]
fn the_end_step_is_pasar_a_produccion_not_activar() {
    let d = build(&queja_phone(), &patch_proposal(), Some(&story("story_template_estado_pqr")), &labels()).unwrap();
    assert!(es(&d).contains("Siguiente paso humano: Pasar a producción"), "{}", es(&d));
    assert!(!es(&d).contains("Activar"));
    assert!(d["pt"]["description"].as_str().unwrap().contains("Passar para produção"));
    assert_eq!(d["end_step"], "Pasar a producción");
    let na = build(&queja_phone(), &patch_proposal(), None, &labels()).unwrap();
    assert!(!es(&na).contains("Pasar a producción"), "an unannounced draft proposes no step");
}
