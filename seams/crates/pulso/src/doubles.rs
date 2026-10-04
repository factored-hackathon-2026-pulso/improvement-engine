//! doubles[] (what is NOT real), derived from the report the same way as the G1 `generate_doubles` (contracts/engine-run):
//! every step that is not `real` (or says `real` with a non-real provider), every port, every simulated override.
//! `summary` prints them FIRST, then the steps with their labels (layout of demo/run-demo0.ps1 / demo0.summarize).
use serde_json::{Value, json};

const NONREAL_PROVIDERS: [&str; 4] = ["agent-roleplay", "recorded", "scripted", "claude-standin"];

fn norm(s: &str) -> String {
    s.trim().to_lowercase().split(|c: char| c.is_whitespace() || c == '_' || c == '-').filter(|p| !p.is_empty()).collect::<Vec<_>>().join("-")
}

pub fn generate(report: &Value) -> Vec<Value> {
    let mut d = Vec::new();
    for st in report["steps"].as_array().into_iter().flatten() {
        let provider = st["receipt"]["provider"].as_str();
        let lies = st["status"] == "real" && provider.is_none_or(|p| p.is_empty() || NONREAL_PROVIDERS.contains(&norm(p).as_str()));
        if st["status"] != "real" || lies {
            d.push(json!({"part": st["id"], "status": st["status"], "data_class": st["data_class"], "provider": provider}));
        }
    }
    for o in report["overrides"].as_array().into_iter().flatten() {
        let label = o["label"].as_str().unwrap_or("override");
        let status = if o["simulated"] == true { format!("{label}(simulated human, DEMO-0 stand-in)") } else { label.to_string() };
        d.push(json!({"part": format!("{}.override", o["of"].as_str().unwrap_or("?")), "status": status, "verdict": o["verdict"], "by": o["by"]}));
    }
    for p in report["ports"].as_array().into_iter().flatten() {
        d.push(json!({"part": format!("port.{}", p["port"].as_str().unwrap_or("?")), "status": p["provenance"], "price_source": p["price_source"]}));
    }
    d
}

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or("None")
}

pub fn summary(report: &Value) -> String {
    let mut l = vec![
        format!("DEMO-0 engine-run report: label {}, host {}, target {}, contract {}, sha {}", s(&report["label"]), s(&report["host"]), s(&report["target"]), s(&report["contract_revision"]), s(&report["sha"]).chars().take(12).collect::<String>()),
        format!("quality_claims: {}; gate judge {}, gate verdict {}", s(&report["quality_claims"]), s(&report["gate"]["judge"]), s(&report["gate"]["verdict"])),
        String::new(),
        "NOT REAL (doubles[], listed first; everything below is only as real as this list allows):".into(),
    ];
    for d in report["doubles"].as_array().into_iter().flatten() {
        let extra: Vec<String> = ["provider", "data_class"].iter().filter_map(|k| d[k].as_str().map(|v| format!("{k}={v}"))).collect();
        let tail = if extra.is_empty() { String::new() } else { format!(" ({})", extra.join(", ")) };
        l.push(format!("- {}: {}{}", s(&d["part"]), s(&d["status"]), tail));
    }
    for o in report["overrides"].as_array().into_iter().flatten() {
        l.push(format!("- override: {} of {} verdict {} by {} (simulated={}): {}", s(&o["label"]), s(&o["of"]), s(&o["verdict"]), s(&o["by"]), o["simulated"] == true, s(&o["reason"])));
    }
    l.push(String::new());
    l.push("STEPS (id, status, data class, receipt provider):".into());
    for st in report["steps"].as_array().into_iter().flatten() {
        l.push(format!("{}. {}: {} | {} | {}", st["n"].as_u64().map_or(String::new(), |n| n.to_string()), s(&st["id"]), s(&st["status"]), s(&st["data_class"]), st["receipt"]["provider"].as_str().unwrap_or("-")));
    }
    l.join("\n") + "\n"
}
