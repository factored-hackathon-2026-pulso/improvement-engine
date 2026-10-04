//! Harness binary: run_once <store_dir> [marker_file kill_after_index]
//! With a marker, creates the file after handler N commits and then blocks (waiting to be killed).
use engine::{demo, event_log, run_once, FileStore, Options};

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let store = FileStore::open(&a[1]).expect("store");
    let after_commit: Option<Box<dyn Fn(usize)>> = if a.len() >= 4 {
        let marker = a[2].clone();
        let n: usize = a[3].parse().expect("index");
        Some(Box::new(move |i| {
            if i == n {
                std::fs::write(&marker, "x").unwrap();
                loop {
                    std::thread::sleep(std::time::Duration::from_secs(1));
                }
            }
        }))
    } else {
        None
    };
    let opts = Options { job_id: "job-1".into(), worker_id: "w1".into(), crash_after: None, after_commit };
    let h = demo::handlers();
    run_once(&store, &h, "x", &opts).expect("run");
    for l in event_log(&store, h.len()).expect("log") {
        println!("{l}");
    }
}
