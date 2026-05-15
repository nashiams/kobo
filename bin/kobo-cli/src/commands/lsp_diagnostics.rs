use std::path::{Path, PathBuf};

use anyhow::Context;
use kobo_driver::run_check_pipeline;
use kobo_errors::{DiagDecision, DiagLabel, KDiagnostic, KErrorCode, Severity};
use kobo_ir::{KoboMode, KoboSpan};
use serde_json::{json, Value};

use crate::ErrorFormat;

use super::session::build_session;
use super::sim_model;

pub(super) fn cmd_lsp_diagnostics(
    file: &Path,
    format: ErrorFormat,
    _no_project_ok: bool,
    include_actions: bool,
) -> anyhow::Result<()> {
    let mut session = build_session(file, Some(KoboMode::Checked))?;
    let _ = run_check_pipeline(&mut session, file);
    let extra_diagnostics = artifact_backed_diagnostics(&session, file)?;

    match format {
        ErrorFormat::Json => {
            for diagnostic in session.visible_diagnostics() {
                print_lsp_payload(&session, diagnostic, include_actions)?;
            }
            for diagnostic in &extra_diagnostics {
                print_lsp_payload(&session, diagnostic, include_actions)?;
            }
            Ok(())
        }
        ErrorFormat::Human => anyhow::bail!("lsp-diagnostics currently supports --format=json"),
    }
}

fn print_lsp_payload(
    session: &kobo_driver::CompileSession,
    diagnostic: &KDiagnostic,
    include_actions: bool,
) -> anyhow::Result<()> {
    let mut value = kobo_lsp::diagnostic_value(session.file_set(), diagnostic, include_actions)?;
    if let Some(line) = session
        .file_set()
        .get(diagnostic.primary.span.file_id)
        .map(|file| file.line_col(diagnostic.primary.span.start).0)
    {
        value["line"] = json!(line);
    }
    println!("{}", serde_json::to_string(&value)?);
    Ok(())
}

fn artifact_backed_diagnostics(
    session: &kobo_driver::CompileSession,
    file: &Path,
) -> anyhow::Result<Vec<KDiagnostic>> {
    let Some((file_id, entry)) = session.file_set().iter_files().next() else {
        return Ok(Vec::new());
    };
    let source_hash = kobo_sim_core::digest::stable_hash(entry.source());
    let source_path = cli_relative_path(file)?;
    let witnesses = matching_witnesses(&source_path, &source_hash)?;
    if witnesses.is_empty() {
        return fallback_semantic_diagnostics(file_id, entry.source(), file);
    }
    Ok(witnesses
        .into_iter()
        .filter_map(|artifact| diagnostic_from_witness(file_id, entry.source(), artifact).ok())
        .collect())
}

fn fallback_semantic_diagnostics(
    file_id: kobo_ir::FileId,
    source: &str,
    file: &Path,
) -> anyhow::Result<Vec<KDiagnostic>> {
    let document = sim_model::parse_document(source.to_owned());
    let Some(target) = document.scenarios.first().map(|scenario| scenario.name.as_str()) else {
        return Ok(Vec::new());
    };
    let run = kobo_sim_core::run_full_depth(
        source,
        target,
        kobo_sim_core::EngineMode::SemanticOnly,
        kobo_sim_core::ScenarioOptions {
            profile: "checked".to_owned(),
            seed: 0,
            inject: None,
            event_budget: None,
        },
    )?;
    let Some(failure) = run.failure.as_ref() else {
        return Ok(Vec::new());
    };
    let span = KoboSpan::new(
        failure.primary_start as u32,
        failure.primary_end.max(failure.primary_start + 1) as u32,
        file_id,
    );
    let witness_path = std::env::current_dir()
        .context("failed to determine current directory")?
        .join(".kobo")
        .join("witnesses")
        .join(format!("{}-0.kwit", sanitize_witness_name(target)));
    let diagnostic = KDiagnostic::new(
        failure.code,
        Severity::Error,
        DiagLabel::primary(span, label_for_code(failure.code)),
        failure.message.clone(),
        DiagDecision(decision_for_code(failure.code).to_owned()),
    )
    .with_run(format!("kobo replay {}", witness_path.display()));
    let _ = file;
    Ok(vec![diagnostic])
}

struct WitnessArtifact {
    path: PathBuf,
    json: Value,
}

fn matching_witnesses(source_path: &str, source_hash: &str) -> anyhow::Result<Vec<WitnessArtifact>> {
    let witness_dir = std::env::current_dir()
        .context("failed to determine current directory")?
        .join(".kobo")
        .join("witnesses");
    let Ok(entries) = std::fs::read_dir(&witness_dir) else {
        return Ok(Vec::new());
    };
    let mut matches = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("kwit") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(json) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        if json["source"]["hash"].as_str() != Some(source_hash) {
            continue;
        }
        if json["source"]["path"].as_str() != Some(source_path) {
            continue;
        }
        matches.push(WitnessArtifact { path, json });
    }
    matches.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(matches)
}

fn diagnostic_from_witness(
    file_id: kobo_ir::FileId,
    source: &str,
    artifact: WitnessArtifact,
) -> anyhow::Result<KDiagnostic> {
    let code = parse_code(artifact.json["failure"]["code"].as_str().unwrap_or("K0104"))
        .unwrap_or(KErrorCode::K0104);
    let span = primary_span_from_witness(file_id, source, &artifact.json);
    let message = artifact.json["failure"]["message"]
        .as_str()
        .unwrap_or("simulation evidence failed");
    Ok(KDiagnostic::new(
        code,
        Severity::Error,
        DiagLabel::primary(span, label_for_code(code)),
        message.to_owned(),
        DiagDecision(decision_for_code(code).to_owned()),
    )
    .with_run(format!("kobo replay {}", artifact.path.display())))
}

fn primary_span_from_witness(
    file_id: kobo_ir::FileId,
    source: &str,
    witness: &Value,
) -> KoboSpan {
    if let Some(start) = witness["obligations"]
        .as_array()
        .and_then(|items| items.first())
        .and_then(|item| item["declaration_span"]["start"].as_u64())
    {
        let end = witness["obligations"]
            .as_array()
            .and_then(|items| items.first())
            .and_then(|item| item["declaration_span"]["end"].as_u64())
            .unwrap_or(start + 1);
        return KoboSpan::new(start as u32, end.max(start + 1) as u32, file_id);
    }
    KoboSpan::new(0, source.len().min(1) as u32, file_id)
}

fn label_for_code(code: KErrorCode) -> &'static str {
    match code {
        KErrorCode::K0100 => "must_call liveness token can be dropped",
        KErrorCode::K0102 => "raw nondeterminism appears on replay path",
        KErrorCode::K0105 => "simulation event budget exceeded",
        KErrorCode::K0107 => "external replay boundary requires policy",
        KErrorCode::K0116 => "scenario coverage is incomplete",
        KErrorCode::K0117 => "semantic and harness traces diverged",
        KErrorCode::K0118 => "evidence artifact is stale",
        _ => "simulation contract failed",
    }
}

fn decision_for_code(code: KErrorCode) -> &'static str {
    match code {
        KErrorCode::K0100 => "discharge the must_call action or replay the .kwit witness",
        KErrorCode::K0102 => "route time/random through the deterministic scenario ward",
        KErrorCode::K0105 => "raise the event budget or remove the unbounded scenario loop",
        KErrorCode::K0107 => "select a boundary policy before exact replay",
        KErrorCode::K0116 => "keep the witness partial until the scenario coverage is modeled",
        KErrorCode::K0117 => "regenerate the witness and investigate the trace mismatch",
        KErrorCode::K0118 => "regenerate artifacts for the current source hash",
        _ => "run kobo test --sim quick for the scenario failure",
    }
}

fn parse_code(code: &str) -> Option<KErrorCode> {
    KErrorCode::ALL
        .iter()
        .copied()
        .find(|candidate| candidate.as_str() == code)
}

fn cli_relative_path(file: &Path) -> anyhow::Result<String> {
    let absolute = if file.is_absolute() {
        file.to_path_buf()
    } else {
        std::env::current_dir()
            .context("failed to determine current directory")?
            .join(file)
    };
    let cwd = std::env::current_dir().context("failed to determine current directory")?;
    let display_path = absolute.strip_prefix(&cwd).unwrap_or(&absolute);
    Ok(display_path
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/"))
}

fn sanitize_witness_name(value: &str) -> String {
    value
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect()
}
