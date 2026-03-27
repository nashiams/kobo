use kobo_ir::{FileEntry, FileSet};

use crate::diagnostic::{CliSuggestion, DiagHelp, DiagLabel, DiagLabelKind, KDiagnostic};

pub fn format_diagnostic(file_set: &FileSet, diagnostic: &KDiagnostic) -> String {
    let mut rendered = String::new();
    rendered.push_str(&format!(
        "{}[{}]: {}",
        diagnostic.severity,
        diagnostic.code,
        diagnostic.code.metadata().short_description
    ));
    rendered.push('\n');
    rendered.push_str(&render_label_group(file_set, diagnostic));
    rendered.push('\n');
    rendered.push_str("   = ");
    rendered.push_str(&diagnostic.explanation.0);
    rendered.push('\n');
    rendered.push_str("   = kobo decision: ");
    rendered.push_str(&diagnostic.decision.0);

    if let Some(help) = diagnostic.help.as_ref() {
        push_help(&mut rendered, help);
    }

    if let Some(run) = diagnostic.run.as_ref() {
        push_run(&mut rendered, run);
    }

    rendered
}

fn render_label_group(file_set: &FileSet, diagnostic: &KDiagnostic) -> String {
    let Some(file) = file_set.get(diagnostic.primary.span.file_id) else {
        return render_individual_blocks(file_set, diagnostic);
    };

    let labels = ordered_labels(diagnostic);
    if labels
        .iter()
        .any(|label| label.span.file_id != diagnostic.primary.span.file_id || !label.span.is_valid_for(file))
    {
        return render_individual_blocks(file_set, diagnostic);
    }

    render_grouped_block(file, &labels, &diagnostic.primary)
}

fn ordered_labels<'a>(diagnostic: &'a KDiagnostic) -> Vec<&'a DiagLabel> {
    let mut labels = Vec::with_capacity(diagnostic.secondary.len() + 1);
    labels.extend(diagnostic.secondary.iter());
    labels.push(&diagnostic.primary);
    labels.sort_by_key(|label| label.span.start);
    labels
}

fn render_grouped_block(file: &FileEntry, labels: &[&DiagLabel], primary: &DiagLabel) -> String {
    let (primary_line, primary_column) = file.line_col(primary.span.start);
    let gutter_width = labels
        .iter()
        .map(|label| file.line_col(label.span.start).0)
        .max()
        .unwrap_or(primary_line)
        .to_string()
        .len();
    let mut rendered = String::new();
    rendered.push_str(&format!(
        "  --> {}:{primary_line}:{primary_column}",
        file.path.display()
    ));
    rendered.push('\n');
    rendered.push_str(&format!("{:>gutter_width$} |", ""));

    for label in labels {
        let (line, column) = file.line_col(label.span.start);
        let line_text = file.line_text(line).unwrap_or_default();
        let caret_padding = " ".repeat(column.saturating_sub(1));
        let caret = marker_for_kind(label.kind).repeat(marker_width(file, label));

        rendered.push('\n');
        rendered.push_str(&format!("{line:>gutter_width$} | {line_text}"));
        rendered.push('\n');
        rendered.push_str(&format!("{:>gutter_width$} | {}{}", "", caret_padding, caret));
        if !label.text.is_empty() {
            rendered.push(' ');
            rendered.push_str(&label.text);
        }
    }

    rendered.push('\n');
    rendered.push_str(&format!("{:>gutter_width$} |", ""));
    rendered
}

fn render_individual_blocks(file_set: &FileSet, diagnostic: &KDiagnostic) -> String {
    let mut rendered = render_label_block(file_set, &diagnostic.primary, true);

    for secondary in &diagnostic.secondary {
        rendered.push('\n');
        rendered.push('\n');
        rendered.push_str(&render_label_block(file_set, secondary, true));
    }

    rendered
}

fn render_label_block(file_set: &FileSet, label: &DiagLabel, include_label_text: bool) -> String {
    let Some(file) = file_set.get(label.span.file_id) else {
        return render_fallback(label, "<unknown>");
    };

    if !label.span.is_valid_for(file) {
        return render_fallback(label, &file.path.display().to_string());
    }

    let (line, column) = file.line_col(label.span.start);
    let Some(line_text) = file.line_text(line) else {
        return render_fallback(label, &file.path.display().to_string());
    };
    let marker_width = marker_width(file, label);
    let caret = marker_for_kind(label.kind).repeat(marker_width);
    let gutter_width = line.to_string().len();
    let line_prefix = format!("{line:>gutter_width$}");
    let caret_padding = " ".repeat(column.saturating_sub(1));

    let mut rendered = String::new();
    rendered.push_str(&format!("  --> {}:{line}:{column}", file.path.display()));
    rendered.push('\n');
    rendered.push_str(&format!("{:>gutter_width$} |", ""));
    rendered.push('\n');
    rendered.push_str(&format!("{line_prefix} | {line_text}"));
    rendered.push('\n');
    rendered.push_str(&format!(
        "{:>gutter_width$} | {}{}",
        "",
        caret_padding,
        caret
    ));
    if include_label_text && !label.text.is_empty() {
        rendered.push(' ');
        rendered.push_str(&label.text);
    }

    rendered
}

fn render_fallback(label: &DiagLabel, path: &str) -> String {
    format!(
        "  --> {path}\n   |\n   | [bytes {}..{}] {}",
        label.span.start, label.span.end, label.text
    )
}

fn marker_for_kind(kind: DiagLabelKind) -> &'static str {
    match kind {
        DiagLabelKind::Primary => "^",
        DiagLabelKind::Secondary | DiagLabelKind::Removal => "-",
        DiagLabelKind::Ownership => "~",
        DiagLabelKind::Addition => "+",
    }
}

fn marker_width(file: &kobo_ir::FileEntry, label: &DiagLabel) -> usize {
    if label.span.is_empty() {
        return 1;
    }

    let (line, column_start) = file.line_col(label.span.start);
    let (_, column_end) = file.line_col(label.span.end.saturating_sub(1));
    if let Some(line_text) = file.line_text(line) {
        return (column_end.saturating_sub(column_start) + 1).min(line_text.len().max(1));
    }

    1
}

fn push_help(rendered: &mut String, help: &DiagHelp) {
    if help.is_empty() {
        return;
    }

    rendered.push('\n');
    rendered.push_str("help: ");
    rendered.push_str(&help.0);
}

fn push_run(rendered: &mut String, run: &CliSuggestion) {
    if run.is_empty() {
        return;
    }

    rendered.push('\n');
    rendered.push_str("run: ");
    rendered.push_str(&run.0);
}

#[cfg(test)]
mod tests {
    use kobo_ir::{FileSetBuilder, KoboSpan};

    use crate::codes::{KErrorCode, Severity};
    use crate::diagnostic::{DiagExplanation, DiagLabel, DiagLabelKind, KDiagnostic};

    use super::format_diagnostic;

    #[test]
    fn formatter_renders_k0001_with_secondary_label() {
        let mut file_set_builder = FileSetBuilder::new();
        let file_id = file_set_builder.add_file(
            "src/main.kobo".into(),
            "process(config);\nlog(config);\n".to_owned(),
        );
        let file_set = file_set_builder.finish();
        let diagnostic = KDiagnostic::new(
            KErrorCode::K0001,
            Severity::Error,
            DiagLabel::primary(KoboSpan::new(21, 27, file_id), "value used here"),
            DiagExplanation("`config` was moved earlier and is no longer valid".to_owned()),
            "no automatic rewrite applied",
        )
        .with_secondary_label(DiagLabel::new(
            KoboSpan::new(8, 14, file_id),
            DiagLabelKind::Secondary,
            "value moved here",
        ))
        .with_help("clone explicitly at the call site")
        .with_run("kobo check src/main.kobo:2");

        let rendered = format_diagnostic(&file_set, &diagnostic);

        assert!(rendered.contains("error[K0001]: value used after move"));
        assert!(rendered.contains("--> src/main.kobo:2:5"));
        assert!(rendered.contains("------ value moved here"));
        assert!(rendered.contains("help: clone explicitly at the call site"));
        assert!(rendered.contains("run: kobo check src/main.kobo:2"));
    }

    #[test]
    fn formatter_renders_k0002_and_omits_empty_help_run() {
        let mut file_set_builder = FileSetBuilder::new();
        let file_id = file_set_builder.add_file(
            "src/main.kobo".into(),
            "let r = &data;\ndata.push(1);\n".to_owned(),
        );
        let file_set = file_set_builder.finish();
        let diagnostic = KDiagnostic::new(
            KErrorCode::K0002,
            Severity::Error,
            DiagLabel::primary(KoboSpan::new(15, 19, file_id), "mutable borrow occurs here"),
            "immutable borrow is still active",
            "flagged as conflict; simultaneous borrows would panic",
        )
        .with_secondary_label(DiagLabel::new(
            KoboSpan::new(9, 13, file_id),
            DiagLabelKind::Secondary,
            "immutable borrow occurs here",
        ));

        let rendered = format_diagnostic(&file_set, &diagnostic);

        assert!(rendered.contains("error[K0002]: cannot borrow as mutable - already borrowed"));
        assert!(rendered.contains("---- immutable borrow occurs here"));
        assert!(!rendered.contains("help:"));
        assert!(!rendered.contains("run:"));
    }

    #[test]
    fn formatter_falls_back_to_byte_ranges_for_invalid_spans() {
        let mut file_set_builder = FileSetBuilder::new();
        let file_id = file_set_builder.add_file("src/main.kobo".into(), "data".to_owned());
        let file_set = file_set_builder.finish();
        let diagnostic = KDiagnostic::new(
            KErrorCode::K0001,
            Severity::Error,
            DiagLabel::primary(KoboSpan::new(10, 20, file_id), "value used here"),
            "fallback expected",
            "no automatic rewrite applied",
        );

        let rendered = format_diagnostic(&file_set, &diagnostic);
        assert!(rendered.contains("[bytes 10..20]"));
        assert!(rendered.contains("src/main.kobo"));
    }
}
