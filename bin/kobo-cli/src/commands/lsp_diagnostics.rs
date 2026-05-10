use std::path::Path;

use kobo_driver::run_check_pipeline;
use kobo_errors::{DiagDecision, DiagLabel, KDiagnostic, KErrorCode, Severity};
use kobo_ir::{KoboMode, KoboSpan};
use serde_json::json;

use crate::ErrorFormat;

use super::session::build_session;
use super::sim_model::{self, ScenarioFailure, SimulationOptions};

pub(super) fn cmd_lsp_diagnostics(
    file: &Path,
    format: ErrorFormat,
    _no_project_ok: bool,
    include_actions: bool,
) -> anyhow::Result<()> {
    let mut session = build_session(file, Some(KoboMode::Checked))?;
    let _ = run_check_pipeline(&mut session, file);
    let extra_diagnostics = v09_lsp_diagnostics(&session, file)?;

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
        ErrorFormat::Human => {
            anyhow::bail!("lsp-diagnostics currently supports --format=json")
        }
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

fn v09_lsp_diagnostics(
    session: &kobo_driver::CompileSession,
    file: &Path,
) -> anyhow::Result<Vec<KDiagnostic>> {
    let Some((file_id, _)) = session.file_set().iter_files().next() else {
        return Ok(Vec::new());
    };
    let document = sim_model::load_document(file)?;
    if document.scenarios.is_empty() {
        return Ok(Vec::new());
    }
    let run = sim_model::run_quick(
        &document,
        SimulationOptions {
            profile: "checked",
            seed: 0,
            inject: None,
            event_budget: None,
        },
    );

    Ok(run
        .failure
        .iter()
        .map(|failure| diagnostic_from_sim_failure(file_id, &document.source, failure))
        .collect())
}

fn diagnostic_from_sim_failure(
    file_id: kobo_ir::FileId,
    source: &str,
    failure: &ScenarioFailure,
) -> KDiagnostic {
    let start = failure.primary_start.min(source.len()) as u32;
    let mut end = failure.primary_end.min(source.len()) as u32;
    if end < start {
        end = start;
    }
    let span = KoboSpan::new(start, end, file_id);
    let mut diagnostic = KDiagnostic::new(
        failure.code,
        Severity::Error,
        DiagLabel::primary(span, label_for_sim_failure(failure.code)),
        explanation_for_sim_failure(failure),
        decision_for_sim_failure(failure.code),
    );
    diagnostic = match failure.code {
        KErrorCode::K0100 => diagnostic.with_run("kobo replay .kobo/witnesses/<target>.kwit"),
        KErrorCode::K0107 => diagnostic.with_run("kobo test --sim quick"),
        _ => diagnostic.with_run("kobo test --sim quick"),
    };
    diagnostic
}

fn label_for_sim_failure(code: KErrorCode) -> &'static str {
    match code {
        KErrorCode::K0100 => "must_call liveness token can be dropped",
        KErrorCode::K0102 => "raw nondeterminism appears on replay path",
        KErrorCode::K0105 => "simulation event budget exceeded",
        KErrorCode::K0107 => "external replay boundary requires policy",
        _ => "simulation contract failed",
    }
}

fn explanation_for_sim_failure(failure: &ScenarioFailure) -> String {
    match failure.code {
        KErrorCode::K0100 => format!("{}; witness .kobo/witnesses/<target>.kwit", failure.message),
        _ => failure.message.clone(),
    }
}

fn decision_for_sim_failure(code: KErrorCode) -> DiagDecision {
    let decision = match code {
        KErrorCode::K0100 => "discharge the must_call action or replay the .kwit witness",
        KErrorCode::K0102 => "route time/random through the deterministic scenario ward",
        KErrorCode::K0105 => "raise the event budget or remove the unbounded scenario loop",
        KErrorCode::K0107 => "select a boundary policy before exact replay",
        _ => "run kobo test --sim quick for the scenario failure",
    };
    DiagDecision(decision.to_owned())
}
