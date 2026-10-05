//! Argument parsing for `pulso` (pure, so it is unit-testable) and the real-Core refusal text.
use std::path::PathBuf;

pub const USAGE: &str = "usage:\n  pulso run [--exit-on-stdin-eof]   (container entrypoint; environment-only config, see `pulso run --help`)\n  pulso monitor [--once] [...]   (see `pulso monitor --help`)\n  pulso healthcheck [--port N]\n  pulso serve [--addr 127.0.0.1:4020] [--console-dir DIR] [--store-dir DIR] [--admin-token T] [--token T]\n  pulso demo [--api 127.0.0.1:4020] [--admin-token T] [--pace-ms N] [--run-id ID] [--work-dir DIR] [--sha S] [--no-override] [--denied-kind] [--real-core]\nEnv: PULSO_ADMIN_TOKEN, PULSO_DEBUG_TOKEN, PULSO_WORK_DIR, STEPS_RUNNER_EXE, THREAD10_SHA, PULSO_CORE_URL.";

#[derive(Debug, PartialEq)]
pub struct ServeArgs {
    pub addr: String,
    pub console_dir: Option<PathBuf>,
    pub store_dir: Option<PathBuf>,
    pub admin_token: Option<String>,
    pub token: Option<String>,
    /// Exit when stdin reaches EOF (the launching parent died or closed the pipe): no orphaned server after a hard kill.
    pub exit_on_stdin_eof: bool,
}

#[derive(Debug, PartialEq)]
pub struct DemoArgs {
    pub api: String,
    pub admin_token: Option<String>,
    pub pace_ms: u64,
    pub run_id: Option<String>,
    pub work_dir: Option<PathBuf>,
    pub sha: Option<String>,
    pub human_override: bool,
    pub denied_kind: bool,
    pub real_core: bool,
}

#[derive(Debug, PartialEq)]
pub enum Cmd {
    Serve(ServeArgs),
    Demo(DemoArgs),
}

pub fn parse(args: &[String]) -> Result<Cmd, String> {
    let (cmd, rest) = args.split_first().ok_or("missing subcommand (serve | demo)")?;
    let mut it = rest.iter();
    match cmd.as_str() {
        "serve" => {
            let mut a = ServeArgs { addr: "127.0.0.1:4020".into(), console_dir: None, store_dir: None, admin_token: None, token: None, exit_on_stdin_eof: false };
            while let Some(f) = it.next() {
                let mut v = || it.next().cloned().ok_or(format!("{f} needs a value"));
                match f.as_str() {
                    "--addr" => a.addr = v()?,
                    "--console-dir" => a.console_dir = Some(v()?.into()),
                    "--store-dir" => a.store_dir = Some(v()?.into()),
                    "--admin-token" => a.admin_token = Some(v()?),
                    "--token" => a.token = Some(v()?),
                    "--exit-on-stdin-eof" => a.exit_on_stdin_eof = true,
                    other => return Err(format!("unknown argument {other:?} for serve")),
                }
            }
            Ok(Cmd::Serve(a))
        }
        "demo" => {
            let mut a = DemoArgs { api: "127.0.0.1:4020".into(), admin_token: None, pace_ms: 400, run_id: None, work_dir: None, sha: None, human_override: true, denied_kind: false, real_core: false };
            while let Some(f) = it.next() {
                let mut v = || it.next().cloned().ok_or(format!("{f} needs a value"));
                match f.as_str() {
                    "--api" => a.api = v()?,
                    "--admin-token" => a.admin_token = Some(v()?),
                    "--pace-ms" => a.pace_ms = v()?.parse().map_err(|_| "--pace-ms needs a whole number of milliseconds".to_string())?,
                    "--run-id" => a.run_id = Some(v()?),
                    "--work-dir" => a.work_dir = Some(v()?.into()),
                    "--sha" => a.sha = Some(v()?),
                    "--no-override" => a.human_override = false,
                    "--denied-kind" => a.denied_kind = true,
                    "--real-core" => a.real_core = true,
                    other => return Err(format!("unknown argument {other:?} for demo")),
                }
            }
            Ok(Cmd::Demo(a))
        }
        other => Err(format!("unknown subcommand {other:?} (serve | demo)")),
    }
}

/// Why `--real-core` is refused. The default profile (offline DoublePort, DEMO-0) never needs it; nothing is labelled real here.
pub fn real_core_refusal(core_url: Option<&str>, reachable: bool) -> String {
    match (core_url, reachable) {
        (None, _) => "refusing --real-core: no real Core is configured (PULSO_CORE_URL is unset). Nothing was run and nothing is labelled real; \
            run `pulso demo` for the offline DEMO-0 profile."
            .into(),
        (Some(u), false) => format!("refusing --real-core: no real Core is reachable at {u}. Nothing was run and nothing is labelled real."),
        (Some(u), true) => format!(
            "refusing --real-core: a Core answers at {u}, but this build does not wire the live Core port into the demo, so no step can honestly become real-narrow here. \
             Nothing was run. The real-Core steps (5, 6, 8, 9) run via e2e-core/run.ps1 (one torn-down stack), not through pulso."
        ),
    }
}
