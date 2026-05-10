use std::path::{Path, PathBuf};

use anyhow::Context;
use kobo_errors::{
    diagnostic_to_json_value, ColorMode, DiagDecision, DiagLabel, DiagnosticOutputFormat,
    DiagnosticRenderer, KDiagnostic, KErrorCode, Severity,
};
use kobo_ir::{FileSetBuilder, KoboSpan};

use crate::ErrorFormat;

use super::sim_model::{
    self, ScenarioDocument, ScenarioFailure, SimEvent, SimulationOptions, SimulationRun,
};

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum ReplayGuarantee {
    Exact,
    Partial,
    NotReplayable,
}

impl ReplayGuarantee {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::Partial => "partial",
            Self::NotReplayable => "not_replayable",
        }
    }
}

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
    let document = sim_model::load_document(file)?;
    let run = sim_model::run_quick(
        &document,
        SimulationOptions {
            profile,
            seed,
            inject,
            event_budget,
        },
    );

    if events == Some("json") {
        print_events(file, seed, &run.events)?;
        return Ok(());
    }

    let Some(failure) = run.failure.as_ref() else {
        println!(
            "{}",
            serde_json::to_string(&serde_json::json!({
                "scenario": run.scenario.name,
                "seed": seed,
                "backend_profile": run.scenario.profile,
                "status": "passed",
            }))?
        );
        return Ok(());
    };

    let witness_path = Some(write_failure_witness(
        file,
        &document,
        profile,
        seed,
        witness_dir,
        &run,
        failure,
    )?);
    emit_failure(
        file,
        &document.source,
        failure,
        witness_path.as_deref(),
        error_format,
    )?;
    anyhow::bail!("{}", failure.message)
}

fn print_events(file: &Path, seed: u64, events: &[SimEvent]) -> anyhow::Result<()> {
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

fn write_failure_witness(
    file: &Path,
    document: &ScenarioDocument,
    guarantee_profile: &str,
    seed: u64,
    witness_dir: Option<&Path>,
    run: &SimulationRun,
    failure: &ScenarioFailure,
) -> anyhow::Result<PathBuf> {
    let directory = witness_directory(file, witness_dir)?;
    std::fs::create_dir_all(&directory)
        .with_context(|| format!("failed to create witness directory {}", directory.display()))?;
    let witness_path = directory.join(format!("{}-{seed}.kwit", sanitize_name(&run.scenario.name)));
    let source_path = sim_model::cli_relative_path(file)?;
    let primary_span = format!(
        "{}:{}:1",
        source_path,
        one_based_line_for_offset(&document.source, failure.primary_start)
    );
    let replay_guarantee = replay_guarantee_for(run, failure);
    let witness = serde_json::json!({
        "schema_version": 1,
        "kobo_version": env!("CARGO_PKG_VERSION"),
        "target": format!("{}:{}", source_path, run.scenario.name),
        "scenario": {
            "name": run.scenario.name,
            "profile": run.scenario.profile,
        },
        "source": {
            "path": source_path,
            "hash": document.source_hash,
        },
        "guarantee_profile": guarantee_profile,
        "expanded_policy": expanded_policy_json(guarantee_profile),
        "seed": seed,
        "backend_profile": run.scenario.profile,
        "backend": run.backend.as_str(),
        "backend_replay": sim_model::replay_token(&document.source_hash, seed, run),
        "replay_guarantee": replay_guarantee.as_str(),
        "modeled_boundaries": modeled_boundaries_json(run),
        "opaque_boundaries": run.opaque_boundaries.clone(),
        "boundary_assumptions": boundary_assumptions_json(run, failure, replay_guarantee),
        "obligations": obligations_json(&source_path, &document.source, run),
        "boundary_decisions": boundary_decisions_json(run),
        "available_boundary_policies": sim_model::boundary_policy_choices()
            .iter()
            .map(|choice| choice.as_str())
            .collect::<Vec<_>>(),
        "failure": {
            "code": failure.code.as_str(),
            "message": failure.message.clone(),
            "primary_span": primary_span,
            "related_spans": related_spans_json(&source_path, &document.source, run, failure),
        },
        "events": events_json(&run.events),
    });
    std::fs::write(&witness_path, serde_json::to_string_pretty(&witness)?)
        .with_context(|| format!("failed to write {}", witness_path.display()))?;
    Ok(witness_path)
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
        message,
        DiagDecision("make replay evidence deterministic and explicit".to_owned()),
    );

    match error_format {
        ErrorFormat::Json => {
            let mut value = diagnostic_to_json_value(files.as_file_set(), &diagnostic);
            if let Some(entry) = files.as_file_set().get(file_id) {
                value["line"] = serde_json::json!(entry.line_col(span.start).0);
            }
            println!("{}", serde_json::to_string(&value)?);
        }
        ErrorFormat::Human => {
            let color = if std::env::var_os("NO_COLOR").is_some() {
                ColorMode::Never
            } else {
                ColorMode::Auto
            };
            let renderer = DiagnosticRenderer::new(color, DiagnosticOutputFormat::HumanCard);
            eprintln!("{}", renderer.render(files.as_file_set(), &diagnostic));
        }
    }

    Ok(())
}

fn modeled_boundaries_json(run: &SimulationRun) -> Vec<&'static str> {
    run.modeled_boundaries
        .iter()
        .map(|boundary| boundary.as_str())
        .collect()
}

fn obligations_json(
    source_path: &str,
    source: &str,
    run: &SimulationRun,
) -> Vec<serde_json::Value> {
    run.obligations
        .iter()
        .map(|obligation| {
            serde_json::json!({
                "binding": obligation.binding.clone(),
                "type": obligation.type_name.clone(),
                "actions": obligation.actions.clone(),
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

fn replay_guarantee_for(run: &SimulationRun, failure: &ScenarioFailure) -> ReplayGuarantee {
    if matches!(failure.code, KErrorCode::K0102 | KErrorCode::K0103) {
        return ReplayGuarantee::NotReplayable;
    }
    if run.opaque_boundaries.is_empty() {
        ReplayGuarantee::Exact
    } else {
        ReplayGuarantee::Partial
    }
}

fn boundary_assumptions_json(
    run: &SimulationRun,
    failure: &ScenarioFailure,
    replay_guarantee: ReplayGuarantee,
) -> Vec<serde_json::Value> {
    let mut assumptions = run
        .boundary_decisions
        .iter()
        .map(|decision| {
            serde_json::json!({
                "boundary": decision.crate_name.clone(),
                "policy": decision.policy.as_str(),
                "reason": decision.reason.clone(),
                "replay_effect": if replay_guarantee == ReplayGuarantee::Exact {
                    "modeled"
                } else {
                    replay_guarantee.as_str()
                },
            })
        })
        .collect::<Vec<_>>();

    if replay_guarantee == ReplayGuarantee::NotReplayable && assumptions.is_empty() {
        let boundary = failure
            .events
            .first()
            .and_then(|event| event.label.clone())
            .unwrap_or_else(|| failure.code.as_str().to_owned());
        assumptions.push(serde_json::json!({
            "boundary": boundary,
            "policy": "debt",
            "reason": failure.message,
            "replay_effect": replay_guarantee.as_str(),
        }));
    }

    assumptions
}

fn related_spans_json(
    source_path: &str,
    source: &str,
    run: &SimulationRun,
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

fn boundary_decisions_json(run: &SimulationRun) -> Vec<serde_json::Value> {
    run.boundary_decisions
        .iter()
        .map(|decision| {
            serde_json::json!({
                "crate": decision.crate_name.clone(),
                "policy": decision.policy.as_str(),
                "reason": decision.reason.clone(),
            })
        })
        .collect()
}

fn events_json(events: &[SimEvent]) -> Vec<serde_json::Value> {
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
