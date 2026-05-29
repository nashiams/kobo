use kobo_codegen::{KoboSourceMap, RsSpan};
use kobo_errors::{
    CliSuggestion, DiagDecision, DiagExplanation, DiagLabel, DiagLabelKind, KDiagnostic,
    KErrorCode, Severity,
};
use kobo_ir::{
    FileId, KoboSpan, OwnershipDebtCode, OwnershipDebtKind, OwnershipDebtRecord,
    OwnershipDebtSeverity,
};

use super::json::{parse_rustc_diagnostics, RustcJsonError, RustcSpan};

pub(crate) fn remap_rustc_output(
    raw_output: &str,
    source_map: &KoboSourceMap,
    kobo_file_id: FileId,
) -> Vec<KDiagnostic> {
    let parsed_output = parse_rustc_diagnostics(raw_output);
    if parsed_output.parsed_any {
        return parsed_output
            .errors
            .into_iter()
            .map(|error| remap_error(error, source_map, kobo_file_id))
            .collect();
    }

    vec![unparsed_output_diagnostic(
        raw_output.trim(),
        source_map,
        kobo_file_id,
    )]
}

pub(crate) fn unparsed_output_diagnostic(
    raw_output: &str,
    source_map: &KoboSourceMap,
    file_id: FileId,
) -> KDiagnostic {
    KDiagnostic::new(
        KErrorCode::K0099,
        Severity::Error,
        DiagLabel::primary(
            KoboSpan::new(0, 0, file_id),
            "compiler output could not be remapped",
        ),
        DiagExplanation(format!("[remapping unavailable] {raw_output}")),
        DiagDecision(
            "rustc output did not match Kobo's JSON remapper; surfaced a Tier-3 envelope"
                .to_owned(),
        ),
    )
    .with_run(run_suggestion(source_map))
}

fn remap_error(
    error: RustcJsonError,
    source_map: &KoboSourceMap,
    kobo_file_id: FileId,
) -> KDiagnostic {
    remap_diagnostic(
        error,
        source_map,
        kobo_file_id,
        Severity::Error,
        KErrorCode::K0099,
    )
}

/// Re-map a surviving rustc warning to a .kobo-span diagnostic.
/// Uses K0019 (uncategorized ownership) with Warning severity so that
/// `has_errors()` remains false and exit code stays 0.
pub(crate) fn remap_warning_diagnostic(
    warning: RustcJsonError,
    source_map: &KoboSourceMap,
    kobo_file_id: FileId,
) -> KDiagnostic {
    remap_diagnostic(
        warning,
        source_map,
        kobo_file_id,
        Severity::Warning,
        KErrorCode::K0019,
    )
}

/// Core diagnostic re-mapper — shared by error and warning paths.
/// `severity` and `code` determine how the output diagnostic is classified.
fn remap_diagnostic(
    diag: RustcJsonError,
    source_map: &KoboSourceMap,
    kobo_file_id: FileId,
    severity: Severity,
    code: KErrorCode,
) -> KDiagnostic {
    let remapped = remap_labels(&diag, source_map, kobo_file_id);
    let ownership_escape = rustc_ownership_code(&diag);
    if remapped.mapped_label_count == 0 {
        // Unmappable: surface as tier-3 envelope but preserve severity/code.
        return tier_three_diagnostic_with_severity(
            &diag,
            source_map,
            kobo_file_id,
            remapped.help.as_deref(),
            severity,
            code,
        );
    }

    let decision_text = if let Some(rustc_code) = ownership_escape {
        format!(
            "ownership error escaped Kobo analysis ({rustc_code}); record this as ownership debt or teach Kobo this source shape"
        )
    } else if severity == Severity::Error {
        "rustc rejected generated output; Kobo remapped the spans back to .kobo source".to_owned()
    } else {
        "rustc warning in generated output; Kobo remapped the spans back to .kobo source".to_owned()
    };

    let mut diagnostic = KDiagnostic::new(
        code,
        severity,
        remapped.primary,
        remap_explanation(&diag, remapped.remapping_unavailable),
        DiagDecision(decision_text),
    );
    let ownership_escape_debt = ownership_escape
        .map(|rustc_code| rustc_escape_debt_record(rustc_code, &diag, diagnostic.primary.span));

    for label in remapped.secondary {
        diagnostic = diagnostic.with_secondary_label(label);
    }

    if let Some(record) = ownership_escape_debt {
        let help_text = match remapped.help {
            Some(help) => format!("{}; rustc help: {help}", record.hint),
            None => record.hint.clone(),
        };
        diagnostic = diagnostic
            .with_help(help_text)
            .with_hint(record.hint.clone())
            .with_ownership_debt(record);
    } else if let Some(help_text) = remapped.help {
        diagnostic = diagnostic.with_help(help_text);
    }

    diagnostic.with_run(run_suggestion(source_map))
}

fn rustc_ownership_code(error: &RustcJsonError) -> Option<&str> {
    let code = error.code.as_ref()?.code.as_str();
    if matches!(
        code,
        "E0382" | "E0499" | "E0502" | "E0505" | "E0507" | "E0515"
    ) {
        Some(code)
    } else {
        None
    }
}

fn rustc_escape_debt_record(
    rustc_code: &str,
    error: &RustcJsonError,
    span: KoboSpan,
) -> OwnershipDebtRecord {
    OwnershipDebtRecord {
        code: OwnershipDebtCode::K0099,
        severity: OwnershipDebtSeverity::Error,
        kind: OwnershipDebtKind::RustcEscape,
        span,
        binding_name: "generated Rust".to_owned(),
        message: format!(
            "ownership error escaped Kobo analysis ({rustc_code}): {}",
            error.message
        ),
        hint: format!(
            "ownership debt: fix the source ownership shape before generated Rust reaches Rust ({rustc_code})"
        ),
    }
}

struct RemappedLabels {
    primary: DiagLabel,
    secondary: Vec<DiagLabel>,
    help: Option<String>,
    mapped_label_count: usize,
    remapping_unavailable: bool,
}

fn remap_labels(
    error: &RustcJsonError,
    source_map: &KoboSourceMap,
    kobo_file_id: FileId,
) -> RemappedLabels {
    let primary_span_result = remap_span_collection(&error.spans, source_map, kobo_file_id);
    let child_result = remap_child_messages(&error.children, source_map, kobo_file_id);
    let mut primary = primary_span_result.primary;
    let mut secondary = primary_span_result.secondary;
    secondary.extend(child_result.secondary);
    let mut remapping_unavailable =
        primary_span_result.remapping_unavailable || child_result.remapping_unavailable;

    let primary = primary.take().unwrap_or_else(|| {
        remapping_unavailable = true;
        fallback_primary(error.message.clone(), kobo_file_id)
    });

    RemappedLabels {
        primary,
        secondary,
        help: child_result.help,
        mapped_label_count: primary_span_result.mapped_label_count
            + child_result.mapped_label_count,
        remapping_unavailable,
    }
}

struct SpanCollectionRemap {
    primary: Option<DiagLabel>,
    secondary: Vec<DiagLabel>,
    mapped_label_count: usize,
    remapping_unavailable: bool,
}

fn remap_span_collection(
    spans: &[RustcSpan],
    source_map: &KoboSourceMap,
    kobo_file_id: FileId,
) -> SpanCollectionRemap {
    let mut primary = None;
    let mut secondary = Vec::new();
    let mut mapped_label_count = 0;
    let mut remapping_unavailable = false;

    for span in spans {
        let (label, used_fallback) = remap_span(span, source_map, kobo_file_id);
        remapping_unavailable |= used_fallback;
        mapped_label_count += usize::from(!used_fallback);

        if span.is_primary && primary.is_none() {
            primary = Some(label);
        } else {
            secondary.push(label);
        }
    }

    SpanCollectionRemap {
        primary,
        secondary,
        mapped_label_count,
        remapping_unavailable,
    }
}

struct ChildMessageRemap {
    secondary: Vec<DiagLabel>,
    help: Option<String>,
    mapped_label_count: usize,
    remapping_unavailable: bool,
}

fn remap_child_messages(
    children: &[RustcJsonError],
    source_map: &KoboSourceMap,
    kobo_file_id: FileId,
) -> ChildMessageRemap {
    let mut secondary = Vec::new();
    let mut help = None;
    let mut mapped_label_count = 0;
    let mut remapping_unavailable = false;

    for child in children {
        if child.level == "help" {
            help = Some(child.message.clone());
        }

        if child.level != "note" {
            continue;
        }

        let child_result = remap_span_collection(&child.spans, source_map, kobo_file_id);
        if let Some(primary) = child_result.primary {
            secondary.push(as_secondary_label(primary));
        }
        secondary.extend(child_result.secondary);
        mapped_label_count += child_result.mapped_label_count;
        remapping_unavailable |= child_result.remapping_unavailable;
    }

    ChildMessageRemap {
        secondary,
        help,
        mapped_label_count,
        remapping_unavailable,
    }
}

fn remap_span(
    span: &RustcSpan,
    source_map: &KoboSourceMap,
    kobo_file_id: FileId,
) -> (DiagLabel, bool) {
    let rs_span = RsSpan {
        line: span.line_start,
        column_start: span.column_start,
        column_end: span.column_end.max(span.column_start),
    };
    let label_kind = span_label_kind(span);
    let label_text = span_label_text(span);

    let Some(kobo_span) = source_map.lookup_kobo_span(rs_span) else {
        let fallback_text = format!(
            "{label_text} (generated line {}, col {})",
            span.line_start, span.column_start
        );
        return (
            DiagLabel::new(KoboSpan::new(0, 0, kobo_file_id), label_kind, fallback_text),
            true,
        );
    };

    (DiagLabel::new(kobo_span, label_kind, label_text), false)
}

fn span_label_kind(span: &RustcSpan) -> DiagLabelKind {
    if span.is_primary {
        DiagLabelKind::Primary
    } else {
        DiagLabelKind::Secondary
    }
}

fn span_label_text(span: &RustcSpan) -> String {
    span.label
        .clone()
        .unwrap_or_else(|| "rustc span".to_owned())
}

fn fallback_primary(text: String, file_id: FileId) -> DiagLabel {
    DiagLabel::primary(KoboSpan::new(0, 0, file_id), text)
}

fn as_secondary_label(label: DiagLabel) -> DiagLabel {
    DiagLabel::new(label.span, DiagLabelKind::Secondary, label.text)
}

fn remap_explanation(error: &RustcJsonError, remapping_unavailable: bool) -> DiagExplanation {
    let explanation_prefix = if remapping_unavailable {
        "[remapping unavailable] "
    } else {
        ""
    };
    let rustc_code = error
        .code
        .as_ref()
        .map(|code| format!(" ({})", code.code))
        .unwrap_or_default();

    if let Some(code) = rustc_ownership_code(error) {
        DiagExplanation(format!(
            "{explanation_prefix}ownership debt escaped Kobo analysis: {} ({code})",
            error.message
        ))
    } else {
        DiagExplanation(format!(
            "{explanation_prefix}{}{}",
            error.message, rustc_code
        ))
    }
}

fn tier_three_diagnostic_with_severity(
    error: &RustcJsonError,
    source_map: &KoboSourceMap,
    file_id: FileId,
    help: Option<&str>,
    severity: Severity,
    code: KErrorCode,
) -> KDiagnostic {
    let primary_text = if let Some(span) = error.spans.first() {
        format!(
            "compiler output could not be remapped (generated line {}, col {})",
            span.line_start, span.column_start
        )
    } else {
        "compiler output could not be remapped".to_owned()
    };

    let ownership_escape = rustc_ownership_code(error);
    let decision = if let Some(rustc_code) = ownership_escape {
        format!(
            "ownership error escaped Kobo analysis ({rustc_code}); surfaced a Tier-3 ownership debt envelope"
        )
    } else {
        "rustc output did not map back to Kobo spans; surfaced a Tier-3 envelope".to_owned()
    };

    let mut diagnostic = KDiagnostic::new(
        code,
        severity,
        DiagLabel::primary(KoboSpan::new(0, 0, file_id), primary_text),
        remap_explanation(error, true),
        DiagDecision(decision),
    );

    if let Some(rustc_code) = ownership_escape {
        let record = rustc_escape_debt_record(rustc_code, error, KoboSpan::new(0, 0, file_id));
        let help_text = match help {
            Some(help) => format!("{}; rustc help: {help}", record.hint),
            None => record.hint.clone(),
        };
        diagnostic = diagnostic
            .with_help(help_text)
            .with_hint(record.hint.clone())
            .with_ownership_debt(record);
    } else if let Some(help_text) = help {
        diagnostic = diagnostic.with_help(help_text.to_owned());
    }

    diagnostic.with_run(run_suggestion(source_map))
}

fn run_suggestion(source_map: &KoboSourceMap) -> CliSuggestion {
    CliSuggestion(format!("kobo inspect {}", source_map.kobo_path()))
}

#[cfg(test)]
mod remap_tests;
