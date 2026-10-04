//! thread10 <work_dir> [--override] [--denied-kind] [--claimed-rate X] [--marker FILE IDX] [--now N] [--sha S]
//! Runs the offline Rust-shell thread and prints the report (JSON, one line) on stdout. Exit 0 job completed, 1 job stopped
//! (the report is still printed and says where), 2 usage. Env: STEPS_RUNNER_EXE (default: synth_runner next to this exe),
//! THREAD10_SHA. The report has no `doubles[]`: the Python twin (claude_standin.thread01_rust) adds them with G1 `generate_doubles`.
use thread10::{Opts, run};

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let Some(work) = a.first().filter(|w| !w.starts_with("--")) else {
        eprintln!("usage: thread10 <work_dir> [--override] [--denied-kind] [--claimed-rate X] [--marker FILE IDX] [--now N] [--sha S]");
        std::process::exit(2);
    };
    let runner = std::env::var_os("STEPS_RUNNER_EXE").map(std::path::PathBuf::from).unwrap_or_else(|| {
        let mut p = std::env::current_exe().expect("exe path");
        p.set_file_name(format!("synth_runner{}", std::env::consts::EXE_SUFFIX));
        p
    });
    let mut o = Opts::new(work.into(), runner);
    o.sha = std::env::var("THREAD10_SHA").ok().filter(|s| s.len() == 40).unwrap_or(o.sha);
    let mut it = a[1..].iter();
    let usage = |m: &str| -> ! {
        eprintln!("thread10: {m}");
        std::process::exit(2)
    };
    while let Some(f) = it.next() {
        match f.as_str() {
            "--override" => o.human_override = true,
            "--denied-kind" => o.denied_kind = true,
            "--claimed-rate" => o.claimed_rate = Some(it.next().and_then(|v| v.parse().ok()).unwrap_or_else(|| usage("--claimed-rate needs a number"))),
            "--now" => o.now = it.next().and_then(|v| v.parse().ok()).unwrap_or_else(|| usage("--now needs unix seconds")),
            "--sha" => o.sha = it.next().cloned().unwrap_or_else(|| usage("--sha needs a value")),
            "--ledger" => o.ledger = Some(it.next().cloned().unwrap_or_else(|| usage("--ledger needs FILE")).into()),
            "--kill-in-publish" => o.kill_in_publish = Some(it.next().cloned().unwrap_or_else(|| usage("--kill-in-publish needs FILE")).into()),
            "--marker" => {
                let (m, i) = (it.next(), it.next().and_then(|v| v.parse::<usize>().ok()));
                match (m, i) {
                    (Some(m), Some(i)) => o.kill_marker = Some((m.into(), i)),
                    _ => usage("--marker needs FILE IDX"),
                }
            }
            other => usage(&format!("unknown flag {other}")),
        }
    }
    match run(&o) {
        Ok(r) => {
            println!("{}", r.report);
            if let Some(e) = &r.error {
                eprintln!("thread10: job stopped: {e}");
                std::process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("thread10: {e}");
            std::process::exit(1);
        }
    }
}
