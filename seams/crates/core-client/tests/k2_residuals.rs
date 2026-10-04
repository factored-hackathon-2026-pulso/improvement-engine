//! K2 review residuals: path-param safety, code-point length limits, strict golden matching, placeholder tolerance.
mod common;
use common::{FakeCore, KID, SEED};
use core_client::dto::{Alias, Change, DryRunRequest, Stage, TaskInvocation};
use core_client::{CallError, ClientConfig, CoreClient, OpError, routes};
use serde_json::json;

fn client(addr: &str) -> CoreClient {
    let mut cfg = ClientConfig::new(addr, KID, SEED, "bridge-1");
    cfg.timeout = std::time::Duration::from_secs(5);
    CoreClient::new(cfg)
}

// ---- (a) path params --------------------------------------------------------------------------------------------

#[test]
fn dot_segments_are_rejected_before_anything_is_sent() {
    let f = FakeCore::start();
    for bad in ["..", ".", "", "a/b", r"a\b","x\u{0}y", "%2e%2e"] {
        let e = client(&f.addr).call(&routes::READ_TASK, "t1", None, &[bad], None, None).unwrap_err();
        assert!(matches!(e, CallError::InvalidPathParam(_)), "{bad:?} -> {e:?}");
    }
    let e = client(&f.addr).read_alias("t1", None, "..", Alias::Prod).unwrap_err();
    assert!(matches!(e, OpError::Call(CallError::InvalidPathParam(_))), "{e:?}");
    assert!(f.requests().is_empty(), "nothing may reach the wire");
}

#[test]
fn legal_path_params_are_percent_encoded() {
    let f = FakeCore::start();
    f.script(404, common::envelope("pulso:not_found", false));
    let _ = client(&f.addr).call(&routes::READ_TASK, "t1", None, &["run 1é..x"], None, None);
    assert_eq!(f.requests()[0].path, "/internal/v1/core-tasks/run%201%C3%A9..x");
}

// ---- (b) code points --------------------------------------------------------------------------------------------

#[test]
fn id_limits_count_code_points_like_python_not_utf8_bytes() {
    let ok = |n: usize, c: &str| DryRunRequest::new(&c.repeat(n), "atencion", None, vec![]).validate();
    assert!(ok(255, "é").is_ok(), "255 code points of 2 bytes each");
    assert!(ok(255, "😀").is_ok(), "255 code points of 4 bytes each");
    assert!(ok(256, "é").is_err());
    let mut t = TaskInvocation::new("t1", "job", Stage::Scout, &"ñ".repeat(256));
    t.agent_id = "a".into();
    t.agent_version = "1.0.0".into();
    t.release_id = "r".into();
    t.pulso_run_ref = "p".into();
    t.lab_grant_ref = "g".into();
    assert!(t.validate().is_ok(), "256 code points == the 256 limit");
    t.logical_key = "ñ".repeat(257);
    assert!(t.validate().is_err());
    t.logical_key = "k".into();
    t.release_id = "é".repeat(128);
    assert!(t.validate().is_ok());
    let c = vec![Change::new(&"é".repeat(64), json!({}), json!({}))];
    assert!(DryRunRequest::new("t", "a", None, c).validate().is_ok());
}

// ---- (c) strict golden matching --------------------------------------------------------------------------------

#[test]
fn a_nested_null_is_not_an_absent_key() {
    use common::golden::body_matches;
    let g = json!({"seed_manifest_ref": null, "input": {"proposal_id": null, "x": 1}, "target": {"a": null}});
    // top-level optional null == absent
    let ok = json!({"input": {"proposal_id": null, "x": 1}, "target": {"a": null}});
    assert!(body_matches(&g, &ok).is_ok());
    // dropping a nested null changes the request content: must be reported
    let no_nested = json!({"input": {"x": 1}, "target": {"a": null}});
    assert!(body_matches(&g, &no_nested).is_err());
    let no_target_key = json!({"input": {"proposal_id": null, "x": 1}, "target": {}});
    assert!(body_matches(&g, &no_target_key).is_err());
    // an extra nested null in the request is not in the golden either
    let extra = json!({"input": {"proposal_id": null, "x": 1, "y": null}, "target": {"a": null}});
    assert!(body_matches(&g, &extra).is_err());
}

// ---- (d) placeholders are for golden responses only --------------------------------------------------------------

#[test]
fn a_strict_client_refuses_golden_placeholders_in_binding_fields() {
    let f = FakeCore::start();
    f.script(200, json!({"execution_id":"<x>","status":"completed"}));
    let mut req = core_client::dto::ArmRequest::new("arm-k-1", "b", "c", "native", 1, json!(1), json!({}), "s", "bud");
    req.mode = Some(core_client::dto::ArmMode::Native);
    let e = client(&f.addr).run_arm("t1", None, &req).unwrap_err();
    assert!(matches!(e, OpError::Contract(_)), "{e:?}");
}

#[test]
fn ids_with_dot_at_and_colon_are_legal_path_params() {
    let f = FakeCore::start();
    f.script(404, common::envelope("pulso:not_found", false));
    let _ = client(&f.addr).call(&routes::READ_TASK, "t1", None, &["rel.1@a:b-c_d"], None, None);
    let reqs = f.requests();
    assert_eq!(reqs.len(), 1, "a legal id must reach the wire");
    assert!(reqs[0].path.starts_with("/internal/v1/core-tasks/rel.1"), "{}", reqs[0].path);
}
