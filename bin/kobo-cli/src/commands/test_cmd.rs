use std::path::{Path, PathBuf};

use anyhow::Context;
use kobo_errors::{
    diagnostic_to_json_value, ColorMode, DiagDecision, DiagLabel, DiagnosticOutputFormat,
    DiagnosticRenderer, KDiagnostic, KErrorCode, Severity,
};
use kobo_ir::{FileSetBuilder, KoboMode, KoboSpan};
use kobo_sim_core::{EngineMode, FullDepthRun, ReplayGuarantee, ScenarioEvent, ScenarioFailure};

use crate::ErrorFormat;

use super::sim_model::{self, ScenarioDocument};

pub(super) fn cmd_test(
    file: &Path,
    sim: Option<&str>,
    profile: Option<&str>,
    seed: Option<u64>,
    events: Option<&str>,
    inject: Option<&str>,
    event_budget: Option<u64>,
    witness_dir: Option<&Path>,
    error_format: ErrorFormat,
    target: Option<&str>,
    engine: Option<&str>,
) -> anyhow::Result<()> {
    match sim {
        Some("quick") => {}
        Some("deep") => {
            anyhow::bail!(
                "kobo test --sim deep is reserved for v0.10 scheduler portfolios; use --sim quick in v0.9"
            );
        }
        Some(other) => {
            anyhow::bail!("kobo test --sim {other} is not available in v0.9; use --sim quick");
        }
        None => {
            anyhow::bail!("kobo test requires --sim quick in v0.9");
        }
    }

    let profile = profile.unwrap_or("checked");
    let seed = seed.unwrap_or(0);
    let engine = parse_engine(engine.unwrap_or("both"))?;
    let document = sim_model::load_document(file)?;
    let target_name = target
        .map(str::to_owned)
        .or_else(|| {
            document
                .scenarios
                .first()
                .map(|scenario| scenario.name.clone())
        })
        .unwrap_or_else(|| "<missing>".to_owned());
    let mut session = super::session::build_session(file, Some(KoboMode::Checked))?;
    let artifacts = kobo_driver::run_codegen_pipeline(&mut session, file)
        .map_err(|()| anyhow::anyhow!("failed to build compiler scenario artifacts"))?;
    let scenario_program = kobo_driver::build_scenario_program(
        &artifacts.kobo_file,
        &artifacts,
        &target_name,
        document.source_hash.clone(),
        profile,
    )?;
    let options = kobo_sim_core::ScenarioOptions {
        profile: profile.to_owned(),
        seed,
        inject: inject.map(str::to_owned),
        event_budget,
    };
    let run = kobo_sim_core::run_full_depth_from_program(
        &scenario_program,
        &artifacts.rs_source,
        &options,
        engine,
    )?;

    let mut witness_path = None;
    if run.failure.is_some() || witness_dir.is_some() {
        witness_path = Some(write_run_witness(
            file,
            &document,
            profile,
            seed,
            witness_dir,
            &run,
        )?);
    }

    if events == Some("json") {
        print_events(file, seed, &run.events)?;
        return Ok(());
    }

    if let Some(failure) = run.failure.as_ref() {
        emit_failure(
            file,
            &document.source,
            failure,
            witness_path.as_deref(),
            error_format,
        )?;
        anyhow::bail!("{}", failure.message)
    }

    println!(
        "{}",
        serde_json::to_string(&serde_json::json!({
            "scenario": run.target,
            "seed": seed,
            "backend_profile": run.profile,
            "status": "passed",
        }))?
    );
    Ok(())
}

fn parse_engine(value: &str) -> anyhow::Result<EngineMode> {
    match value {
        "semantic" => Ok(EngineMode::SemanticOnly),
        "harness" => Ok(EngineMode::HarnessOnly),
        "both" => Ok(EngineMode::Both),
        _ => anyhow::bail!("invalid sim engine; expected semantic, harness, or both"),
    }
}

fn print_events(file: &Path, seed: u64, events: &[ScenarioEvent]) -> anyhow::Result<()> {
    println!(
        "{}",
        serde_json::to_string(&serde_json::json!({
            "schema_version": 1,
            "file": sim_model::cli_relative_path(file)?,
            "seed": seed,
            "events": events_json(events),
        }))?
    );
    Ok(())
}

fn write_run_witness(
    file: &Path,
    document: &ScenarioDocument,
    guarantee_profile: &str,
    seed: u64,
    witness_dir: Option<&Path>,
    run: &FullDepthRun,
) -> anyhow::Result<PathBuf> {
    let directory = witness_directory(file, witness_dir)?;
    std::fs::create_dir_all(&directory)
        .with_context(|| format!("failed to create witness directory {}", directory.display()))?;
    let witness_path = directory.join(format!("{}-{seed}.kwit", sanitize_name(&run.target)));
    let source_path = sim_model::cli_relative_path(file)?;
    let primary_span = run.failure.as_ref().map(|failure| {
        format!(
            "{}:{}:1",
            source_path,
            one_based_line_for_offset(&document.source, failure.primary_start)
        )
    });
    let witness = serde_json::json!({
        "schema_version": 1,
        "kobo_version": env!("CARGO_PKG_VERSION"),
        "target": format!("{}:{}", source_path, run.target),
        "scenario": {
            "name": run.target,
            "profile": run.profile,
        },
        "source": {
            "path": source_path,
            "hash": document.source_hash,
        },
        "guarantee_profile": guarantee_profile,
        "expanded_policy": expanded_policy_json(guarantee_profile),
        "seed": seed,
        "backend_profile": run.profile,
        "backend": backend_for_profile(&run.profile),
        "backend_replay": replay_token(&document.source_hash, seed, run),
        "execution_digest": execution_digest_json(run),
        "harness_manifest": run.harness_manifest.clone(),
        "coverage": coverage_json(run),
        "replay_guarantee": run.replay_guarantee.as_str(),
        "modeled_boundaries": modeled_boundaries_json(run),
        "opaque_boundaries": run.opaque_boundaries.clone(),
        "boundary_assumptions": boundary_assumptions_json(run),
        "obligations": obligations_json(&source_path, &document.source, run),
        "boundary_decisions": boundary_decisions_json(run),
        "available_boundary_policies": ["model", "record", "stub", "outside", "opaque", "debt"],
        "failure": failure_json(&source_path, &document.source, run, primary_span),
        "events": events_json(&run.events),
    });
    std::fs::write(&witness_path, serde_json::to_string_pretty(&witness)?)
        .with_context(|| format!("failed to write {}", witness_path.display()))?;
    Ok(witness_path)
}

fn execution_digest_json(run: &FullDepthRun) -> serde_json::Value {
    serde_json::json!({
        "engine": "semantic-sim",
        "semantic_engine": run.digest.semantic_engine,
        "harness_engine": run.digest.harness_engine,
        "model_version": run.digest.model_version,
        "scenario_ir_hash": run.digest.scenario_ir_hash,
        "operation_count": run.digest.operation_count,
        "event_hash": run.digest.semantic_trace_hash,
        "semantic_trace_hash": run.digest.semantic_trace_hash,
        "harness_trace_hash": run.digest.harness_trace_hash,
        "agreement": run.digest.agreement,
        "generated_rust_hash": run.digest.generated_rust_hash,
        "harness_manifest_hash": run.digest.harness_manifest_hash,
        "harness_exit_code": run.digest.harness_exit_code,
        "harness_event_count": run.digest.harness_event_count,
    })
}

fn coverage_json(run: &FullDepthRun) -> serde_json::Value {
    serde_json::json!({
        "unsupported_constructs": run.coverage.unsupported_constructs,
        "reason": run.coverage.reason,
    })
}

fn witness_directory(file: &Path, witness_dir: Option<&Path>) -> anyhow::Result<PathBuf> {
    match witness_dir {
        Some(directory) if directory.is_absolute() => Ok(directory.to_path_buf()),
        Some(directory) => Ok(std::env::current_dir()
            .context("failed to determine current directory")?
            .join(directory)),
        None => Ok(file
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."))),
    }
}

fn emit_failure(
    file: &Path,
    source: &str,
    failure: &ScenarioFailure,
    witness_path: Option<&Path>,
    error_format: ErrorFormat,
) -> anyhow::Result<()> {
    let mut files = FileSetBuilder::new();
    let file_id = files.add_file(file.to_path_buf(), source.to_owned());
    let span = KoboSpan::new(
        failure.primary_start as u32,
        failure.primary_end.max(failure.primary_start + 1) as u32,
        file_id,
    );
    let message = match witness_path {
        Some(path) => format!("{}; witness {}", failure.message, path.display()),
        None => failure.message.clone(),
    };
    let diagnostic = KDiagnostic::new(
        failure.code,
        Severity::Error,
        DiagLabel::primary(span, message.clone()),
        scenario_failure_explanation(failure),
        DiagDecision(scenario_failure_decision(failure)),
    )
    .with_finding(scenario_failure_finding(failure, witness_path));

    match error_format {
        ErrorFormat::Json => {
            let mut value = diagnostic_to_json_value(files.as_file_set(), &diagnostic);
            if let Some(entry) = files.as_file_set().get(file_id) {
                value["line"] = serde_json::json!(entry.line_col(span.start).0);
            }
            println!("{}", serde_json::to_string(&value)?);
        }
        ErrorFormat::Human => {
            let color = super::session::resolve_color_mode(ColorMode::Auto);
            let renderer = DiagnosticRenderer::new(color, DiagnosticOutputFormat::HumanCard);
            eprintln!("{}", renderer.render(files.as_file_set(), &diagnostic));
        }
    }

    Ok(())
}

fn scenario_failure_finding(failure: &ScenarioFailure, witness_path: Option<&Path>) -> String {
    let finding = match failure.code {
        KErrorCode::K0100 => {
            let binding = scenario_failure_label(failure).unwrap_or("the value");
            format!(
                "A checked scenario can drop `{binding}` before calling one of its required actions."
            )
        }
        KErrorCode::K0102 => {
            "This replay path uses raw time, randomness, file IO, or task scheduling.".to_owned()
        }
        KErrorCode::K0103 => {
            "This replay path performs an effect Kobo cannot replay deterministically.".to_owned()
        }
        KErrorCode::K0105 => {
            "This scenario exceeded the quick simulation budget before it finished.".to_owned()
        }
        KErrorCode::K0107 => {
            let boundary = scenario_failure_label(failure).unwrap_or("external code");
            format!("This replay path crosses `{boundary}` without a boundary policy.")
        }
        KErrorCode::K0116 => {
            "This scenario uses syntax Kobo has not modeled for exact replay yet.".to_owned()
        }
        KErrorCode::K0117 => {
            "The compiler semantic trace and generated harness trace do not agree.".to_owned()
        }
        KErrorCode::K0118 => {
            "The action points at an artifact that does not match the current source.".to_owned()
        }
        _ => failure.message.clone(),
    };

    match witness_path {
        Some(path) => format!("{finding} Witness: {}", path.display()),
        None => finding,
    }
}

fn scenario_failure_explanation(failure: &ScenarioFailure) -> String {
    match failure.code {
        KErrorCode::K0100 => {
            "Kobo tracks must_call values as obligations. Every return, cancellation, or replayed path must finish the obligation before the value leaves scope.".to_owned()
        }
        KErrorCode::K0102 => {
            "Replay evidence only stays useful when the same inputs produce the same event stream. Raw nondeterminism can make a replay pass or fail for the wrong reason.".to_owned()
        }
        KErrorCode::K0103 => {
            "An uncontrolled effect can change outside Kobo's replay model. The scenario needs a model, a recording, or an explicit debt boundary before Kobo can trust the replay.".to_owned()
        }
        KErrorCode::K0105 => {
            "The quick profile is for small, fast evidence. A scenario that exceeds its budget needs to be shrunk or moved to a slower profile.".to_owned()
        }
        KErrorCode::K0107 => {
            "External code can perform IO, scheduling, time, randomness, or other effects that Kobo cannot infer from the source alone. The boundary policy says what replay may assume.".to_owned()
        }
        KErrorCode::K0116 => {
            "Exact replay is only sound for modeled syntax. Kobo found a construct outside the current modeled island coverage.".to_owned()
        }
        KErrorCode::K0117 => {
            "Full-depth replay needs two independent traces to match: the compiler semantic trace and the generated harness trace.".to_owned()
        }
        KErrorCode::K0118 => {
            "Editor and replay actions must point at artifacts created from the current source hash and target.".to_owned()
        }
        _ => failure.message.clone(),
    }
}

fn scenario_failure_decision(failure: &ScenarioFailure) -> String {
    match failure.code {
        KErrorCode::K0100 => {
            let actions = scenario_failure_actions(&failure.message)
                .unwrap_or_else(|| "one required action".to_owned());
            format!(
                "Call {actions} on every path, or record explicit debt if cleanup happens outside this scenario."
            )
        }
        KErrorCode::K0102 => {
            "Route time, randomness, file IO, and task scheduling through modeled facades before claiming replay evidence.".to_owned()
        }
        KErrorCode::K0103 => {
            "Model the effect, record the effect stream, move it outside replay, or mark replay debt explicitly.".to_owned()
        }
        KErrorCode::K0105 => {
            "Reduce the scenario, split it into smaller scenarios, or run it under a profile with a larger budget.".to_owned()
        }
        KErrorCode::K0107 => {
            "Choose model, record, stub, outside, opaque, or debt for this boundary before claiming exact replay.".to_owned()
        }
        KErrorCode::K0116 => {
            "Use a modeled construct, split the scenario, or keep the witness partial until coverage is implemented.".to_owned()
        }
        KErrorCode::K0117 => {
            "Regenerate the witness with --engine both and investigate any trace mismatch before replaying it.".to_owned()
        }
        KErrorCode::K0118 => {
            "Regenerate the witness/session artifacts for the current source before using the action.".to_owned()
        }
        _ => "Make replay evidence deterministic and explicit.".to_owned(),
    }
}

fn scenario_failure_label(failure: &ScenarioFailure) -> Option<&str> {
    failure
        .events
        .iter()
        .find_map(|event| event.label.as_deref())
}

fn scenario_failure_actions(message: &str) -> Option<String> {
    let actions = message.split("discharge with ").nth(1)?;
    Some(actions.trim_end_matches('.').to_owned())
}

fn modeled_boundaries_json(run: &FullDepthRun) -> Vec<&'static str> {
    run.modeled_boundaries
        .iter()
        .map(|boundary| boundary.as_str())
        .collect()
}

fn obligations_json(source_path: &str, source: &str, run: &FullDepthRun) -> Vec<serde_json::Value> {
    run.obligations
        .iter()
        .map(|obligation| {
            serde_json::json!({
                "binding": obligation.binding,
                "type": obligation.type_name,
                "actions": obligation.actions,
                "discharged": obligation.is_discharged,
                "declaration_span": span_json(source_path, source, obligation.declaration_span),
                "drop_span": obligation.drop_span.map(|span| span_json(source_path, source, span)),
            })
        })
        .collect()
}

fn expanded_policy_json(profile: &str) -> serde_json::Value {
    let (ownership, liveness, replay, boundaries, errors) = match profile {
        "dev" => ("record", "record", "record", "record", "ergonomic"),
        "release" => ("strict", "checked", "checked", "strict", "explicit"),
        _ => ("checked", "checked", "checked", "checked", "typed"),
    };
    serde_json::json!({
        "profile": profile,
        "ownership": ownership,
        "liveness": liveness,
        "replay": replay,
        "boundaries": boundaries,
        "errors": errors,
    })
}

fn boundary_assumptions_json(run: &FullDepthRun) -> Vec<serde_json::Value> {
    let mut assumptions = run
        .boundary_decisions
        .iter()
        .map(|decision| {
            serde_json::json!({
                "boundary": decision.crate_name,
                "policy": decision.policy.as_str(),
                "reason": decision.reason,
                "replay_effect": if run.replay_guarantee == ReplayGuarantee::Exact {
                    "modeled"
                } else {
                    run.replay_guarantee.as_str()
                },
            })
        })
        .collect::<Vec<_>>();

    if run.replay_guarantee == ReplayGuarantee::NotReplayable && assumptions.is_empty() {
        let boundary = run
            .failure
            .as_ref()
            .and_then(|failure| failure.events.first())
            .and_then(|event| event.label.clone())
            .unwrap_or_else(|| "uncontrolled".to_owned());
        let reason = run
            .failure
            .as_ref()
            .map(|failure| failure.message.clone())
            .unwrap_or_else(|| "not replayable".to_owned());
        assumptions.push(serde_json::json!({
            "boundary": boundary,
            "policy": "debt",
            "reason": reason,
            "replay_effect": run.replay_guarantee.as_str(),
        }));
    }

    assumptions
}

fn failure_json(
    source_path: &str,
    source: &str,
    run: &FullDepthRun,
    primary_span: Option<String>,
) -> serde_json::Value {
    let Some(failure) = run.failure.as_ref() else {
        return serde_json::Value::Null;
    };
    serde_json::json!({
        "code": failure.code.as_str(),
        "message": failure.message,
        "primary_span": primary_span.unwrap_or_else(|| format!("{source_path}:1:1")),
        "related_spans": related_spans_json(source_path, source, run, failure),
    })
}

fn related_spans_json(
    source_path: &str,
    source: &str,
    run: &FullDepthRun,
    failure: &ScenarioFailure,
) -> Vec<serde_json::Value> {
    if failure.code != KErrorCode::K0100 {
        return Vec::new();
    }
    run.obligations
        .iter()
        .filter(|obligation| !obligation.is_discharged)
        .map(|obligation| {
            serde_json::json!({
                "label": format!("obligation `{}` declared here", obligation.binding),
                "span": span_json(source_path, source, obligation.declaration_span),
            })
        })
        .collect()
}

fn span_json(source_path: &str, source: &str, span: (usize, usize)) -> serde_json::Value {
    serde_json::json!({
        "path": source_path,
        "line": one_based_line_for_offset(source, span.0),
        "start": span.0,
        "end": span.1.max(span.0 + 1),
    })
}

fn boundary_decisions_json(run: &FullDepthRun) -> Vec<serde_json::Value> {
    run.boundary_decisions
        .iter()
        .map(|decision| {
            serde_json::json!({
                "crate": decision.crate_name,
                "policy": decision.policy.as_str(),
                "reason": decision.reason,
            })
        })
        .collect()
}

fn events_json(events: &[ScenarioEvent]) -> Vec<serde_json::Value> {
    events
        .iter()
        .map(|event| {
            serde_json::json!({
                "kind": event.kind,
                "label": event.label,
                "value": event.value,
            })
        })
        .collect()
}

fn replay_token(source_identity: &str, seed: u64, run: &FullDepthRun) -> String {
    let mut material = String::new();
    material.push_str(source_identity);
    material.push(':');
    material.push_str(&seed.to_string());
    material.push(':');
    material.push_str(backend_for_profile(&run.profile));
    material.push(':');
    material.push_str(&run.digest.semantic_trace_hash);
    material.push(':');
    material.push_str(&run.digest.harness_trace_hash);
    kobo_sim_core::digest::stable_hash(&material)
}

fn backend_for_profile(profile: &str) -> &'static str {
    match profile {
        "sync" => "loom",
        "stateful-input" => "proptest",
        "failpoint" => "failpoints",
        "network" | "network-design" => "network-design",
        _ => "shuttle",
    }
}

fn sanitize_name(value: &str) -> String {
    value
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect()
}

fn one_based_line_for_offset(source: &str, offset: usize) -> usize {
    source[..offset.min(source.len())]
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count()
        + 1
}
