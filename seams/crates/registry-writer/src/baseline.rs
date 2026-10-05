//! The baseline text from the LIVE registry (`GET /v1/registry/entities/{kind}/{id}`), replacing the `fixture-baseline` catalogue
//! for every artifact the registry can serve. An artifact the registry cannot serve keeps its fixture text and is listed as such,
//! and the catalogue label says which it is: `live-registry`, `mixed-live-and-fixture` or the original fixture label.
use crate::Reason;
use reasoning::catalog::{Artifact, Catalog};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

/// Registry path of an entity (`kind` and `id`, the id may hold `/`).
pub fn entity_path(kind: &str, id: &str) -> Option<String> {
    let ok = crate::guard::ok_seg(kind) && !id.is_empty() && id.split('/').all(crate::guard::ok_seg);
    ok.then(|| format!("/v1/registry/entities/{kind}/{id}"))
}

/// Reads an `EntityVersion` answer (`{ref: {kind,id,version}, content: {id, version?, locales, ...}}`) as a catalogue artifact.
/// `referenced_by` is not part of the answer: the caller carries it over from the fixture.
pub fn parse_entity(v: &Value, referenced_by: Vec<(String, String, String)>) -> Option<Artifact> {
    let r = &v["ref"];
    let (kind, id, version) = (r["kind"].as_str()?, r["id"].as_str()?, r["version"].as_str()?);
    let content = v["content"].as_object()?;
    let locales: BTreeMap<String, String> = content.get("locales")?.as_object()?.iter().filter_map(|(k, t)| Some((k.clone(), t.as_str()?.to_string()))).collect();
    if locales.is_empty() {
        return None;
    }
    let extra: Map<String, Value> = content.iter().filter(|(k, _)| !["id", "version", "locales"].contains(&k.as_str())).map(|(k, x)| (k.clone(), x.clone())).collect();
    Some(Artifact { kind: kind.into(), id: id.into(), version: version.into(), locales, extra: Value::Object(extra), referenced_by })
}

#[derive(Debug, Clone)]
pub struct Refreshed {
    pub catalog: Catalog,
    /// `kind:id` read from the live registry.
    pub live: Vec<String>,
    /// `(kind:id, reason code)` kept at the fixture text.
    pub fixture: Vec<(String, &'static str)>,
}

/// Replaces every artifact `fetch` can serve. `fetch` gets a `kind:id` target reference.
pub fn refresh_with(catalog: &Catalog, refs: &[String], fetch: &dyn Fn(&str) -> Result<Artifact, (Reason, String)>) -> Refreshed {
    let mut out = catalog.clone();
    let (mut live, mut fixture) = (vec![], vec![]);
    for r in refs {
        let carried = catalog.get(r).map(|a| a.referenced_by.clone()).unwrap_or_default();
        match fetch(r) {
            Ok(mut a) => {
                if a.referenced_by.is_empty() {
                    a.referenced_by = carried;
                }
                out.put_artifact(a);
                live.push(r.clone());
            }
            Err((reason, _)) => fixture.push((r.clone(), reason.code())),
        }
    }
    if fixture.is_empty() && !live.is_empty() {
        out.label = "live-registry".into();
    } else if !live.is_empty() {
        out.label = "mixed-live-and-fixture".into();
    }
    Refreshed { catalog: out, live, fixture }
}
