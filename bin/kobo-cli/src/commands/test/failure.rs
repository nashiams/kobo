use super::{
    diagnostic_to_json_value, serde_json, ColorMode, DiagDecision, DiagLabel,
    DiagnosticOutputFormat, DiagnosticRenderer, ErrorFormat, FileSetBuilder, KDiagnostic,
    KErrorCode, KoboSpan, Path, ScenarioFailure, Severity,
};
use crate::commands::session;

pub(super) fn emit_failure(
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
            let color = session::resolve_color_mode(ColorMode::Auto);
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
        KErrorCode::K0108 if scenario_failure_has_event(failure, "invariant-failure") => {
            "An invariant check failed against the recorded scenario trace.".to_owned()
        }
        KErrorCode::K0108 if scenario_failure_has_event(failure, "temporal-failure") => {
            "A temporal trace check failed against the recorded scenario trace.".to_owned()
        }
        KErrorCode::K0117 if scenario_failure_has_event(failure, "model-trace-divergence") => {
            "The ward model and implementation produced different event traces.".to_owned()
        }
        KErrorCode::K0117 if scenario_failure_has_event(failure, "model-obligation-divergence") => {
            "The ward model and implementation disagree about obligation state.".to_owned()
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
        KErrorCode::K0108 if scenario_failure_has_event(failure, "invariant-failure") => {
            "Invariant checks are evaluated over the same event stream written into the witness, so the failure is a counterexample trace, not a replay mismatch.".to_owned()
        }
        KErrorCode::K0108 if scenario_failure_has_event(failure, "temporal-failure") => {
            "Temporal checks are evaluated over ordered witness events with stable event names. Missing or forbidden events are reported separately from replay mismatch diagnostics.".to_owned()
        }
        KErrorCode::K0117 if scenario_failure_has_event(failure, "model-trace-divergence") => {
            "Model-vs-implementation comparison uses the same scheduler seed and compares ordered witness events, so a different first event is real comparison evidence.".to_owned()
        }
        KErrorCode::K0117
            if scenario_failure_has_event(failure, "model-obligation-divergence") =>
        {
            "Model-vs-implementation comparison includes liveness obligation state, not just return values.".to_owned()
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
            "Choose a typed/model policy, record it, wrap it as an activity, or keep this path partial with outside, opaque, or debt before claiming exact replay.".to_owned()
        }
        KErrorCode::K0108 if scenario_failure_has_event(failure, "invariant-failure") => {
            "Inspect the witness invariant_checks trace excerpt, then update the model or scenario so the invariant holds.".to_owned()
        }
        KErrorCode::K0108 if scenario_failure_has_event(failure, "temporal-failure") => {
            "Inspect the witness temporal_checks trace excerpt, then update the model, expected event name, or scenario ordering.".to_owned()
        }
        KErrorCode::K0117 if scenario_failure_has_event(failure, "model-trace-divergence") => {
            "Inspect model_vs_implementation.trace.first_difference, then align the ward model event or the implementation behavior.".to_owned()
        }
        KErrorCode::K0117
            if scenario_failure_has_event(failure, "model-obligation-divergence") =>
        {
            "Inspect model_vs_implementation.obligations.first_difference, then align the model state or discharge path.".to_owned()
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

pub(super) fn scenario_failure_has_event(failure: &ScenarioFailure, kind: &str) -> bool {
    failure.events.iter().any(|event| event.kind == kind)
}

fn scenario_failure_actions(message: &str) -> Option<String> {
    let actions = message.split("discharge with ").nth(1)?;
    Some(actions.trim_end_matches('.').to_owned())
}
