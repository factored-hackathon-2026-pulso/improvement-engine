use std::collections::BTreeSet;
use std::env;
use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use improvement_engine_core::local_simulation::{
    LocalObservedEvent, LocalObservedQuery, LocalRunInput, LocalRunMetadata, LocalRunResult,
    LocalSourceKind, RunEvent, run_local_simulation,
};
use improvement_engine_source_adapters::{
    CasePhase, E0Fact, E0HoldoutEvaluation, E0HoldoutPolicy, E0HoldoutStatus, PreparationConfig,
    PreparedSource, SourceKind, attest_selected_e0_recurrence_candidate,
    evaluate_e0_recurrence_holdout, prepare_e0_package, prepare_original_bank,
};

fn main() {
    if let Err(error) = run(env::args_os().skip(1)) {
        eprintln!("error: {error}");
        std::process::exit(2);
    }
}

fn run(args: impl IntoIterator<Item = OsString>) -> Result<(), String> {
    let options = Options::parse(args)?;
    if options.help {
        print_help();
        return Ok(());
    }
    validate_mode(&options)?;
    let config = PreparationConfig::new(
        options.tenant_id.clone(),
        options.observed_cutoff,
        options.arranque_cases,
    )
    .map_err(|error| format!("invalid source preparation config: {error}"))?;
    let prepared = match options.source.as_str() {
        "e0" => prepare_e0_package(&options.input, &config),
        "original" => prepare_original_bank(&options.input, &config),
        _ => return Err("--source must be e0 or original".into()),
    }
    .map_err(|error| format!("source preparation failed: {error}"))?;

    let run_id = make_run_id()?;
    let input = to_run_input(&prepared, &run_id, &options.tenant_id)?
        .with_minimum_recurring_query_support(options.minimum_recurring_query_support)
        .map_err(|error| format!("invalid recurrence policy: {error:?}"))?;
    let result = run_local_simulation(input).map_err(|error| format!("run failed: {error}"))?;
    let holdout = evaluate_selected_recurrence_after_discovery(
        &prepared,
        &result,
        options.minimum_recurring_query_support,
    )?;
    let holdout_event = holdout.as_ref().map(|evaluation| {
        make_holdout_event(
            evaluation,
            result.events.len() as u32 + 1,
            &result.observed_cutoff_rfc3339,
        )
    });
    persist_result(
        &options.output,
        &result,
        holdout.as_ref(),
        holdout_event.as_ref(),
    )?;
    println!("run_id={run_id}");
    println!("status={}", result.terminal_status);
    println!("mode={}", result.execution_mode);
    println!(
        "result={}",
        options.output.join(&run_id).join("result.json").display()
    );
    println!(
        "events={}",
        options.output.join(&run_id).join("events.ndjson").display()
    );
    Ok(())
}

fn evaluate_selected_recurrence_after_discovery(
    prepared: &PreparedSource,
    result: &LocalRunResult,
    minimum_support: u64,
) -> Result<Option<E0HoldoutEvaluation>, String> {
    if prepared.source_kind() != SourceKind::E0
        || result.signal.as_ref().is_none_or(|signal| {
            signal.metric_id != "e0_recurring_copilot_query_cases" || signal.pattern_ref.is_none()
        })
        || !result
            .candidates
            .iter()
            .any(|candidate| candidate.kind == "opportunity")
    {
        return Ok(None);
    }

    // The core runner has already completed discovery and admitted its
    // opportunity candidate. Reproduccion enters only here, after selection.
    let selected_pattern_ref = result
        .signal
        .as_ref()
        .and_then(|signal| signal.pattern_ref.as_deref())
        .ok_or_else(|| "selected recurrence lacks a pattern reference".to_owned())?;
    let selected_candidate =
        attest_selected_e0_recurrence_candidate(prepared, selected_pattern_ref, minimum_support)
            .map_err(|_| {
                "selected core recurrence could not be attested against Arranque".to_owned()
            })?;
    let policy = E0HoldoutPolicy::new(minimum_support)
        .map_err(|_| "holdout policy is outside its safe range".to_owned())?;
    evaluate_e0_recurrence_holdout(&selected_candidate, prepared, &policy)
        .map(Some)
        .map_err(|_| "E0 holdout evaluation rejected its source scope".to_owned())
}

fn make_holdout_event(
    evaluation: &E0HoldoutEvaluation,
    sequence: u32,
    observed_cutoff: &str,
) -> RunEvent {
    let status = match evaluation.status() {
        E0HoldoutStatus::Replicated => "replicated",
        E0HoldoutStatus::NotObserved => "not_observed",
        E0HoldoutStatus::InsufficientSupport => "insufficient_support",
        E0HoldoutStatus::Unavailable => "unavailable",
    }
    .to_owned();
    let detail = match evaluation.status() {
        E0HoldoutStatus::InsufficientSupport => {
            "post-selection descriptive recurrence: insufficient support; exact counts suppressed; no causal or outcome claim".into()
        }
        E0HoldoutStatus::Unavailable => {
            "post-selection descriptive recurrence unavailable; no causal or outcome claim".into()
        }
        E0HoldoutStatus::Replicated | E0HoldoutStatus::NotObserved => {
            match (
                evaluation.matching_case_count(),
                evaluation.queried_case_count(),
            ) {
                (Some(matching), Some(queried)) => format!(
                    "post-selection descriptive recurrence: {matching} of {queried} queried Reproduccion cases; no causal or outcome claim"
                ),
                _ => "post-selection descriptive recurrence unavailable; no causal or outcome claim".into(),
            }
        }
    };
    RunEvent {
        sequence,
        stage: "e0_recurrence_holdout".into(),
        status,
        detail,
        observed_cutoff_rfc3339: observed_cutoff.to_owned(),
    }
}

fn to_run_input(
    prepared: &PreparedSource,
    run_id: &str,
    tenant_id: &str,
) -> Result<LocalRunInput, String> {
    let source_kind = match prepared.source_kind() {
        improvement_engine_source_adapters::SourceKind::E0 => LocalSourceKind::E0,
        improvement_engine_source_adapters::SourceKind::OriginalBank => {
            LocalSourceKind::OriginalBank
        }
    };
    let case_ordinals: Vec<u32> = prepared
        .agent_inputs()
        .cases()
        .iter()
        .filter(|case| case.phase() == CasePhase::Arranque)
        .map(|case| case.ordinal())
        .collect();
    let discovery_case_set = case_ordinals.iter().copied().collect::<BTreeSet<_>>();
    let excluded_replay_cases = prepared
        .agent_inputs()
        .cases()
        .iter()
        .filter(|case| case.phase() == CasePhase::Reproduccion)
        .count() as u64;
    let events = prepared
        .agent_inputs()
        .cases()
        .iter()
        .filter(|case| case.phase() == CasePhase::Arranque)
        .flat_map(|case| case.events())
        .map(|event| {
            Ok(LocalObservedEvent {
                case_ordinal: event.case_ordinal(),
                event_ordinal: event.event_ordinal(),
                parent_event_ordinal: event.parent_event_ordinal(),
                event_time: event.event_time().to_owned(),
                event_kind: safe_code(event.event_kind())?,
                route_code: event.route_code().map(safe_code).transpose()?,
                actor_layer: event.actor_role().map(safe_code).transpose()?,
                tool_code: event.tool_code().map(safe_code).transpose()?,
                technical_error: event.technical_error(),
                retry_count: event.retry_count(),
                approval: event.approval().map(|value| value.to_string()),
                signal_code: event.signal_code().map(safe_code).transpose()?,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let queries = prepared
        .agent_inputs()
        .facts()
        .iter()
        .filter_map(|fact| match fact {
            E0Fact::CopilotQuery {
                case_ordinal,
                event_time,
                query_signature,
                ..
            } if discovery_case_set.contains(case_ordinal) => Some(LocalObservedQuery::new(
                *case_ordinal,
                query_signature.clone(),
                event_time.clone(),
            )),
            _ => None,
        })
        .collect();
    Ok(LocalRunInput::new(
        LocalRunMetadata::new(
            run_id,
            tenant_id,
            source_kind,
            prepared.manifest_digest(),
            prepared.snapshot_ref().clone(),
            prepared.cutoff_unix_seconds(),
            prepared.observed_cutoff(),
        ),
        case_ordinals,
        excluded_replay_cases,
        events,
    )
    .with_queries(queries)
    .with_query_table_available(prepared.agent_inputs().has_available_table("copilot_query")))
}

fn safe_code(value: &str) -> Result<String, String> {
    if value.is_empty()
        || value.len() > 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    {
        return Err("source adapter emitted a non-code value; refusing unsafe output".into());
    }
    Ok(value.to_owned())
}

fn persist_result(
    output: &PathBuf,
    result: &LocalRunResult,
    holdout: Option<&E0HoldoutEvaluation>,
    holdout_event: Option<&RunEvent>,
) -> Result<(), String> {
    let run_dir = output.join(&result.run_id);
    let staging_dir = output.join(format!(".{}.partial", result.run_id));
    fs::create_dir_all(output).map_err(io_message)?;
    fs::create_dir(&staging_dir).map_err(|error| {
        if error.kind() == io::ErrorKind::AlreadyExists {
            "run staging output already exists; refusing to overwrite".to_owned()
        } else {
            io_message(error)
        }
    })?;
    let write_result = (|| {
        let mut result_json = serde_json::to_value(result).map_err(|error| error.to_string())?;
        if result.source_kind == LocalSourceKind::E0 {
            result_json["excluded_replay_case_count"] = serde_json::Value::Null;
        }
        result_json["e0_recurrence_holdout"] = match holdout {
            Some(evaluation) => serde_json::to_value(evaluation),
            None => Ok(serde_json::Value::Null),
        }
        .map_err(|error| error.to_string())?;
        if let Some(event) = holdout_event {
            result_json["events"]
                .as_array_mut()
                .ok_or_else(|| "local result events are not an array".to_owned())?
                .push(serde_json::to_value(event).map_err(|error| error.to_string())?);
        }
        let result_bytes =
            serde_json::to_vec_pretty(&result_json).map_err(|error| error.to_string())?;
        write_new(&staging_dir.join("result.json"), &result_bytes)?;
        let mut ndjson = Vec::new();
        for event in &result.events {
            serde_json::to_writer(&mut ndjson, event).map_err(|error| error.to_string())?;
            ndjson.push(b'\n');
        }
        if let Some(event) = holdout_event {
            serde_json::to_writer(&mut ndjson, event).map_err(|error| error.to_string())?;
            ndjson.push(b'\n');
        }
        write_new(&staging_dir.join("events.ndjson"), &ndjson)?;
        fs::rename(&staging_dir, &run_dir).map_err(io_message)
    })();
    if write_result.is_err() {
        let _ = fs::remove_dir_all(&staging_dir);
    }
    write_result
}

fn write_new(path: &PathBuf, bytes: &[u8]) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(io_message)?;
    file.write_all(bytes).map_err(io_message)?;
    file.sync_all().map_err(io_message)
}

fn make_run_id() -> Result<String, String> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_nanos();
    Ok(format!("run_{}_{}", std::process::id(), nanos))
}

fn io_message(error: io::Error) -> String {
    format!("local filesystem operation failed: {error}")
}

#[derive(Debug, Default)]
struct Options {
    mode: String,
    source: String,
    input: PathBuf,
    output: PathBuf,
    tenant_id: String,
    observed_cutoff: String,
    arranque_cases: usize,
    minimum_recurring_query_support: u64,
    help: bool,
}

impl Options {
    fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Self, String> {
        let mut options = Self {
            mode: "".into(),
            source: "".into(),
            input: PathBuf::new(),
            output: PathBuf::new(),
            tenant_id: "pulso_local".into(),
            observed_cutoff: "".into(),
            arranque_cases: 200,
            minimum_recurring_query_support: 20,
            help: false,
        };
        let mut args = args.into_iter();
        if args.next().as_deref() != Some(std::ffi::OsStr::new("local-sim")) {
            return Err("expected subcommand local-sim (try --help)".into());
        }
        while let Some(arg) = args.next() {
            let key = arg.to_string_lossy();
            if key == "--help" || key == "-h" {
                options.help = true;
                continue;
            }
            let value = args
                .next()
                .ok_or_else(|| format!("missing value for {key}"))?;
            let value = value.to_string_lossy().into_owned();
            match key.as_ref() {
                "--mode" => options.mode = value,
                "--source" => options.source = value,
                "--input" => options.input = value.into(),
                "--output" => options.output = value.into(),
                "--tenant-id" => options.tenant_id = value,
                "--observed-cutoff" => options.observed_cutoff = value,
                "--arranque-cases" => {
                    options.arranque_cases = value
                        .parse()
                        .map_err(|_| "--arranque-cases must be a positive integer".to_owned())?
                }
                "--min-recurring-query-cases" => {
                    options.minimum_recurring_query_support = value.parse().map_err(|_| {
                        "--min-recurring-query-cases must be an integer from 5 to 5000".to_owned()
                    })?;
                    if !(5..=5_000).contains(&options.minimum_recurring_query_support) {
                        return Err(
                            "--min-recurring-query-cases must be an integer from 5 to 5000"
                                .to_owned(),
                        );
                    }
                }
                _ => return Err(format!("unknown option {key}")),
            }
        }
        if !options.help
            && (options.mode.is_empty()
                || options.source.is_empty()
                || options.input.as_os_str().is_empty()
                || options.output.as_os_str().is_empty()
                || options.observed_cutoff.is_empty())
        {
            return Err(
                "required options: --mode --source --input --output --observed-cutoff".into(),
            );
        }
        Ok(options)
    }
}

fn print_help() {
    println!(
        "improvement-engine local-sim --mode local-simulation --source <e0|original> --input <path> --output <dir> [--tenant-id pulso_local] --observed-cutoff <UTC timestamp> [--arranque-cases 200] [--min-recurring-query-cases 20]"
    );
}

fn validate_mode(options: &Options) -> Result<(), String> {
    if options.mode == "local-simulation" {
        Ok(())
    } else {
        Err("only explicit --mode local-simulation is currently supported".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_requires_an_explicit_local_simulation_and_cutoff() {
        let args =
            ["local-sim", "--mode", "local-simulation", "--source", "e0"].map(OsString::from);
        let error = Options::parse(args).unwrap_err();
        assert!(error.contains("--input --output --observed-cutoff"));
    }

    #[test]
    fn cli_rejects_implicit_or_native_execution_modes() {
        let args = [
            "local-sim",
            "--mode",
            "native",
            "--source",
            "e0",
            "--input",
            "x",
            "--output",
            "y",
            "--observed-cutoff",
            "2026-10-02T12:00:00Z",
        ]
        .map(OsString::from);
        let options = Options::parse(args).unwrap();
        assert!(
            validate_mode(&options)
                .unwrap_err()
                .contains("only explicit")
        );
    }

    #[test]
    fn cli_rejects_recurrence_support_below_privacy_floor() {
        let args = [
            "local-sim",
            "--mode",
            "local-simulation",
            "--source",
            "e0",
            "--input",
            "x",
            "--output",
            "y",
            "--observed-cutoff",
            "2026-10-02T12:00:00Z",
            "--min-recurring-query-cases",
            "4",
        ]
        .map(OsString::from);

        assert!(
            Options::parse(args)
                .unwrap_err()
                .contains("integer from 5 to 5000")
        );
    }

    #[test]
    fn safe_code_projection_rejects_arbitrary_text() {
        assert_eq!(safe_code("tool_lookup").unwrap(), "tool_lookup");
        assert!(safe_code("customer says private text").is_err());
    }

    #[test]
    fn persistence_publishes_result_and_timeline_together_and_never_overwrites() {
        let output = env::temp_dir().join(make_run_id().unwrap());
        let result = LocalRunResult {
            run_id: "run_persistence_test".into(),
            tenant_id: "pulso_local".into(),
            source_kind: LocalSourceKind::E0,
            manifest_digest: "sha256:test".into(),
            snapshot_ref: improvement_engine_core::ArtifactReference {
                tenant_id: "pulso_local".into(),
                id: "0199b21c-7eab-7000-8000-000000000001".into(),
                revision: 1,
                digest: "sha256:test".into(),
            },
            observed_cutoff_rfc3339: "2025-07-01T00:00:00Z".into(),
            execution_mode: "local_simulation".into(),
            simulation_version: "test".into(),
            simulation_seed: "test".into(),
            determinism: "deterministic".into(),
            terminal_status: "complete_simulated".into(),
            formal_route: "do_nothing".into(),
            primary_signal_policy: "local_primary_signal_v2".into(),
            recurrence_measurement_status: "observed".into(),
            discovery_case_count: 0,
            excluded_replay_case_count: 0,
            signal: None,
            signals: Vec::new(),
            candidates: Vec::new(),
            verification_status: None,
            proposal: None,
            evaluation: None,
            events: Vec::new(),
        };

        persist_result(&output, &result, None, None).unwrap();
        let run_dir = output.join(&result.run_id);
        assert!(run_dir.join("result.json").is_file());
        assert!(run_dir.join("events.ndjson").is_file());
        assert!(persist_result(&output, &result, None, None).is_err());
        fs::remove_dir_all(output).unwrap();
    }
}
