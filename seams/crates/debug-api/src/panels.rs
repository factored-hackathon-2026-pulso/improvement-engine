//! Panel projection: what the offline thread COMMITTED (`{"spec":..,"out":..}`, optionally with the report) -> the run events that
//! fill the console's Investigation, Diff, Gates, Alternatives and Decision panels. Pure and total: a panel whose inputs are not
//! committed yet produces no event (the console keeps saying "unknown"), nothing is invented, every string is derived from a
//! committed field, and every evidence item carries the digest of the committed value it summarises.
//!
//! Honesty rules kept here: the scout is a scripted value, the judge a stand-in, the Core an offline double and the human
//! SIMULATED; each of those is a visible `limits` item, never a quality claim. No trace id and no span is ever produced.
use crate::event::NewEvent;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

fn digest(v: &Value) -> String {
    Sha256::digest(v.to_string().as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

fn s<'a>(v: &'a Value, k: &str) -> Option<&'a str> {
    v.get(k).and_then(Value::as_str)
}

struct Evidence {
    items: Vec<Value>,
    at: String,
}

impl Evidence {
    /// Adds one item and returns its id (so a hypothesis can reference it).
    fn add(&mut self, id: &str, relation: &str, summary: String, source_kind: &str, validation: &str, committed: &Value) -> String {
        self.items.push(json!({
            "evidence_ref": {"id": id, "digest": digest(committed), "media_type": "application/json"},
            "relation": relation, "summary": summary, "source_kind": source_kind, "validation": validation, "available_at": self.at,
        }));
        id.to_string()
    }
}

fn completed_by_side(arms: &Value) -> Option<(usize, usize, usize)> {
    let runs = arms.get("runs")?.as_array()?;
    let count = |side: &str, only_done: bool| runs.iter().filter(|r| s(r, "side") == Some(side) && (!only_done || s(r, "status") == Some("completed"))).count();
    Some((count("base", true), count("cand", true), count("base", false)))
}

fn gate_of<'a>(gate: &'a Value, name: &str) -> Option<&'a Value> {
    gate.get("gates")?.as_array()?.iter().find(|g| s(g, "gate") == Some(name))
}

/// The scout's model record in the report (`models[]`, role `scout`), when the run recorded one.
fn scout_model(report: Option<&Value>) -> Option<&Value> {
    report?.get("models")?.as_array()?.iter().find(|m| m["role"] == "scout")
}

/// (limit text, is the claim a scripted value). Truthful per provider; no record keeps the offline-thread wording.
fn scout_limit(model: Option<&Value>) -> (String, bool) {
    let Some(m) = model else {
        return ("The scout claim is a scripted value of the job spec: no model produced it and no real source was read".into(), true);
    };
    let (provider, id) = (m["provider"].as_str().unwrap_or("scripted"), m["model_id"].as_str().unwrap_or("unknown"));
    if m["outcome"].as_str().is_some_and(|o| o != "answered") {
        return (format!("The scout model ({provider}, {id}) did not answer ({}): the claim below is the job spec's value, no model produced it", m["outcome"].as_str().unwrap_or("unknown")), true);
    }
    if m["real"] == true || provider.starts_with("gateway") {
        (format!("The scout claim was answered by the LLM gateway (model {id}); it was not recomputed from a real source, only by the synthetic lab sample"), false)
    } else if provider.contains("roleplay") {
        (format!("The scout claim is a replay of the roleplay-llm queue ({id}): an agent role-play, not a real model, and no real source was read"), false)
    } else if provider.contains("local") {
        (format!("The scout claim came from a locally declared model endpoint ({id}), not a hosted real model; no real source was read"), false)
    } else {
        ("The scout claim is a scripted value of the job spec: no model produced it and no real source was read".into(), true)
    }
}

fn investigation(committed: &Value, report: Option<&Value>, at: &str) -> Option<Value> {
    let (spec, out) = (&committed["spec"], &committed["out"]);
    let (recompute, validation) = (out.get("recompute"), out.get("validation"));
    if recompute.is_none() && validation.is_none() {
        return None;
    }
    let claim = spec["recompute"]["scout_claims"].as_array().and_then(|a| a.first());
    let sid = claim.and_then(|c| s(c, "signal_id")).or_else(|| validation.and_then(|v| s(v, "signal_id"))).unwrap_or("unknown");
    let claimed = claim.and_then(|c| c["claimed_rate"].as_f64());
    let verdict = validation.and_then(|v| s(v, "verdict")).unwrap_or("unknown").to_string();
    let mut ev = Evidence { items: vec![], at: at.to_string() };
    let mut main_refs = vec![];

    if let Some(row) = recompute.and_then(|r| r["recomputes"].as_array()).and_then(|a| a.iter().find(|r| s(r, "signal_id") == Some(sid))) {
        let matched = row["match"] == true;
        let (c, r) = (row["claimed_rate"].as_f64().unwrap_or(f64::NAN), row["recomputed_rate"].as_f64().unwrap_or(f64::NAN));
        main_refs.push(ev.add(
            &format!("ev-recompute-{sid}"), if matched { "supports" } else { "contradicts" },
            format!("Recompute over the synthetic lab sample: scout claimed {c:.2}, recomputed {r:.2} for {sid} ({})", if matched { "match" } else { "no match" }),
            "synthetic_lab", "recomputed", row,
        ));
    }
    for c in validation.and_then(|v| v["check_results"].as_array()).into_iter().flatten() {
        let (name, status) = (s(c, "check").unwrap_or("check"), s(c, "status").unwrap_or("unknown"));
        let rel = match status {
            "pass" => "supports",
            "fail" => "contradicts",
            _ => "limits",
        };
        main_refs.push(ev.add(&format!("ev-check-{name}"), rel, format!("Validation check {name}: {status}"), "engine_step", "stand_in", c));
    }
    if let Some(v) = validation {
        let (scout, verifier) = (s(&spec["validation"], "scout_actor").unwrap_or("unknown"), s(v, "verifier_actor").unwrap_or("unknown"));
        let independent = scout != verifier && scout != "unknown" && verifier != "unknown";
        main_refs.push(ev.add(
            "ev-verifier-independence", if independent { "limits" } else { "contradicts" },
            if independent {
                format!("Verifier {verifier} is a different actor id than the scout {scout}; independence is by id only, both are stand-ins and no model ran")
            } else {
                format!("Verifier {verifier} is NOT independent of the scout {scout}")
            },
            "engine_step", "stand_in", &json!({"scout_actor": scout, "verifier_actor": verifier}),
        ));
    }
    let (scout_text, scripted) = scout_limit(scout_model(report));
    main_refs.push(ev.add(
        "ev-limit-scripted-scout", "limits",
        scout_text, "job_spec", "stand_in", &claim.cloned().unwrap_or(Value::Null),
    ));
    // A signal that came from a real sensor run (the monitor tick over a platform event package) carries its provenance in the
    // report's `source`; the thread's own sensors step is then still a fixed-output stand-in and says so.
    let source = report.and_then(|r| r.get("source")).filter(|x| x.is_object());
    if let Some(src) = source {
        let g = |k: &str| s(src, k).unwrap_or("unknown");
        let (num, den) = (src["numerator"].as_u64().map_or_else(|| "?".to_string(), |n| n.to_string()), src["denominator"].as_u64().map_or_else(|| "?".to_string(), |n| n.to_string()));
        main_refs.push(ev.add(
            "ev-source-signal", "supports",
            format!("Signal {} of cell {} ({num}/{den}) was admitted by the {} sensor over event package {} ({} events read via {} from {}, data origin {})", g("metric_id"), g("cell"), g("sensor"), g("package"), src["events_read"].as_u64().unwrap_or(0), g("adapter"), g("source_id"), g("data_origin")),
            "sensor_package", "computed", src,
        ));
    }
    if let Some(sensed) = out.get("sensors") {
        let n = |k: &str| sensed[k].as_array().map_or(0, Vec::len);
        let text = if source.is_some() {
            format!("The thread's sensors step is a fixed-output stand-in ({} signal(s), {} discard(s)); the signal under test came from the real sensor run above and entered the thread only through the lab row", n("signals"), n("discards"))
        } else {
            format!("The sensor stand-in produced {} signal(s) and {} discard(s) from a fixed-output runner; it reads no data", n("signals"), n("discards"))
        };
        main_refs.push(ev.add("ev-limit-sensor", "limits", text, "engine_step", "stand_in", sensed));
    }

    let who = if scripted { "Scripted scout" } else { "Scout" };
    let statement = match claimed {
        Some(c) => format!("{who} claims signal {sid} has rate {c:.2}"),
        None => format!("{who} claim about signal {sid}"),
    };
    let mut hypotheses = vec![json!({"hypothesis_id": "main", "statement": statement, "verdict": verdict, "evidence_refs": main_refs})];

    if let Some(compile) = out.get("compile") {
        let ops = compile["draft_plan"]["operations"].as_array();
        let mechanism = s(&spec["compile"]["change_spec"], "expected_mechanism").unwrap_or("unspecified mechanism");
        let what = ops.map_or_else(
            || "the requested change".to_string(),
            |o| o.iter().map(|x| match s(x, "op") {
                Some("add") => format!("add {}", s(x, "new_ref").unwrap_or("?")),
                op => format!("{} {} -> {}", op.unwrap_or("?"), s(x, "target_ref").unwrap_or("?"), s(x, "new_ref").unwrap_or("?")),
            }).collect::<Vec<_>>().join(" + "),
        );
        let mut refs = vec![];
        let hverdict = if s(compile, "status") != Some("compiled") {
            format!("blocked({})", s(compile, "denied_reason").unwrap_or("compile"))
        } else if let Some(gate) = out.get("gate") {
            let imp = gate_of(gate, "improvement");
            let (status, reason) = (imp.and_then(|g| s(g, "status")).unwrap_or("unknown"), imp.and_then(|g| s(g, "reason")));
            let arms = out.get("arms").and_then(completed_by_side);
            let counts = arms.map_or(String::new(), |(b, c, n)| format!(". Base completed {b}/{n} cases, candidate {c}/{n}: arms {}", if b == c { "identical on completed cases" } else { "differ" }));
            refs.push(ev.add(
                "ev-gate-improvement", if status == "pass" { "supports" } else { "contradicts" },
                format!("Improvement gate (stand-in judge): {status}{}{counts}", reason.map_or(String::new(), |r| format!(" ({r})"))),
                "stand_in_gate", "stand_in", imp.unwrap_or(gate),
            ));
            if let Some(sf) = gate_of(gate, "safety") {
                refs.push(ev.add("ev-gate-safety", if s(sf, "status") == Some("pass") { "supports" } else { "contradicts" }, format!("Safety gate (stand-in judge): {}{}", s(sf, "status").unwrap_or("unknown"), s(sf, "reason").map_or(String::new(), |r| format!(" ({r})"))), "stand_in_gate", "stand_in", sf));
            }
            match status {
                "pass" => "supported_by_stand_in_gate".to_string(),
                "fail" => "not_supported".to_string(),
                other => other.to_string(),
            }
        } else {
            "unknown".to_string()
        };
        if let Some(ne) = out.get("native_eval") {
            refs.push(ev.add("ev-native-eval", if s(ne, "verdict") == Some("pass") { "supports" } else { "contradicts" }, format!("Native evaluation (offline Core double): {}", s(ne, "verdict").unwrap_or("unknown")), "core_double", "stand_in", ne));
        }
        refs.push(ev.add("ev-limit-judge", "limits", "The gate judge is a stand-in; the oracle is the suite's own expect blocks; the Core is an offline double. This is a structural check, not a quality claim".into(), "engine_step", "stand_in", &json!({"judge": s(&out["gate"], "judge_actor")})));
        hypotheses.push(json!({"hypothesis_id": "change", "statement": format!("Applying {what} ({mechanism}) improves the suite's cases"), "verdict": hverdict, "evidence_refs": refs}));
    }
    Some(json!({"hypothesis": hypotheses[0]["statement"], "verifier": verdict, "evidence": ev.items, "hypotheses": hypotheses}))
}

fn alternatives(committed: &Value) -> Option<Value> {
    let compile = committed["out"].get("compile")?;
    let base = s(&committed["spec"]["compile"], "base_bundle_ref").unwrap_or("the base bundle");
    let ops = compile["draft_plan"]["operations"].as_array().map(|o| o.iter().map(|x| format!("{} {}", s(x, "op").unwrap_or("?"), s(x, "new_ref").unwrap_or("?"))).collect::<Vec<_>>().join(", "));
    let mut items = vec![json!({"id": "alt-do-nothing", "kind": "do_nothing", "summary": format!("Keep {base} unchanged"), "expected_abandoned": null, "risk": null})];
    items.push(match ops {
        Some(o) => json!({"id": "alt-proposed", "kind": "proposed_change", "summary": format!("Propose: {o}"), "expected_abandoned": null, "risk": null}),
        None => json!({"id": "alt-proposed", "kind": "proposed_change", "summary": format!("Blocked: compile {}", s(compile, "denied_reason").unwrap_or("did not produce a plan")), "expected_abandoned": null, "risk": null}),
    });
    Some(json!({"items": items}))
}

fn proposal_id(committed: &Value) -> Option<String> {
    let out = &committed["out"];
    if let Some(id) = s(&out["arms"]["frozen"], "proposal_id") {
        return Some(id.to_string());
    }
    let d = s(&out["compile"]["draft_plan"], "digest")?;
    Some(format!("draft-{}", d.trim_start_matches("sha256:").chars().take(16).collect::<String>()))
}

fn diff(committed: &Value) -> Option<Value> {
    let (spec, plan) = (&committed["spec"], committed["out"].get("compile")?.get("draft_plan")?);
    let mut lines = vec![json!({"op": "ctx", "text": format!("bundle: {}", s(&spec["compile"], "base_bundle_ref").unwrap_or("unknown"))})];
    for (i, o) in plan["operations"].as_array()?.iter().enumerate() {
        let (op, kind) = (s(o, "op").unwrap_or("?"), s(o, "target_kind").unwrap_or("?"));
        lines.push(json!({"op": "ctx", "text": format!("operation {}: {op} {kind}", i + 1)}));
        if op != "add" {
            lines.push(json!({"op": "del", "text": format!("  {}", s(o, "target_ref").unwrap_or("?"))}));
        }
        lines.push(json!({"op": "add", "text": format!("  {}", s(o, "new_ref").unwrap_or("?"))}));
        lines.push(json!({"op": "ctx", "text": format!("  precondition_digest: {}", s(o, "precondition_digest").unwrap_or("?"))}));
    }
    lines.push(json!({"op": "ctx", "text": format!("draft plan digest: {}", s(plan, "digest").unwrap_or("?"))}));
    lines.push(json!({"op": "ctx", "text": "The committed draft plan carries refs and digests only: the prompt and suite text is not in the engine output, so no text diff is shown"}));
    Some(json!({"proposal_id": proposal_id(committed)?, "lines": lines}))
}

fn gates(committed: &Value, at: &str) -> Option<Value> {
    let (spec, out) = (&committed["spec"], &committed["out"]);
    let gate = out.get("gate")?;
    let (safety, imp) = (gate_of(gate, "safety")?, gate_of(gate, "improvement")?);
    let (ss, is) = (s(safety, "status").unwrap_or("unknown"), s(imp, "status").unwrap_or("unknown"));
    let ne = out.get("native_eval").and_then(|n| s(n, "verdict"));
    let auth = out.get("authority");
    let overridden = auth.is_some_and(|a| a["override"].is_object());
    let decision = if overridden {
        "override_simulated_human"
    } else if auth.is_some() {
        "approved_simulated_human"
    } else if ss == "pass" && is == "pass" {
        "eligible"
    } else {
        "hold"
    };
    let reason = s(imp, "reason").or_else(|| s(safety, "reason")).map(|r| if overridden { format!("{r}_overridden_by_simulated_human") } else { r.to_string() });
    let native_reason = ne.map_or_else(|| s(safety, "reason").map(String::from), |v| Some(format!("{}Core native evaluation (offline double): {v}", s(safety, "reason").map_or(String::new(), |r| format!("{r}; ")))));
    let pid = proposal_id(committed);
    let scope = out["compile"]["draft_plan"]["operations"].as_array().map(|o| o.iter().filter_map(|x| s(x, "target_kind")).collect::<Vec<_>>().join("+")).unwrap_or_else(|| "unknown".into());
    let run_id = s(&spec["gate"]["gate_in"], "run_id").unwrap_or("run");
    Some(json!({
        "native": {"status": ss, "reason_code": s(safety, "reason"), "report_ref": {"id": format!("gate-report:{run_id}"), "digest": digest(gate), "media_type": "application/json"}, "checked_at": at},
        "improvement": {"status": is, "reason_code": s(imp, "reason"), "receipt_refs": [], "checked_at": at},
        "combined": {"decision": decision, "reason_code": reason},
        "proposal_id": pid,
        "attempts": [{
            "attempt": 1, "candidate_id": pid.clone().unwrap_or_else(|| "unknown".into()), "scope": scope, "native": ss, "native_reason": native_reason,
            "improvement": {"status": is, "reason_code": s(imp, "reason"), "lift": null, "lift_lo": null, "exposure": null, "guard_max_exposure": null},
        }],
    }))
}

fn decision(committed: &Value, report: Option<&Value>) -> Option<Value> {
    let (spec, out) = (&committed["spec"], &committed["out"]);
    let gate = out.get("gate")?;
    let auth = out.get("authority");
    let blocked = report.and_then(|r| r["steps"].as_array()).and_then(|a| a.iter().find(|st| s(st, "id") == Some("approval"))).and_then(|st| s(st, "status")).filter(|st| st.starts_with("blocked"));
    if auth.is_none() && blocked.is_none() {
        return None;
    }
    let gate_state = |name: &str| gate_of(gate, name).map(|g| json!({"status": s(g, "status"), "reason": s(g, "reason")}));
    let run_id = s(&spec["gate"]["gate_in"], "run_id").unwrap_or("run");
    let mut reasons = vec![];
    let (id, state, actor, over) = match auth {
        Some(a) => {
            let over = a.get("override").filter(|o| o.is_object()).cloned();
            reasons.push(match &over {
                Some(o) => format!("Gate verdict was {}; a SIMULATED human override ({}) was applied: {}", s(gate, "verdict").unwrap_or("unknown"), s(o, "label").unwrap_or("human_override"), s(o, "reason").unwrap_or("no reason given")),
                None => {
                    let st = |n: &str| gate_of(gate, n).and_then(|g| s(g, "status")).unwrap_or("unknown");
                    if st("safety") == "pass" && st("improvement") == "pass" {
                        "Both gates passed; the SIMULATED human approved without override".to_string()
                    } else {
                        format!("The SIMULATED human approved without a recorded override; the committed gates say safety {} and improvement {}", st("safety"), st("improvement"))
                    }
                }
            });
            (s(a, "decision_id").unwrap_or("dec-unknown").to_string(), s(a, "state").unwrap_or("unknown").to_string(), s(a, "actor").map(String::from), over)
        }
        None => {
            reasons.push(format!("Approval {}: the gate verdict {} is not pass and no complete explicit simulated override was configured", blocked.unwrap_or("blocked"), s(gate, "verdict").unwrap_or("unknown")));
            (format!("dec-{run_id}-blocked"), "blocked".to_string(), None, None)
        }
    };
    reasons.push("No human took this decision: the issuer, the actor and any override are simulated by the demo (DEMO-0). This is not a quality claim".to_string());
    Some(json!({
        "decision_id": id, "available_commands": [], "needs_step_up": false, "domain_revision": 1,
        "card": {
            "state": state, "simulated": true, "label": "SIMULATED", "issuer": "simulated-issuer", "actor": actor,
            "gate": {"verdict": s(gate, "verdict"), "safety": gate_state("safety"), "improvement": gate_state("improvement")},
            "override": over, "reasons": reasons, "proposal_id": proposal_id(committed), "quality_claims": "forbidden",
        },
    }))
}

/// The panel events of what is committed so far. `at` stamps evidence and gate checks (RFC 3339). Order: investigation,
/// alternatives, diff, gates, decision. A panel with no committed inputs yields no event.
pub fn project(committed: &Value, report: Option<&Value>, at: &str) -> Vec<NewEvent> {
    let mut v = vec![];
    if let Some(d) = investigation(committed, report, at) {
        v.push(NewEvent::new("investigation_set", "run", "investigation", d));
    }
    if let Some(d) = alternatives(committed) {
        v.push(NewEvent::new("alternatives_set", "run", "alternatives", d));
    }
    if let Some(d) = diff(committed) {
        v.push(NewEvent::new("diff_set", "proposal", d["proposal_id"].as_str().unwrap_or("proposal"), d.clone()));
    }
    if let Some(d) = gates(committed, at) {
        v.push(NewEvent::new("gates_set", "run", "gates", d));
    }
    if let Some(d) = decision(committed, report) {
        v.push(NewEvent::new("decision_set", "decision", d["decision_id"].as_str().unwrap_or("decision"), d.clone()));
    }
    v
}
