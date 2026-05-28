use kobo_errors::{DiagnosticLspPayload, KDiagnostic};
use kobo_ir::{FileSet, KoboSpan, ScenarioOpKind, ScenarioProgram};
use kobo_parser::PreprocessSourceMap;
use serde_json::{json, Value};

use crate::actions::{action_commands, code_actions_for_diagnostic};
use crate::protocol::ProtocolDiagnosticFact;
use crate::ranges::{line_range_from_span, original_span_for};

pub fn diagnostic_payload(file_set: &FileSet, diagnostic: &KDiagnostic) -> DiagnosticLspPayload {
    DiagnosticLspPayload::from_diagnostic(file_set, diagnostic)
}

pub fn diagnostic_value(
    file_set: &FileSet,
    diagnostic: &KDiagnostic,
    include_actions: bool,
) -> serde_json::Result<Value> {
    let mut value = serde_json::to_value(diagnostic_payload(file_set, diagnostic))?;
    if include_actions {
        let actions = code_actions_for_diagnostic(diagnostic);
        value["codeAction"] = json!(action_commands(&actions));
        value["codeActions"] = serde_json::to_value(actions)?;
    }
    Ok(value)
}

pub(crate) fn compiler_liveness_diagnostics(
    source: &str,
    program: &ScenarioProgram,
    preprocess_source_map: &PreprocessSourceMap,
) -> Vec<ProtocolDiagnosticFact> {
    program
        .operations
        .iter()
        .filter_map(|operation| match &operation.kind {
            ScenarioOpKind::CreateObligation {
                binding, actions, ..
            } if !compiler_binding_is_resolved(program, binding, actions) => {
                Some(compiler_liveness_diagnostic(
                    source,
                    preprocess_source_map,
                    operation.span,
                    &format!(
                        "unresolved liveness obligation `{binding}` requires {}",
                        actions.join(" | ")
                    ),
                ))
            }
            ScenarioOpKind::BranchUnresolved { binding } => {
                let required_actions = compiler_actions_for_binding(program, binding)
                    .map(|actions| actions.join(" | "))
                    .unwrap_or_else(|| "a terminal action".to_owned());
                Some(compiler_liveness_diagnostic(
                    source,
                    preprocess_source_map,
                    operation.span,
                    &format!(
                        "unresolved liveness obligation `{binding}` on at least one branch requires {required_actions}"
                    ),
                ))
            }
            _ => None,
        })
        .collect()
}

fn compiler_liveness_diagnostic(
    source: &str,
    preprocess_source_map: &PreprocessSourceMap,
    span: KoboSpan,
    message: &str,
) -> ProtocolDiagnosticFact {
    let original_span = original_span_for(preprocess_source_map, span);
    let (line, character_start, character_end) = line_range_from_span(source, original_span);
    ProtocolDiagnosticFact {
        line,
        character_start,
        character_end,
        code: "K0100",
        source: "kobo-compiler",
        fact_source: "compiler-scenario-program",
        message: message.to_owned(),
    }
}

fn compiler_actions_for_binding<'program>(
    program: &'program ScenarioProgram,
    binding: &str,
) -> Option<&'program [String]> {
    program
        .operations
        .iter()
        .find_map(|operation| match &operation.kind {
            ScenarioOpKind::CreateObligation {
                binding: candidate,
                actions,
                ..
            } if candidate == binding => Some(actions.as_slice()),
            _ => None,
        })
}

fn compiler_binding_is_resolved(
    program: &ScenarioProgram,
    binding: &str,
    actions: &[String],
) -> bool {
    program
        .operations
        .iter()
        .any(|operation| match &operation.kind {
            ScenarioOpKind::Discharge {
                binding: candidate,
                action,
            } => candidate == binding && compiler_discharge_resolves_obligation(action, actions),
            ScenarioOpKind::Transfer {
                binding: candidate,
                proven,
                ..
            } => candidate == binding && *proven,
            _ => false,
        })
}

fn compiler_discharge_resolves_obligation(action: &str, actions: &[String]) -> bool {
    actions.iter().any(|expected| expected == action)
        || action == "return"
        || action.starts_with("escape:")
        || action.starts_with("suppressed:")
}

pub(crate) fn parse_recovery_diagnostics(
    source: &str,
    diagnostics: &[KDiagnostic],
    preprocess_source_map: &PreprocessSourceMap,
) -> Vec<ProtocolDiagnosticFact> {
    diagnostics
        .iter()
        .map(|diagnostic| {
            let original_span = original_span_for(preprocess_source_map, diagnostic.primary.span);
            let (line, character_start, character_end) =
                line_range_from_span(source, original_span);
            ProtocolDiagnosticFact {
                line,
                character_start,
                character_end,
                code: diagnostic.code.as_str(),
                source: "kobo-compiler",
                fact_source: "compiler-parse-recovery",
                message: diagnostic
                    .finding
                    .clone()
                    .unwrap_or_else(|| diagnostic.primary.text.clone()),
            }
        })
        .collect()
}
