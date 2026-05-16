use std::path::{Path, PathBuf};

use anyhow::Context;
use kobo_driver::run_check_pipeline;
use kobo_errors::{DiagDecision, DiagLabel, KDiagnostic, KErrorCode, Severity};
use kobo_ir::{GuaranteePolicy, GuaranteeProfile, KoboSpan};
use serde_json::{json, Value};

use crate::ErrorFormat;

use super::session::build_session;
pub(super) fn cmd_lsp_diagnostics(
    file: &Path,
    format: ErrorFormat,
    _no_project_ok: bool,
    include_actions: bool,
) -> anyhow::Result<()> {
    let mut session = build_session(
        file,
        Some(GuaranteePolicy::for_profile(GuaranteeProfile::Checked)),
    )?;
    let _ = run_check_pipeline(&mut session, file);
    let mut extra_diagnostics = artifact_backed_diagnostics(&session, file)?;
    extra_diagnostics.extend(replay_boundary_diagnostics(&session, file)?);

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

fn replay_boundary_diagnostics(
    session: &kobo_driver::CompileSession,
    file: &Path,
) -> anyhow::Result<Vec<KDiagnostic>> {
    if session
        .visible_diagnostics()
        .any(|diagnostic| diagnostic.code == KErrorCode::K0107)
    {
        return Ok(Vec::new());
    }
    let source = std::fs::read_to_string(file)?;
    let Some(boundary) = external_replay_boundary(&source) else {
        return Ok(Vec::new());
    };
    let Some((file_id, _)) = session.file_set().iter_files().next() else {
        return Ok(Vec::new());
    };
    let span = KoboSpan::new(
        boundary.span_start as u32,
        boundary.span_end as u32,
        file_id,
    );
    Ok(vec![KDiagnostic::new(
        KErrorCode::K0107,
        Severity::Warning,
        DiagLabel::primary(
            span,
            format!(
                "unmodeled external boundary `{}`; choose model, record, stub, outside, opaque, or debt",
                boundary.crate_name
            ),
        ),
        format!(
            "unmodeled external boundary `{}`; choose model, record, stub, outside, opaque, or debt. Available policies: model, record, stub, outside, opaque, debt.",
            boundary.crate_name
        ),
        DiagDecision("select an explicit boundary policy for replay-critical evidence".to_owned()),
    )])
}

struct ExternalReplayBoundary {
    crate_name: String,
    span_start: usize,
    span_end: usize,
}

fn external_replay_boundary(source: &str) -> Option<ExternalReplayBoundary> {
    direct_client_boundary(source).or_else(|| imported_client_boundary(source))
}

fn direct_client_boundary(source: &str) -> Option<ExternalReplayBoundary> {
    let mut line_offset = 0usize;
    for line in source.lines() {
        if let Some(separator) = line.find("::Client::new") {
            let prefix = &line[..separator];
            let path_start = prefix
                .char_indices()
                .rev()
                .find(|(_, ch)| !(ch.is_ascii_alphanumeric() || *ch == '_' || *ch == ':'))
                .map(|(index, _)| index + 1)
                .unwrap_or(0);
            let path = &prefix[path_start..];
            let crate_name = path.split("::").next().unwrap_or(path).to_owned();
            return Some(ExternalReplayBoundary {
                span_start: line_offset + path_start,
                span_end: line_offset + path_start + crate_name.len(),
                crate_name,
            });
        }
        line_offset += line.len() + 1;
    }
    None
}

fn imported_client_boundary(source: &str) -> Option<ExternalReplayBoundary> {
    if !source.contains("Client::new") {
        return None;
    }
    let mut line_offset = 0usize;
    for line in source.lines() {
        let Some(rest) = line.trim_start().strip_prefix("use ") else {
            line_offset += line.len() + 1;
            continue;
        };
        let Some(separator) = rest.find("::Client") else {
            line_offset += line.len() + 1;
            continue;
        };
        let path = rest[..separator].trim();
        let crate_name = path.split("::").next().unwrap_or(path).to_owned();
        let local_start = line.find(&crate_name).unwrap_or(0);
        return Some(ExternalReplayBoundary {
            span_start: line_offset + local_start,
            span_end: line_offset + local_start + crate_name.len(),
            crate_name,
        });
    }
    None
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
    Ok(witnesses
        .into_iter()
        .filter_map(|artifact| diagnostic_from_witness(file_id, entry.source(), artifact).ok())
        .collect())
}

struct WitnessArtifact {
    path: PathBuf,
    json: Value,
}

fn matching_witnesses(
    source_path: &str,
    source_hash: &str,
) -> anyhow::Result<Vec<WitnessArtifact>> {
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
        if json["schema_version"].as_u64() != Some(1) {
            continue;
        }
        if json["replay_guarantee"].as_str() == Some("exact")
            && json["execution_digest"]["agreement"].as_str() != Some("matched")
        {
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

fn primary_span_from_witness(file_id: kobo_ir::FileId, source: &str, witness: &Value) -> KoboSpan {
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
