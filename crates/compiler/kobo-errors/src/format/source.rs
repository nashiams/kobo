use kobo_ir::{FileEntry, FileSet, KoboSpan};

use crate::diagnostic::{DiagLabel, DiagLabelKind, KDiagnostic};

use super::labels::public_label_text;
pub(crate) fn render_label_group(file_set: &FileSet, diagnostic: &KDiagnostic) -> String {
    let Some(file) = file_set.get(diagnostic.primary.span.file_id) else {
        return render_individual_blocks(file_set, diagnostic);
    };

    let labels = ordered_labels(diagnostic);
    if labels.iter().any(|label| {
        label.span.file_id != diagnostic.primary.span.file_id
            || !can_render_span_start(file, label.span)
    }) {
        return render_individual_blocks(file_set, diagnostic);
    }

    render_grouped_block(file, &labels, &diagnostic.primary)
}

fn ordered_labels(diagnostic: &KDiagnostic) -> Vec<&DiagLabel> {
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
        rendered.push_str(&format!(
            "{:>gutter_width$} | {}{}",
            "", caret_padding, caret
        ));
        let label_text = public_label_text(&label.text);
        if !label_text.is_empty() {
            rendered.push(' ');
            rendered.push_str(&label_text);
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

    if !can_render_span_start(file, label.span) {
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
        "", caret_padding, caret
    ));
    let label_text = public_label_text(&label.text);
    if include_label_text && !label_text.is_empty() {
        rendered.push(' ');
        rendered.push_str(&label_text);
    }

    rendered
}

fn render_fallback(label: &DiagLabel, path: &str) -> String {
    let label_text = public_label_text(&label.text);
    format!(
        "  --> {path}\n   |\n   | [bytes {}..{}] {}",
        label.span.start, label.span.end, label_text
    )
}

fn can_render_span_start(file: &FileEntry, span: KoboSpan) -> bool {
    span.start <= span.end
        && (span.start as usize) <= file.source().len()
        && file.line_text(file.line_col(span.start).0).is_some()
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
    let (end_line, column_end) = file.line_col(label.span.end.saturating_sub(1));
    if let Some(line_text) = file.line_text(line) {
        if end_line != line {
            return line_text
                .len()
                .saturating_sub(column_start.saturating_sub(1))
                .max(1);
        }

        return (column_end.saturating_sub(column_start) + 1).min(line_text.len().max(1));
    }

    1
}
