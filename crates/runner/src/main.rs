use std::collections::BTreeSet;
use std::env;
use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::PathBuf;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use improvement_engine_core::ArtifactReference;
use improvement_engine_core::e0_investigation_plan::build_e0_investigation_plan;
use improvement_engine_core::e0_mechanism_resolution::{
    E0MechanismEvidencePacket, E0RouteCatalog, resolve_e0_mechanism_route,
};
use improvement_engine_core::e0_proposal_assembly::{
    LocalProposalAssembly, LocalProposalAssemblyStatus, assemble_e0_proposals,
};
use improvement_engine_core::local_simulation::{
    LocalContactVolumeCell, LocalContactVolumeProjection, LocalObservedEvent, LocalObservedQuery,
    LocalRunInput, LocalRunMetadata, LocalRunResult, LocalSnapshotContactAggregate,
    LocalSnapshotContactProjection, LocalSourceKind, RunEvent, run_local_simulation,
};
use improvement_engine_source_adapters::{
    CasePhase, E0Fact, E0HoldoutEvaluation, E0HoldoutPolicy, E0HoldoutStatus,
    OriginalPreparationProgress, PreparationConfig, PreparedSource, SourceKind,
    attest_selected_e0_recurrence_candidate, evaluate_e0_recurrence_holdout, prepare_e0_package,
    prepare_original_bank, prepare_original_bank_with_progress,
};

mod e0_builder_input_preparation;
use e0_builder_input_preparation::E0BuilderInputPreparation;

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
    let progress = ProgressReporter::new(options.progress_jsonl);
    let config = PreparationConfig::new(
        options.tenant_id.clone(),
        options.observed_cutoff,
        options.arranque_cases,
    )
    .map_err(|error| format!("invalid source preparation config: {error}"))?;
    let run_id = make_run_id()?;
    progress.stage("source_preparation", "started")?;
    let prepared_result = match options.source.as_str() {
        "e0" => prepare_e0_package(&options.input, &config),
        "original" if options.progress_jsonl => {
            let mut progress_error = None;
            let prepared = prepare_original_bank_with_progress(&options.input, &config, |event| {
                if progress_error.is_none() {
                    progress_error = progress.source_progress(event).err();
                }
            });
            if let Some(error) = progress_error {
                return Err(error);
            }
            prepared
        }
        "original" => prepare_original_bank(&options.input, &config),
        _ => return Err("--source must be e0 or original".into()),
    };
    let prepared = match prepared_result {
        Ok(prepared) => prepared,
        Err(error) => {
            progress.stage("source_preparation", "failed")?;
            return Err(format!("source preparation failed: {error}"));
        }
    };

    let input = match to_run_input(&prepared, &run_id, &options.tenant_id).and_then(|input| {
        input
            .with_minimum_recurring_query_support(options.minimum_recurring_query_support)
            .map_err(|error| format!("invalid recurrence policy: {error:?}"))
    }) {
        Ok(input) => input,
        Err(error) => {
            progress.stage("source_preparation", "failed")?;
            return Err(error);
        }
    };
    // Projection validation is part of preparation; retain both its provenance
    // and the input until the phase is fully ready for detection.
    progress.stage("source_preparation", "completed")?;
    progress.stage("detection", "started")?;
    let result = match run_local_simulation(input) {
        Ok(result) => {
            progress.stage("detection", "completed")?;
            result
        }
        Err(error) => {
            progress.stage("detection", "failed")?;
            return Err(format!("run failed: {error}"));
        }
    };
    progress.stage("post_selection_holdout", "started")?;
    let holdout = match evaluate_selected_recurrence_after_discovery(
        &prepared,
        &result,
        options.minimum_recurring_query_support,
    ) {
        Ok(Some(holdout)) => {
            progress.stage("post_selection_holdout", "completed")?;
            Some(holdout)
        }
        Ok(None) => {
            progress.stage("post_selection_holdout", "skipped")?;
            None
        }
        Err(error) => {
            progress.stage("post_selection_holdout", "failed")?;
            return Err(error);
        }
    };
    let holdout_event = holdout.as_ref().map(|evaluation| {
        make_holdout_event(
            evaluation,
            result.events.len() as u32 + 1,
            &result.observed_cutoff_rfc3339,
        )
    });
    progress.stage("persist_outputs", "started")?;
    let persist = persist_result(
        &options.output,
        &result,
        holdout.as_ref(),
        holdout_event.as_ref(),
    );
    if persist.is_ok() {
        progress.stage("persist_outputs", "completed")?;
    } else {
        progress.stage("persist_outputs", "failed")?;
    }
    persist?;
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

struct ProgressReporter {
    enabled: bool,
    started: Instant,
}

impl ProgressReporter {
    fn new(enabled: bool) -> Self {
        Self {
            enabled,
            started: Instant::now(),
        }
    }

    fn stage(&self, phase: &str, status: &str) -> Result<(), String> {
        if !self.enabled {
            return Ok(());
        }
        let event = serde_json::json!({
            "schema_version": 1,
            "event": "run_progress",
            "phase": phase,
            "status": status,
            "elapsed_ms": self.started.elapsed().as_millis(),
        });
        let mut stderr = io::stderr().lock();
        serde_json::to_writer(&mut stderr, &event)
            .map_err(|_| "progress output failed".to_owned())?;
        stderr
            .write_all(b"\n")
            .map_err(|_| "progress output failed".to_owned())?;
        stderr
            .flush()
            .map_err(|_| "progress output failed".to_owned())
    }

    fn source_progress(&self, progress: OriginalPreparationProgress) -> Result<(), String> {
        if !self.enabled {
            return Ok(());
        }
        let event = serde_json::json!({
            "schema_version": 1,
            "event": "run_progress",
            "phase": "source_preparation",
            "stage": progress.stage,
            "status": progress.status,
            "files_completed": progress.files_completed,
            "files_total": progress.files_total,
            "bytes_completed": progress.bytes_completed,
            "bytes_total": progress.bytes_total,
            "elapsed_ms": self.started.elapsed().as_millis(),
        });
        let mut stderr = io::stderr().lock();
        serde_json::to_writer(&mut stderr, &event)
            .map_err(|_| "progress output failed".to_owned())?;
        stderr
            .write_all(b"\n")
            .map_err(|_| "progress output failed".to_owned())?;
        stderr
            .flush()
            .map_err(|_| "progress output failed".to_owned())
    }
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
    let input = LocalRunInput::new(
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
    .with_query_table_available(prepared.agent_inputs().has_available_table("copilot_query"));
    let mut input = input;
    if let Some(projection) = prepared.agent_inputs().contact_projection() {
        let cells = prepared
            .agent_inputs()
            .contact_volumes()
            .iter()
            .map(|cell| {
                LocalContactVolumeCell::new(
                    cell.reason().to_owned(),
                    cell.channel().to_owned(),
                    cell.record_count(),
                )
            })
            .collect();
        let projection = LocalContactVolumeProjection::new(
            projection.policy_version(),
            projection.minimum_cell_count(),
            projection.included_record_count(),
            cells,
        )
        .map_err(|_| "source adapter emitted an invalid contact projection".to_owned())?;
        input = input.with_contact_volume_projection(projection);
    }
    if let Some(projection) = prepared.agent_inputs().descriptive_contact_projection() {
        let aggregates = projection
            .aggregates()
            .iter()
            .map(|cell| {
                LocalSnapshotContactAggregate::new(
                    cell.period(),
                    cell.reason(),
                    cell.channel(),
                    cell.contact_count(),
                )
            })
            .collect();
        let projection = LocalSnapshotContactProjection::new(
            projection.policy_version(),
            projection.minimum_cell_count(),
            projection.included_contact_count(),
            aggregates,
        )
        .map_err(|_| {
            "source adapter emitted an invalid descriptive contact projection".to_owned()
        })?;
        input = input.with_snapshot_descriptive_contact_projection(projection);
    }
    Ok(input)
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
        let (proposal_event, builder_input_event, mechanism_event, investigation_plan_event) =
            if result.source_kind == LocalSourceKind::E0 {
                result_json["excluded_replay_case_count"] = serde_json::Value::Null;
                let proposal_assembly = assemble_e0_proposals(result).map_err(|_| {
                    "E0 proposal assembly rejected invalid simulator evidence".to_owned()
                })?;
                let status = match proposal_assembly.status {
                    LocalProposalAssemblyStatus::CandidatesReady => "candidates_ready",
                    LocalProposalAssemblyStatus::InsufficientEvidence => "insufficient_evidence",
                    LocalProposalAssemblyStatus::NoQualifyingSignals => "no_qualifying_signals",
                    LocalProposalAssemblyStatus::Unsupported => "unsupported",
                };
                let event = RunEvent {
                    sequence: next_persisted_event_sequence(&result.events, holdout_event)?,
                    stage: "proposal_assembly".to_owned(),
                    status: status.to_owned(),
                    detail: format!(
                        "candidate_count={}; disposition_count={}",
                        proposal_assembly.candidates.len(),
                        proposal_assembly.dispositions.len()
                    ),
                    observed_cutoff_rfc3339: result.observed_cutoff_rfc3339.clone(),
                };
                result_json["proposal_assembly"] = serde_json::to_value(&proposal_assembly)
                    .map_err(|_| "E0 proposal assembly serialization failed".to_owned())?;
                let builder_sequence = increment_event_sequence(event.sequence)?;
                let (builder_input, builder_event) = E0BuilderInputPreparation::from_run(
                    result,
                    &proposal_assembly,
                    builder_sequence,
                )
                .map_err(|_| "E0 builder-input preparation rejected run provenance".to_owned())?;
                result_json["e0_builder_input_preparation"] = serde_json::to_value(&builder_input)
                    .map_err(|_| "E0 builder-input preparation serialization failed".to_owned())?;
                let mechanism = resolve_local_e0_mechanism(result, &proposal_assembly)?;
                let mechanism_event = mechanism
                    .as_ref()
                    .map(|mechanism| make_mechanism_resolution_event(mechanism, &builder_event))
                    .transpose()?;
                if let Some(mechanism) = &mechanism {
                    result_json["e0_mechanism_resolution"] = mechanism.payload.clone();
                    result_json["e0_investigation_proposal_plan"] =
                        mechanism.investigation_plan.clone();
                }
                let investigation_plan_event = mechanism
                    .as_ref()
                    .zip(mechanism_event.as_ref())
                    .map(|(mechanism, event)| {
                        make_investigation_plan_event(&mechanism.investigation_plan, event)
                    })
                    .transpose()?;
                (
                    Some(event),
                    Some(builder_event),
                    mechanism_event,
                    investigation_plan_event,
                )
            } else {
                (None, None, None, None)
            };
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
        if let Some(event) = &proposal_event {
            result_json["events"]
                .as_array_mut()
                .ok_or_else(|| "local result events are not an array".to_owned())?
                .push(serde_json::to_value(event).map_err(|error| error.to_string())?);
        }
        if let Some(event) = &builder_input_event {
            result_json["events"]
                .as_array_mut()
                .ok_or_else(|| "local result events are not an array".to_owned())?
                .push(serde_json::to_value(event).map_err(|error| error.to_string())?);
        }
        if let Some(event) = &mechanism_event {
            result_json["events"]
                .as_array_mut()
                .ok_or_else(|| "local result events are not an array".to_owned())?
                .push(serde_json::to_value(event).map_err(|error| error.to_string())?);
        }
        if let Some(event) = &investigation_plan_event {
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
        if let Some(event) = &proposal_event {
            serde_json::to_writer(&mut ndjson, event).map_err(|error| error.to_string())?;
            ndjson.push(b'\n');
        }
        if let Some(event) = &builder_input_event {
            serde_json::to_writer(&mut ndjson, event).map_err(|error| error.to_string())?;
            ndjson.push(b'\n');
        }
        if let Some(event) = &mechanism_event {
            serde_json::to_writer(&mut ndjson, event).map_err(|error| error.to_string())?;
            ndjson.push(b'\n');
        }
        if let Some(event) = &investigation_plan_event {
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

const LOCAL_E0_EMPTY_CATALOG_ID: &str = "0199b21c-7eab-7000-8000-000000000205";
const LOCAL_E0_EVIDENCE_ORIGIN: &str = "e0_local_run";
const LOCAL_E0_CATALOG_ORIGIN: &str = "team_generated_empty_local_catalog_fixture";
const LOCAL_E0_MECHANISM_DURABILITY: &str = "ephemeral";

struct LocalMechanismResolution {
    payload: serde_json::Value,
    investigation_plan: serde_json::Value,
    status: &'static str,
    reason: Option<&'static str>,
}

fn resolve_local_e0_mechanism(
    result: &LocalRunResult,
    assembly: &LocalProposalAssembly,
) -> Result<Option<LocalMechanismResolution>, String> {
    let Some(candidate) = assembly
        .candidates
        .iter()
        .find(|candidate| candidate.metric_id == "e0_recurring_copilot_query_cases")
    else {
        return Ok(None);
    };
    let signal = result
        .signals
        .iter()
        .find(|signal| {
            signal.metric_id == candidate.metric_id && signal.digest == candidate.signal_digest
        })
        .ok_or_else(|| "E0 mechanism candidate has no corresponding signal".to_owned())?;
    let packet = E0MechanismEvidencePacket::from_candidate(result, candidate, signal)
        .map_err(|_| "E0 mechanism evidence rejected candidate binding".to_owned())?;
    let catalog = E0RouteCatalog::empty(ArtifactReference {
        tenant_id: result.tenant_id.clone(),
        id: LOCAL_E0_EMPTY_CATALOG_ID.to_owned(),
        revision: 1,
        digest: E0RouteCatalog::content_digest(&[]),
    })
    .map_err(|_| "local E0 empty mechanism catalog is invalid".to_owned())?;
    let resolution = resolve_e0_mechanism_route(&packet, &catalog);
    let status = resolution.status();
    let reason = resolution.reason();
    let investigation_plan = build_e0_investigation_plan(candidate, &packet, &resolution)
        .map_err(|_| "E0 investigation plan rejected candidate or route binding".to_owned())?;
    investigation_plan
        .validate_integrity()
        .map_err(|_| "E0 investigation plan failed integrity validation".to_owned())?;
    let investigation_plan = serde_json::to_value(&investigation_plan)
        .map_err(|_| "E0 investigation plan serialization failed".to_owned())?;
    let payload = serde_json::json!({
        "evidence_origin": LOCAL_E0_EVIDENCE_ORIGIN,
        "catalog_origin": LOCAL_E0_CATALOG_ORIGIN,
        "catalog_durability": LOCAL_E0_MECHANISM_DURABILITY,
        "evidence_packet": packet,
        "resolution": resolution,
    });
    Ok(Some(LocalMechanismResolution {
        payload,
        investigation_plan,
        status,
        reason,
    }))
}

fn make_investigation_plan_event(
    plan: &serde_json::Value,
    mechanism_event: &RunEvent,
) -> Result<RunEvent, String> {
    let sequence = increment_event_sequence(mechanism_event.sequence)?;
    let recommendation = match plan["decision"]["recommended_option"].as_str() {
        Some("investigate_mapping") => "investigate_mapping",
        Some("investigate_mapped_flow") => "investigate_mapped_flow",
        _ => return Err("E0 investigation plan has an unsupported recommendation".to_owned()),
    };
    if plan["artifact_kind"] != "e0_read_only_investigation_plan_not_agent_core_proposal"
        || plan["decision"]["execution_state"] != "not_executable"
        || plan["decision"]["authority"] != "none"
    {
        return Err("E0 investigation plan is not read-only".to_owned());
    }
    Ok(RunEvent {
        sequence,
        stage: "e0_investigation_proposal_plan".to_owned(),
        status: "pending_review".to_owned(),
        detail: format!(
            "artifact_kind=e0_read_only_investigation_plan_not_agent_core_proposal; recommendation={recommendation}; execution=not_executable"
        ),
        observed_cutoff_rfc3339: mechanism_event.observed_cutoff_rfc3339.clone(),
    })
}

fn make_mechanism_resolution_event(
    mechanism: &LocalMechanismResolution,
    proposal_event: &RunEvent,
) -> Result<RunEvent, String> {
    let sequence = increment_event_sequence(proposal_event.sequence)?;
    let reason = mechanism
        .reason
        .map_or_else(|| "none".to_owned(), str::to_owned);
    Ok(RunEvent {
        sequence,
        stage: "e0_mechanism_resolution".to_owned(),
        status: mechanism.status.to_owned(),
        detail: format!(
            "catalog_origin={LOCAL_E0_CATALOG_ORIGIN}; catalog_durability={LOCAL_E0_MECHANISM_DURABILITY}; resolution_count=1; mapped_count={}; unlinked_count={}; reason={reason}",
            u8::from(mechanism.status == "mapped"),
            u8::from(mechanism.status == "unlinked")
        ),
        observed_cutoff_rfc3339: proposal_event.observed_cutoff_rfc3339.clone(),
    })
}

fn next_persisted_event_sequence(
    events: &[RunEvent],
    holdout_event: Option<&RunEvent>,
) -> Result<u32, String> {
    let mut expected = 1_u32;
    for event in events {
        if event.sequence != expected {
            return Err("local run event sequence is not contiguous".to_owned());
        }
        expected = increment_event_sequence(expected)?;
    }
    if let Some(event) = holdout_event {
        if event.sequence != expected {
            return Err("holdout event sequence is not contiguous".to_owned());
        }
        expected = increment_event_sequence(expected)?;
    }
    Ok(expected)
}

fn increment_event_sequence(sequence: u32) -> Result<u32, String> {
    sequence
        .checked_add(1)
        .ok_or_else(|| "local run event sequence is exhausted".to_owned())
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
    progress_jsonl: bool,
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
            progress_jsonl: false,
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
            if key == "--progress-jsonl" {
                options.progress_jsonl = true;
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
        "improvement-engine local-sim --mode local-simulation --source <e0|original> --input <path> --output <dir> [--tenant-id pulso_local] --observed-cutoff <UTC timestamp> [--arranque-cases 200] [--min-recurring-query-cases 20] [--progress-jsonl]"
    );
}

fn validate_mode(options: &Options) -> Result<(), String> {
    if options.mode == "local-simulation" {
        match options.source.as_str() {
            "e0" | "original" => Ok(()),
            _ => Err("--source must be e0 or original".into()),
        }
    } else {
        Err("only explicit --mode local-simulation is currently supported".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn timeline_test_event(sequence: u32) -> RunEvent {
        RunEvent {
            sequence,
            stage: "test".to_owned(),
            status: "completed".to_owned(),
            detail: "aggregate-only".to_owned(),
            observed_cutoff_rfc3339: "2025-07-01T00:00:00Z".to_owned(),
        }
    }

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
    fn cli_rejects_contact_suppression_policy_overrides() {
        for requested_minimum in ["4", "10000"] {
            let args = [
                "local-sim",
                "--mode",
                "local-simulation",
                "--source",
                "original",
                "--input",
                "x",
                "--output",
                "y",
                "--observed-cutoff",
                "2026-10-02T12:00:00Z",
                "--min-contact-cell-count",
                requested_minimum,
            ]
            .map(OsString::from);

            assert!(
                Options::parse(args)
                    .unwrap_err()
                    .contains("unknown option --min-contact-cell-count")
            );
        }
    }

    #[test]
    fn persisted_event_sequence_requires_contiguous_values_and_checked_capacity() {
        assert_eq!(next_persisted_event_sequence(&[], None), Ok(1));
        assert_eq!(
            next_persisted_event_sequence(&[timeline_test_event(1)], Some(&timeline_test_event(2))),
            Ok(3)
        );
        assert_eq!(
            next_persisted_event_sequence(&[timeline_test_event(1), timeline_test_event(3)], None),
            Err("local run event sequence is not contiguous".to_owned())
        );
        assert_eq!(
            increment_event_sequence(u32::MAX),
            Err("local run event sequence is exhausted".to_owned())
        );
    }

    #[test]
    fn safe_code_projection_rejects_arbitrary_text() {
        assert_eq!(safe_code("tool_lookup").unwrap(), "tool_lookup");
        assert!(safe_code("customer says private text").is_err());
    }

    #[test]
    fn original_contact_snapshot_counts_reach_local_motor_without_event_claims() {
        let root = env::temp_dir().join(format!("snapshot_{}", make_run_id().unwrap()));
        let table = root.join("call_center_interactions");
        fs::create_dir_all(&table).unwrap();
        fs::write(
            table.join("part-000.csv"),
            concat!(
                "interaction_id,customer_id,interaction_date,contact_reason,channel\n",
                "id-1,c-1,2027-01-01 10:00:00,Complaint,Phone\n",
                "id-2,c-2,2027-01-02 10:00:00,Complaint,Phone\n",
                "id-3,c-3,2027-01-03 10:00:00,Complaint,Phone\n",
                "id-4,c-4,2027-01-04 10:00:00,Complaint,Phone\n",
                "id-5,c-5,2027-01-05 10:00:00,Complaint,Phone\n",
                "id-6,c-6,2027-02-01 10:00:00,Complaint,Phone\n",
                "id-7,c-7,2027-02-02 10:00:00,Complaint,Phone\n",
                "id-8,c-8,2027-02-03 10:00:00,Complaint,Phone\n",
                "id-9,c-9,2027-02-04 10:00:00,Complaint,Phone\n",
                "id-10,c-10,2027-02-05 10:00:00,Complaint,Phone\n",
            ),
        )
        .unwrap();
        let config = PreparationConfig::new("pulso_local", "2025-07-01T00:00:00Z", 10).unwrap();
        let prepared = prepare_original_bank(&root, &config).unwrap();

        let input = to_run_input(&prepared, "run-original-snapshot", "pulso_local").unwrap();
        let result = run_local_simulation(input).unwrap();

        assert_eq!(result.terminal_status, "snapshot_descriptive_finding_ready");
        assert_eq!(result.source_kind, LocalSourceKind::OriginalBank);
        assert!(result.signal.is_none());
        assert!(result.proposal.is_none());
        let envelope = result.snapshot_descriptive_envelope.as_ref().unwrap();
        assert_eq!(
            envelope.agent_core_candidate,
            "dependency_blocked_snapshot_semantics"
        );
        assert_eq!(envelope.finding.literal_months, ["2027-01", "2027-02"]);
        assert_eq!(envelope.finding.complaint_contact_count, 10);
        assert_eq!(envelope.proposal.status, "simulated_unverified");
        assert!(!envelope.proposal.publication_eligible);
        let contact_projection = result.contact_volume_projection.as_ref().unwrap();
        assert_eq!(contact_projection.included_record_count(), 10);
        assert_eq!(contact_projection.cells()[0].record_count(), 10);
        assert!(
            !serde_json::to_string(&result)
                .unwrap()
                .contains("customer_id")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn persistence_publishes_result_and_timeline_together_and_never_overwrites() {
        let output = env::temp_dir().join(format!("persistence_{}", make_run_id().unwrap()));
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
            local_simulation_portfolio: None,
            candidates: Vec::new(),
            verification_status: None,
            proposal: None,
            evaluation: None,
            contact_volume_projection: None,
            snapshot_descriptive_envelope: None,
            events: Vec::new(),
        };

        persist_result(&output, &result, None, None).unwrap();
        let run_dir = output.join(&result.run_id);
        assert!(run_dir.join("result.json").is_file());
        assert!(run_dir.join("events.ndjson").is_file());
        let persisted_result: serde_json::Value =
            serde_json::from_slice(&fs::read(run_dir.join("result.json")).unwrap()).unwrap();
        assert!(persisted_result.get("local_simulation_portfolio").is_none());
        assert_eq!(persisted_result["events"][0]["stage"], "proposal_assembly");
        assert_eq!(persisted_result["events"][0]["status"], "unsupported");
        assert_eq!(
            persisted_result["events"][0]["detail"],
            "candidate_count=0; disposition_count=0"
        );
        let ndjson = fs::read_to_string(run_dir.join("events.ndjson")).unwrap();
        let ndjson_events = ndjson
            .lines()
            .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            ndjson_events,
            persisted_result["events"].as_array().unwrap().clone()
        );
        assert_eq!(
            ndjson_events
                .iter()
                .map(|event| event["stage"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["proposal_assembly", "e0_builder_input_preparation"]
        );
        assert!(persist_result(&output, &result, None, None).is_err());
        fs::remove_dir_all(output).unwrap();
    }
}
