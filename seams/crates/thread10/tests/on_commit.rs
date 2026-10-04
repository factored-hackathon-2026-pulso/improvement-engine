//! The live hook: after each handler commits, the caller sees a PARTIAL report built from what is committed so far.
use std::cell::RefCell;
use std::rc::Rc;
use thread10::{Opts, run};

#[test]
fn on_commit_sees_partial_reports_growing_one_handler_at_a_time() {
    let seen: Rc<RefCell<Vec<(usize, Vec<String>)>>> = Rc::default();
    let sink = seen.clone();
    let d = std::env::temp_dir().join(format!("t10-oncommit-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    let mut o = Opts::new(d, env!("CARGO_BIN_EXE_synth_runner").into());
    o.human_override = true;
    o.on_commit = Some(Rc::new(move |i, partial| {
        let ran: Vec<String> = partial["steps"].as_array().unwrap().iter().filter(|s| s["status"] != "not_exercised").map(|s| s["id"].as_str().unwrap().to_string()).collect();
        sink.borrow_mut().push((i, ran));
    }));
    let r = run(&o).expect("run");
    assert_eq!(r.error, None);
    let seen = seen.borrow();
    assert_eq!(seen.len(), 9, "one call per handler: {seen:?}");
    assert_eq!(seen.iter().map(|(i, _)| *i).collect::<Vec<_>>(), (0..9).collect::<Vec<_>>());
    assert!(seen[0].1.contains(&"signals".to_string()) && !seen[0].1.contains(&"gate".to_string()), "{:?}", seen[0]);
    assert!(seen.windows(2).all(|w| w[0].1.len() <= w[1].1.len()), "never shrinks");
    assert!(seen[8].1.contains(&"publish".to_string()));
}
