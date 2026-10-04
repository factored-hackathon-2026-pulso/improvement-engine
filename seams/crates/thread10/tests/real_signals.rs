//! W7: signals that come from a real sensor run (cell metric ids, treated data class) and the per-payload hook of the pipeline.
use engine::models::tps::{DEFAULT_K, scan_payload};
use engine::models::{DataClass, ModelRequest};
use serde_json::Value;
use std::cell::RefCell;
use std::rc::Rc;
use thread10::pipeline::{PipelineOpts, run_signals};
use thread10::requests::{builder_request, scout_request, verifier_request};
use thread10::SignalSeed;

const CELL: &str = "reassignment_rate.pt.web_chat";

fn cell_seed() -> SignalSeed {
    SignalSeed::new("sig-cell", "ev-cell", 54, 321).with_metric(CELL).with_data_class(DataClass::Treated)
}

fn all_requests(s: &SignalSeed) -> Vec<ModelRequest> {
    vec![scout_request(s), verifier_request(s, 0.17), builder_request(s)]
}

#[test]
fn a_cell_metric_id_reaches_the_model_request_registered_and_the_scan_passes() {
    for req in all_requests(&cell_seed()) {
        assert!(req.registry.iter().any(|t| t == CELL), "{:?}: metric id not registered", req.role);
        let scan = scan_payload(&req.payload, DEFAULT_K, &req.registry);
        assert!(scan.ok, "{:?}: {:?}", req.role, scan.violations);
    }
    let scout = scout_request(&cell_seed());
    assert_eq!(scout.payload["inputs"]["metric_id"], CELL);
    assert_eq!(scout.payload["observations"][0]["args"]["metric_id"], CELL);
    assert_eq!(scout.payload["observations"][0]["result"]["rows"][0]["metric_id"], CELL);
}

#[test]
fn a_cell_metric_id_that_is_not_registered_would_be_refused_by_the_scan() {
    // the guard of this test: the registry entry is what makes the id acceptable, not a loophole in the scan
    let mut req = scout_request(&cell_seed());
    req.registry.retain(|t| t != CELL);
    // `shaped` ids pass without registration; a dotted cell id is not system-issued shaped
    let scan = scan_payload(&req.payload, DEFAULT_K, &req.registry);
    assert!(!scan.ok, "an unregistered cell metric id must not pass");
}

#[test]
fn the_default_seed_keeps_the_synthetic_metric_and_class() {
    let s = SignalSeed::lab_default();
    assert_eq!(s.metric_id, None);
    for req in all_requests(&s) {
        assert_eq!(req.data_class, DataClass::Synthetic);
    }
    assert_eq!(scout_request(&s).payload["inputs"]["metric_id"], "synthetic_metric");
}

#[test]
fn the_seed_data_class_is_the_class_of_every_request() {
    for req in all_requests(&cell_seed()) {
        assert_eq!(req.data_class, DataClass::Treated, "{:?}", req.role);
    }
}

#[test]
fn on_payload_is_called_for_every_committed_handler_of_every_signal() {
    let seen: Rc<RefCell<Vec<(usize, usize, bool)>>> = Rc::default();
    let sink = seen.clone();
    let signals = vec![SignalSeed::new("sig-a", "ev-a", 120, 400), SignalSeed::new("sig-b", "ev-b", 30, 100)];
    let work = std::env::temp_dir().join(format!("t10rs-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&work);
    let mut o = PipelineOpts::new(work, env!("CARGO_BIN_EXE_synth_runner").into(), "run-hook", signals);
    o.on_payload = Some(Rc::new(move |signal: usize, handler: usize, payload: &Value| sink.borrow_mut().push((signal, handler, payload["spec"].is_object()))));
    let run = run_signals(&o).unwrap();
    assert_eq!(run.entries.len(), 2);
    let seen = seen.borrow();
    for signal in 0..2 {
        let handlers: Vec<usize> = seen.iter().filter(|(s, ..)| *s == signal).map(|(_, h, _)| *h).collect();
        assert!(!handlers.is_empty(), "signal {signal}: hook never called");
        assert!(handlers.windows(2).all(|w| w[0] < w[1]), "signal {signal}: handlers out of order {handlers:?}");
    }
    assert!(seen.iter().all(|(_, _, spec)| *spec), "every call carries the committed {{spec,out}} payload");
}
