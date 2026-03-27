use kobo_codegen::{KoboSourceMap, RsSpan};
use kobo_errors::{
    CliSuggestion, DiagDecision, DiagExplanation, DiagLabel, DiagLabelKind, KDiagnostic,
    KErrorCode, Severity,
};
use kobo_ir::{FileId, KoboSpan};

use super::json::{parse_rustc_errors, RustcJsonError, RustcSpan};

pub(crate) fn remap_rustc_output(
    raw_output: &str,
    source_map: &KoboSourceMap,
    kobo_file_id: FileId,
) -> Vec<KDiagnostic> {
    let parsed_output = parse_rustc_errors(raw_output);
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

fn remap_error(error: RustcJsonError, source_map: &KoboSourceMap, kobo_file_id: FileId) -> KDiagnostic {
    let remapped = remap_labels(&error, source_map, kobo_file_id);
    if remapped.mapped_label_count == 0 {
        return tier_three_diagnostic(&error, source_map, kobo_file_id, remapped.help.as_deref());
    }

    let mut diagnostic = KDiagnostic::new(
        KErrorCode::K0099,
        Severity::Error,
        remapped.primary,
        remap_explanation(&error, remapped.remapping_unavailable),
        DiagDecision("rustc rejected generated output; Kobo remapped the spans back to .kobo source".to_owned()),
    );

    for label in remapped.secondary {
        diagnostic = diagnostic.with_secondary_label(label);
    }

    if let Some(help_text) = remapped.help {
        diagnostic = diagnostic.with_help(help_text);
    }

    diagnostic.with_run(run_suggestion(source_map))
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
    let primary_span_result =
        remap_span_collection(&error.spans, source_map, kobo_file_id);
    let child_result =
        remap_child_messages(&error.children, source_map, kobo_file_id);
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
        mapped_label_count: primary_span_result.mapped_label_count + child_result.mapped_label_count,
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
        return (
            DiagLabel::new(KoboSpan::new(0, 0, kobo_file_id), label_kind, label_text),
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

    DiagExplanation(format!(
        "{explanation_prefix}{}{}",
        error.message, rustc_code
    ))
}

fn tier_three_diagnostic(
    error: &RustcJsonError,
    source_map: &KoboSourceMap,
    file_id: FileId,
    help: Option<&str>,
) -> KDiagnostic {
    let mut diagnostic = KDiagnostic::new(
        KErrorCode::K0099,
        Severity::Error,
        DiagLabel::primary(
            KoboSpan::new(0, 0, file_id),
            "compiler output could not be remapped",
        ),
        remap_explanation(error, true),
        DiagDecision(
            "rustc output did not map back to Kobo spans; surfaced a Tier-3 envelope".to_owned(),
        ),
    );

    if let Some(help_text) = help {
        diagnostic = diagnostic.with_help(help_text.to_owned());
    }

    diagnostic.with_run(run_suggestion(source_map))
}

fn run_suggestion(source_map: &KoboSourceMap) -> CliSuggestion {
    CliSuggestion(format!("kobo inspect {}", source_map.kobo_path()))
}

#[cfg(test)]
mod tests {
    use kobo_codegen::{KoboSourceMap, RsSpan, SourceMapEntry};
    use kobo_ir::{FileId, KoboSpan};

    use super::remap_rustc_output;

    fn sample_map() -> KoboSourceMap {
        KoboSourceMap {
            version: 3,
            file: "src/main.rs".to_owned(),
            sources: vec!["src/main.kobo".to_owned()],
            x_kobo_mappings: vec![
                SourceMapEntry {
                    rs_span: RsSpan {
                        line: 2,
                        column_start: 0,
                        column_end: 20,
                    },
                    kobo_span: KoboSpan::new(10, 16, FileId(0)),
                    ownership_tier: "rc_refcell".to_owned(),
                },
                SourceMapEntry {
                    rs_span: RsSpan {
                        line: 3,
                        column_start: 0,
                        column_end: 20,
                    },
                    kobo_span: KoboSpan::new(20, 26, FileId(0)),
                    ownership_tier: "rc_refcell".to_owned(),
                },
            ],
        }
    }

    #[test]
    fn remapper_handles_multi_span_errors() {
        let raw = r#"{"message":"cannot borrow","code":{"code":"E0502"},"level":"error","spans":[{"file_name":"src/main.rs","line_start":2,"column_start":1,"line_end":2,"column_end":5,"is_primary":false,"label":"immutable borrow occurs here"},{"file_name":"src/main.rs","line_start":3,"column_start":1,"line_end":3,"column_end":5,"is_primary":true,"label":"mutable borrow occurs here"}],"children":[]}"#;
        let diagnostics = remap_rustc_output(raw, &sample_map(), FileId(0));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].secondary.len(), 1);
        assert_eq!(diagnostics[0].run.as_ref().map(|run| run.0.as_str()), Some("kobo inspect src/main.kobo"));
    }

    #[test]
    fn remapper_handles_single_span_errors() {
        let raw = r#"{"message":"type mismatch","code":{"code":"E0308"},"level":"error","spans":[{"file_name":"src/main.rs","line_start":2,"column_start":1,"line_end":2,"column_end":5,"is_primary":true,"label":"expected type"}],"children":[]}"#;
        let diagnostics = remap_rustc_output(raw, &sample_map(), FileId(0));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].primary.text, "expected type");
        assert!(diagnostics[0].secondary.is_empty());
    }

    #[test]
    fn remapper_falls_back_when_no_spans_map() {
        let raw = r#"{"message":"type mismatch","code":null,"level":"error","spans":[{"file_name":"src/main.rs","line_start":40,"column_start":1,"line_end":40,"column_end":5,"is_primary":true,"label":"expected type"}],"children":[]}"#;
        let diagnostics = remap_rustc_output(raw, &sample_map(), FileId(0));

        assert!(diagnostics[0].explanation.0.starts_with("[remapping unavailable]"));
        assert_eq!(diagnostics[0].primary.span, KoboSpan::new(0, 0, FileId(0)));
        assert_eq!(diagnostics[0].primary.text, "compiler output could not be remapped");
    }

    #[test]
    fn remapper_preserves_help_children() {
        let raw = r#"{"message":"type mismatch","code":null,"level":"error","spans":[{"file_name":"src/main.rs","line_start":2,"column_start":1,"line_end":2,"column_end":5,"is_primary":true,"label":"expected type"}],"children":[{"message":"consider borrowing","code":null,"level":"help","spans":[],"children":[]}]}"#;
        let diagnostics = remap_rustc_output(raw, &sample_map(), FileId(0));

        assert_eq!(
            diagnostics[0].help.as_ref().map(|help| help.0.as_str()),
            Some("consider borrowing")
        );
    }

    #[test]
    fn remapper_preserves_note_child_spans() {
        let raw = r#"{"message":"cannot borrow","code":{"code":"E0502"},"level":"error","spans":[{"file_name":"src/main.rs","line_start":3,"column_start":1,"line_end":3,"column_end":5,"is_primary":true,"label":"mutable borrow occurs here"}],"children":[{"message":"immutable borrow later used here","code":null,"level":"note","spans":[{"file_name":"src/main.rs","line_start":2,"column_start":1,"line_end":2,"column_end":5,"is_primary":true,"label":"immutable borrow occurs here"}],"children":[]}]}"#;
        let diagnostics = remap_rustc_output(raw, &sample_map(), FileId(0));

        assert_eq!(diagnostics[0].secondary.len(), 1);
        assert_eq!(diagnostics[0].secondary[0].text, "immutable borrow occurs here");
    }

    #[test]
    fn remapper_handles_partial_remapping() {
        let raw = r#"{"message":"cannot borrow","code":{"code":"E0502"},"level":"error","spans":[{"file_name":"src/main.rs","line_start":2,"column_start":1,"line_end":2,"column_end":5,"is_primary":false,"label":"immutable borrow occurs here"},{"file_name":"src/main.rs","line_start":40,"column_start":1,"line_end":40,"column_end":5,"is_primary":true,"label":"mutable borrow occurs here"}],"children":[]}"#;
        let diagnostics = remap_rustc_output(raw, &sample_map(), FileId(0));

        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics[0].explanation.0.starts_with("[remapping unavailable]"));
        assert_eq!(diagnostics[0].secondary.len(), 1);
    }

    #[test]
    fn remapper_does_not_leak_rs_paths_in_fallback_labels() {
        let raw = r#"{"message":"type mismatch","code":null,"level":"error","spans":[{"file_name":"src/generated.rs","line_start":40,"column_start":1,"line_end":40,"column_end":5,"is_primary":true,"label":null}],"children":[]}"#;
        let diagnostics = remap_rustc_output(raw, &sample_map(), FileId(0));

        assert!(!diagnostics[0].primary.text.contains(".rs"));
    }
}
