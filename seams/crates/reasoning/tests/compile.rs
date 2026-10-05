//! Anchor menu, mapping and the byte-exact compiler over the REAL baseline artifacts (agent-core registry-e2e seed).
mod common;
use common::*;
use reasoning::catalog::anchors;
use reasoning::finding::Source;
use reasoning::mapping::map_finding;
use reasoning::patch::{Denied, compile};
use reasoning::roles::{Alt, Opportunity};
use serde_json::{Value, json};

fn opp(target: &str, mech: &str) -> Opportunity {
    Opportunity {
        id: "h_1".into(), target_ref: target.into(), mechanism_class: mech.into(), hypothesis: "h".into(), claimed_rate: 0.9, falsifiers: vec!["f".into()],
        alternatives: vec![Alt { kind: "do_nothing".into(), why_not: "x".into() }, Alt { kind: "human_owned".into(), why_not: "y".into() }],
    }
}

fn patch_proposal(target: &str, patches: Value) -> Value {
    json!({"kind": "patch", "target_ref": target, "rationale": "Say what was consulted.", "patches": patches, "expected_direction": "decrease", "alternatives": alts(), "uncertainty": "Association only."})
}

fn denied(r: Result<reasoning::patch::Compiled, Denied>) -> String {
    r.expect_err("must be denied").code.to_string()
}

#[test]
fn mapping_sends_findings_to_real_artifacts_and_leaves_the_rest_unlinked() {
    let t = map_finding(&tecnico_finding()).unwrap();
    assert_eq!((t.id, t.link_grade), ("uncovered_reason", "mechanism_proxy"));
    assert_eq!(t.targets[0].target_ref, "new_agent:consultas");
    assert!(t.targets[0].slugs.contains(&"soporte-tecnico"));
    let p = map_finding(&pqr_finding()).unwrap();
    assert_eq!((p.targets[0].target_ref.as_str(), p.link_grade), ("template:t/estado_pqr", "unlinked"));
    let c = map_finding(&copilot_finding(Source::E0Treated)).unwrap();
    assert_eq!(c.targets[0].target_ref, "prompt:p/copiloto");
    // a dependent survey metric and a non-problem reason have no target: descriptive, no proposal
    let m6 = finding_of(signal("M6", json!({"channel": "Phone"}), stage(300, 1000, 0.1), stage(200, 700, 0.1)), Source::Synthetic);
    assert!(map_finding(&m6).is_none());
    let prod = finding_of(signal("M1", json!({"reason_category": "Producto", "channel": "Phone"}), stage(300, 1000, 0.1), stage(200, 700, 0.1)), Source::Synthetic);
    assert!(map_finding(&prod).is_none());
}

#[test]
fn the_anchor_menu_never_offers_a_protected_clause_of_the_real_copilot_prompt() {
    let c = cat();
    let art = c.get("prompt:p/copiloto").unwrap();
    for (locale, tool_choice) in [("es", "Elige la tool seg"), ("pt", "Escolha a tool")] {
        let menu = anchors(art, locale);
        assert!(menu.iter().any(|a| a.text.contains(tool_choice)), "{locale}: the tool-choice sentence is patchable");
        let base = &art.locales[locale];
        for a in &menu {
            assert_eq!(base.matches(a.text.as_str()).count(), 1, "{}: anchors are unique substrings", a.id);
            for m in ["datos_no_confiables", "Solo lees", "otro cliente", "Nunca inventes", "kind \"final\"", "somente o cliente atendido", "Nunca invente"] {
                assert!(!a.text.contains(m), "{} offers the protected marker {m}", a.id);
            }
        }
        let ids: std::collections::BTreeSet<_> = menu.iter().map(|a| &a.id).collect();
        assert_eq!(ids.len(), menu.len());
    }
}

#[test]
fn a_template_patch_is_applied_byte_exact_with_the_predicted_cascade() {
    let c = cat();
    let f = pqr_finding();
    let row = map_finding(&f).unwrap();
    let art = c.get("template:t/estado_pqr").unwrap();
    assert_eq!(art.locales["es"], "Ya consult\u{e9} tu PQR. Si necesitas algo m\u{e1}s, d\u{ed}melo.");
    let patch = json!([
        {"locale": "es", "anchor_id": "es.a1", "op": "replace", "replacement": "Ya consult\u{e9} tu PQR y su estado actual es {{ facts.pqr.value.status }}."},
        {"locale": "pt", "anchor_id": "pt.a1", "op": "replace", "replacement": "J\u{e1} consultei sua solicita\u{e7}\u{e3}o e o estado atual \u{e9} {{ facts.pqr.value.status }}."}]);
    let out = compile(&c, &f, &row, &opp("template:t/estado_pqr", "status_message_gap"), &patch_proposal("template:t/estado_pqr", patch)).unwrap();
    let ch = &out.changes[0];
    assert_eq!((ch["kind"].as_str(), ch["content"]["id"].as_str(), ch["content"]["version"].as_str()), (Some("template"), Some("t/estado_pqr"), Some("1.0.1")));
    assert_eq!(ch["content"]["locales"]["es"], "Ya consult\u{e9} tu PQR y su estado actual es {{ facts.pqr.value.status }}. Si necesitas algo m\u{e1}s, d\u{ed}melo.");
    assert_eq!(ch["content"]["locales"]["pt"], "J\u{e1} consultei sua solicita\u{e7}\u{e3}o e o estado atual \u{e9} {{ facts.pqr.value.status }}. Se precisar de algo mais, me avise.");
    assert_eq!(out.cascade, vec!["agent:consultas@1.0.1", "flow:consulta-pqr@1.0.1"]);
    assert_eq!(out.base_digest, art.digest());
    assert!(out.edit_chars > 0 && out.edit_chars <= out.edit_budget);
    assert_eq!(out.expected_effect["evidence_ref"], f.evidence_ref().as_str());
    // the base catalogue is untouched (the compiler works on copies)
    assert_eq!(c.get("template:t/estado_pqr").unwrap().locales["es"], art.locales["es"]);
}

#[test]
fn an_insert_after_patch_on_the_copilot_prompt_keeps_every_safety_clause_and_the_profile() {
    let c = cat();
    let f = copilot_finding(Source::Synthetic);
    let row = map_finding(&f).unwrap();
    let art = c.get("prompt:p/copiloto").unwrap();
    let find = |l: &str, n: &str| anchors(art, l).into_iter().find(|a| a.text.contains(n)).unwrap().id;
    let patch = json!([
        {"locale": "es", "anchor_id": find("es", "Elige la tool seg"), "op": "insert_after", "replacement": "Si el pedido es sobre cargos de una disputa, lee leer_movimientos y leer_pqr_cliente en tus dos primeros pasos."},
        {"locale": "pt", "anchor_id": find("pt", "Escolha a tool"), "op": "insert_after", "replacement": "Se o pedido for sobre cobran\u{e7}as de uma disputa, leia leer_movimientos e leer_pqr_cliente nos dois primeiros passos."}]);
    let out = compile(&c, &f, &row, &opp("prompt:p/copiloto", "repeated_lookup"), &patch_proposal("prompt:p/copiloto", patch)).unwrap();
    let content = &out.changes[0]["content"];
    assert_eq!((content["version"].as_str(), content["model_profile"].as_str()), (Some("1.0.1"), Some("perfil-generacion@1")));
    for l in ["es", "pt"] {
        let (old, new) = (&art.locales[l], content["locales"][l].as_str().unwrap());
        assert!(new.len() > old.len() && new.starts_with(&old[..40]));
        for m in reasoning::catalog::protected_markers(art, l) {
            assert_eq!(old.contains(m), new.contains(m), "{l}: marker {m} survives");
        }
    }
    assert_eq!(out.cascade, vec!["agent:copiloto-asesor@1.0.1", "flow:asistir@1.0.1"]);
}

#[test]
fn the_compiler_denies_every_unsafe_proposal_with_a_closed_reason() {
    let c = cat();
    let f = pqr_finding();
    let row = map_finding(&f).unwrap();
    let o = opp("template:t/estado_pqr", "status_message_gap");
    let ok_es = json!({"locale": "es", "anchor_id": "es.a1", "op": "replace", "replacement": "Ya consult\u{e9} tu PQR y su estado es {{ facts.pqr.value.status }}."});
    let ok_pt = json!({"locale": "pt", "anchor_id": "pt.a1", "op": "replace", "replacement": "J\u{e1} consultei sua solicita\u{e7}\u{e3}o e o estado \u{e9} {{ facts.pqr.value.status }}."});
    let run = |patches: Value| compile(&c, &f, &row, &o, &patch_proposal("template:t/estado_pqr", patches));
    assert_eq!(denied(run(json!([ok_es, {"locale": "pt", "anchor_id": "pt.a99", "op": "replace", "replacement": "Texto novo aqui."}]))), "anchor_unknown");
    assert_eq!(denied(run(json!([ok_es]))), "locale_parity", "a locale left unpatched");
    assert_eq!(denied(run(json!([ok_es, {"locale": "pt", "anchor_id": "pt.a1", "op": "replace", "replacement": ok_es["replacement"]}]))), "translation_copy");
    assert_eq!(denied(run(json!([ok_es, {"locale": "pt", "anchor_id": "pt.a1", "op": "delete", "replacement": "x y z"}]))), "op_not_allowed");
    assert_eq!(denied(run(json!([{"locale": "es", "anchor_id": "es.a1", "op": "replace", "replacement": "Estado {{ facts.other.value }}."}, ok_pt]))), "placeholder_not_allowed");
    assert_eq!(denied(run(json!([{"locale": "es", "anchor_id": "es.a1", "op": "replace", "replacement": "Escribe a ayuda@banco.example para saber."}, ok_pt]))), "text_not_clean");
    assert_eq!(denied(run(json!([{"locale": "es", "anchor_id": "es.a1", "op": "replace", "replacement": "Tu radicado 123456789 ya fue consultado."}, ok_pt]))), "text_not_clean");
    assert_eq!(denied(run(json!([ok_es, {"locale": "pt", "anchor_id": "pt.a1", "op": "replace", "replacement": "Texto \u{27e6}pii:1\u{27e7} aqui."}]))), "text_not_clean");
    assert_eq!(denied(run(json!([ok_es, ok_es, ok_pt]))), "patch_overlap", "the same anchor twice");
    let long = "palabra ".repeat(45);
    assert_eq!(denied(run(json!([{"locale": "es", "anchor_id": "es.a1", "op": "replace", "replacement": long}, ok_pt]))), "edit_budget_exceeded");
    // a patch for a locale the artifact does not have
    assert_eq!(denied(run(json!([ok_es, ok_pt, {"locale": "en", "anchor_id": "en.a1", "op": "replace", "replacement": "Hello there."}]))), "locale_parity");
    // the wrong kind for the target (the target itself is the engine's, see the next test)
    let wrong_kind = compile(&c, &f, &row, &o, &json!({"kind": "new_agent", "rationale": "r", "expected_direction": "decrease", "alternatives": alts(), "uncertainty": "u"}));
    assert_eq!(denied(wrong_kind), "kind_mismatch");
}

#[test]
fn a_prompt_patch_cannot_introduce_a_placeholder() {
    let c = cat();
    let f = copilot_finding(Source::Synthetic);
    let row = map_finding(&f).unwrap();
    let art = c.get("prompt:p/copiloto").unwrap();
    let id = |l: &str, n: &str| anchors(art, l).into_iter().find(|a| a.text.contains(n)).unwrap().id;
    let patch = json!([{"locale": "es", "anchor_id": id("es", "Elige la tool seg"), "op": "insert_after", "replacement": "Usa {{ facts.x }} siempre."},
                       {"locale": "pt", "anchor_id": id("pt", "Escolha a tool"), "op": "insert_after", "replacement": "Use sempre o que ler."}]);
    assert_eq!(denied(compile(&c, &f, &row, &opp("prompt:p/copiloto", "repeated_lookup"), &patch_proposal("prompt:p/copiloto", patch))), "placeholder_not_allowed");
}

fn agent_proposal(slug: &str, sum_es: &str, ex_es: Value, ex_pt: Value) -> Value {
    json!({"kind": "new_agent", "target_ref": "new_agent:consultas", "agent_id": slug, "rationale": "A narrow intake for the uncovered topic.", "expected_direction": "decrease",
           "routing": {"summary_es": sum_es, "summary_pt": "Recebe problemas t\u{e9}cnicos do aplicativo e os encaminha a uma pessoa.", "examples_es": ex_es, "examples_pt": ex_pt},
           "intake": {"ask_es": "Cu\u{e9}ntame qu\u{e9} problema tienes con la aplicaci\u{f3}n.", "ask_pt": "Conte qual problema voc\u{ea} tem com o aplicativo.",
                      "notice_es": "Gracias, una persona del equipo te contactar\u{e1}.", "notice_pt": "Obrigado, uma pessoa da equipe vai falar com voc\u{ea}."},
           "alternatives": alts(), "uncertainty": "Where is known, why is not."})
}

#[test]
fn a_new_agent_is_a_closure_copy_of_the_donor_with_a_narrow_routing_card() {
    let c = cat();
    let f = tecnico_finding();
    let row = map_finding(&f).unwrap();
    let o = opp("new_agent:consultas", "uncovered_topic");
    let good = agent_proposal("soporte-tecnico", "Recibe problemas t\u{e9}cnicos de la aplicaci\u{f3}n y los pasa a una persona.", json!(["la app se cierra sola", "no puedo entrar a la aplicaci\u{f3}n"]), json!(["o aplicativo fecha sozinho", "n\u{e3}o consigo entrar no aplicativo"]));
    let out = compile(&c, &f, &row, &o, &good).unwrap();
    let kinds: Vec<&str> = out.changes.iter().map(|x| x["kind"].as_str().unwrap()).collect();
    assert_eq!(kinds, ["agent", "flow", "template", "template"]);
    let agent = &out.changes[0]["content"];
    assert_eq!((agent["id"].as_str(), agent["entry_flow"].as_str(), agent["tools_allowed"].clone()), (Some("soporte-tecnico"), Some("soporte-tecnico-intake@1"), json!([])));
    assert_eq!(agent["routing"]["directory"], c.agent("consultas").unwrap()["routing"]["directory"]);
    assert_eq!(agent["invocable_by"], c.agent("consultas").unwrap()["invocable_by"], "invocable_by is copied, never widened");
    assert_eq!(agent["supported_locales"], json!(["es", "pt"]));
    let flow = &out.changes[1]["content"];
    assert_eq!(flow["nodes"][0]["config"]["prompt_ref"], "t/soporte-tecnico_pedir");
    assert!(flow["nodes"].as_array().unwrap().iter().any(|n| n["type"] == "escalate"));
    assert!(!flow.to_string().contains("tool"), "no tool node: the new agent resolves nothing");
    for t in [&out.changes[2], &out.changes[3]] {
        assert!(t["content"]["locales"]["es"].is_string() && t["content"]["locales"]["pt"].is_string());
    }
    // AGT1: since INH1 the clone INHERITS the donor release settings by reference (agent-core `inherit_from`): the approver must not be told that an
    // admin still has to write the fraude interrupt or the injection ruleset (stale advice); the item names the inheritance and where to read it.
    assert!(out.human_items.iter().all(|h| !h.contains("admin release settings")), "stale pre-INH1 advice: {:?}", out.human_items);
    assert!(out.human_items.iter().any(|h| h.contains("inherit_from") && h.contains("fraude") && h.contains("injection")), "{:?}", out.human_items);

    let exs = (json!(["la app se cierra sola", "no puedo entrar a la aplicaci\u{f3}n"]), json!(["o aplicativo fecha sozinho", "n\u{e3}o consigo entrar no aplicativo"]));
    let try_with = |slug: &str, sum: &str, ex_es: Value| denied(compile(&c, &f, &row, &o, &agent_proposal(slug, sum, ex_es, exs.1.clone())));
    assert_eq!(try_with("soporte-pagos", "Recibe problemas t\u{e9}cnicos de la aplicaci\u{f3}n y los pasa a una persona.", exs.0.clone()), "slug_not_allowed");
    assert_eq!(try_with("disputas", "Recibe problemas t\u{e9}cnicos de la aplicaci\u{f3}n y los pasa a una persona.", exs.0.clone()), "slug_not_allowed");
    assert_eq!(try_with("soporte-tecnico", "Atiende cualquier problema del cliente con el banco y lo resuelve.", exs.0.clone()), "routing_invalid");
    // traffic stealing: an example copied from the sibling `consultas` card
    assert_eq!(try_with("soporte-tecnico", "Recibe problemas t\u{e9}cnicos de la aplicaci\u{f3}n y los pasa a una persona.", json!(["quiero saber el estado de mi PQR", "la app se cierra sola"])), "routing_invalid");
    assert_eq!(try_with("soporte-tecnico", "Recibe problemas t\u{e9}cnicos de la aplicaci\u{f3}n y los pasa a una persona.", json!(["una sola"])), "routing_invalid");
}

#[test]
fn no_change_compiles_to_nothing() {
    let c = cat();
    let f = pqr_finding();
    let row = map_finding(&f).unwrap();
    let out = compile(&c, &f, &row, &opp("template:t/estado_pqr", "wording"), &json!({"kind": "no_change", "rationale": "No safe change.", "expected_direction": "decrease", "alternatives": alts(), "uncertainty": "u"})).unwrap();
    assert!(out.changes.is_empty() && out.kind == "no_change");
}

#[test]
fn the_direction_is_derived_from_the_finding_and_the_models_wording_never_fails_the_proposal() {
    // every cells metric is higher-is-worse and the finding is "up": the expected direction is `decrease`, decided by the engine.
    // A model that says "increase" (BLD1: it meant "resolution increases"), omits it or writes anything else is simply overruled.
    let c = cat();
    let f = pqr_finding();
    let row = map_finding(&f).unwrap();
    let o = opp("template:t/estado_pqr", "status_message_gap");
    let es = json!({"locale": "es", "anchor_id": "es.a1", "op": "replace", "replacement": "Ya consult\u{e9} tu PQR y su estado es {{ facts.pqr.value.status }}."});
    let pt = json!({"locale": "pt", "anchor_id": "pt.a1", "op": "replace", "replacement": "J\u{e1} consultei sua solicita\u{e7}\u{e3}o e o estado \u{e9} {{ facts.pqr.value.status }}."});
    for wording in [json!("increase"), json!("down"), json!(null), json!(7)] {
        let mut p = patch_proposal("template:t/estado_pqr", json!([es, pt]));
        p["expected_direction"] = wording.clone();
        let out = compile(&c, &f, &row, &o, &p).unwrap_or_else(|d| panic!("{wording}: {d:?}"));
        assert_eq!(out.expected_effect["direction"], "decrease", "{wording}");
    }
    let mut p = patch_proposal("template:t/estado_pqr", json!([es, pt]));
    p.as_object_mut().unwrap().remove("expected_direction");
    assert_eq!(compile(&c, &f, &row, &o, &p).unwrap().expected_effect["direction"], "decrease");
}

#[test]
fn the_target_is_the_verified_opportunitys_and_a_models_different_target_ref_is_ignored() {
    // BLD1: for a new agent the model wrote the slug (`new_agent:soporte-tecnico`) where the target is `new_agent:consultas`.
    let c = cat();
    let f = pqr_finding();
    let row = map_finding(&f).unwrap();
    let o = opp("template:t/estado_pqr", "status_message_gap");
    let es = json!({"locale": "es", "anchor_id": "es.a1", "op": "replace", "replacement": "Ya consult\u{e9} tu PQR y su estado es {{ facts.pqr.value.status }}."});
    let pt = json!({"locale": "pt", "anchor_id": "pt.a1", "op": "replace", "replacement": "J\u{e1} consultei sua solicita\u{e7}\u{e3}o e o estado \u{e9} {{ facts.pqr.value.status }}."});
    let out = compile(&c, &f, &row, &o, &patch_proposal("prompt:p/copiloto", json!([es, pt]))).unwrap();
    assert_eq!(out.target_ref, "template:t/estado_pqr");
}

// ---- W15 / R11: the evidence ref and the generated docs never carry a digit run of 6 ------------------------------------------

fn longest_digit_run(s: &str) -> usize {
    let (mut best, mut cur) = (0, 0);
    for c in s.chars() {
        cur = if c.is_ascii_digit() { cur + 1 } else { 0 };
        best = best.max(cur);
    }
    best
}

#[test]
fn the_evidence_ref_is_hex_that_resolves_by_recomputation_and_has_no_digit_run_of_six() {
    for n in 1..800i64 {
        let f = finding_of(signal("M1", json!({"channel": "Phone"}), stage(300 + n, 1000 + n * 3, 0.1), stage(200 + n, 700 + n * 2, 0.1)), Source::Synthetic);
        let r = f.evidence_ref();
        assert!(r.starts_with("ev_") && r.len() == 19 && r[3..].chars().all(|c| c.is_ascii_hexdigit()), "{r}");
        assert!(longest_digit_run(&r) < 6, "{r}");
        assert_eq!(r, f.evidence_ref());
    }
}

#[test]
fn the_compiled_docs_are_free_of_digit_runs_of_six_whatever_the_model_wrote() {
    let f = tecnico_finding();
    let row = map_finding(&f).unwrap();
    let mut o = opp("new_agent:consultas", "uncovered_reason");
    o.hypothesis = "case 12345678 and ticket 987654321".into();
    let d = reasoning::patch::docs(&f, &row, &o, "seen in 123456789 calls");
    for k in ["description", "rationale"] {
        let t = d[k].as_str().unwrap();
        assert!(longest_digit_run(t) < 6, "{k}: {t}");
        assert!(t.contains("seen in"), "the text is kept, only the digit runs are broken: {t}");
    }
}

#[test]
fn a_single_brace_placeholder_is_denied_with_a_problem_the_builder_can_act_on() {
    let c = cat();
    let f = pqr_finding();
    let row = map_finding(&f).unwrap();
    let bad = json!([
        {"locale": "es", "anchor_id": "es.a1", "op": "replace", "replacement": "Tu PQR est\u{e1} en estado {facts.pqr.value.status}."},
        {"locale": "pt", "anchor_id": "pt.a1", "op": "replace", "replacement": "Sua solicita\u{e7}\u{e3}o est\u{e1} com status {{ facts.pqr.value.status }}."}]);
    let e = compile(&c, &f, &row, &opp("template:t/estado_pqr", "status_message_gap"), &patch_proposal("template:t/estado_pqr", bad)).expect_err("a single brace is refused");
    assert_eq!(e.code, "placeholder_not_allowed");
    assert!(e.why.contains("curly braces"), "{}", e.why);
}

fn tool_finding() -> reasoning::finding::Finding {
    finding_of(signal("A5", json!({"agent": "copiloto-asesor", "tool": "leer_pqr_cliente"}), stage(150, 200, 0.40), stage(120, 160, 0.40)), Source::Synthetic)
}

#[test]
fn link_tool_compiles_a_pass_through_read_link_from_an_edge_menu_choice() {
    let f = tool_finding();
    let row = map_finding(&f).expect("A5 on leer_pqr_cliente maps");
    assert_eq!(row.targets[0].kind, "link_tool");
    let o = opp("tool_link:consultas/leer_pqr_cliente", "missing_tool");
    let ok = |edge: &str| compile(&cat(), &f, &row, &o, &json!({"kind": "link_tool", "edge_id": edge, "rationale": "Add the read.", "alternatives": alts(), "uncertainty": "Association only."}));
    let c = ok("consultar.ok").expect("compiles");
    assert_eq!((c.kind.as_str(), c.agent_id.as_str(), c.changes.len()), ("link_tool", "consultas", 3));
    let flow = &c.changes[0]["content"];
    assert_eq!(flow["version"], "1.1.0");
    let n = flow["nodes"].as_array().unwrap().iter().find(|n| n["id"] == "eng_link_leer_pqr_cliente").unwrap();
    assert_eq!((n["next"]["ok"].as_str(), n["next"]["error"].as_str()), (Some("responder"), Some("esc_tool")));
    assert_eq!(c.changes[1]["content"]["tools_allowed"], json!(["obtener_pqr@1", "leer_pqr_cliente@1"]));
    assert_eq!(c.changes[2]["content"], cat().tool_def("leer_pqr_cliente").unwrap().clone()); // unchanged ToolDef copy
    assert!(c.human_items.iter().any(|h| h.contains("read-only")));
    assert_eq!(denied(ok("consultar.error")), "edge_unknown");
}
