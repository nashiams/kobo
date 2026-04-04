use kobo_ir::{FileEntry, FileSet, KoboSpan};

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
    if labels.iter().any(|label| {
        label.span.file_id != diagnostic.primary.span.file_id || !label.span.is_valid_for(file)
    }) {
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
        rendered.push_str(&format!(
            "{:>gutter_width$} | {}{}",
            "", caret_padding, caret
        ));
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
        "", caret_padding, caret
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

// ---------------------------------------------------------------------------
// DiagOwner diagnostic rendering (K0020 / K0021)
// ---------------------------------------------------------------------------

/// Statistics captured by a `DiagOwner<T>` instance and emitted on `Drop`.
///
/// Produced by parsing the `[kobo-diag]` output block from stderr.
/// Used by `kobo perf` to render K0020 / K0021 diagnostics.
#[derive(Clone, Debug)]
pub struct DiagOwnerStats {
    pub source_location: KoboSpan,
    pub binding_name: String,
    pub borrow_count: u64,
    pub mut_borrow_count: u64,
    pub contention_count: u64,
    pub saturated: bool,
    pub threshold: u64,
}

impl DiagOwnerStats {
    /// Derive estimated total overhead in milliseconds.
    ///
    /// Formula: `max(borrow_count, mut_borrow_count) * 14ns / 1_000_000`
    /// This is a reference-architecture heuristic measured on x86-64.
    /// Rendered as `(x86-64 ref)` in diagnostic output.
    pub fn estimated_total_ms(&self) -> u64 {
        self.borrow_count
            .max(self.mut_borrow_count)
            .saturating_mul(14)
            .saturating_div(1_000_000)
    }
}

/// Render a K0020 diagnostic for a hot `Rc<RefCell<T>>` borrow site.
///
/// Emits: `warning[K0020]: RefCell accessed >{threshold} times in hot path`
pub fn render_k0020(stats: &DiagOwnerStats, file_set: &FileSet) -> KDiagnostic {
    use crate::codes::Severity;
    use crate::codes::KErrorCode;

    let label_text = format!("{} borrow calls (x86-64 ref)", stats.borrow_count.max(stats.mut_borrow_count));
    let primary = DiagLabel::primary(stats.source_location, label_text);

    let est = stats.estimated_total_ms();
    let explanation = format!(
        "`{}` is Rc<RefCell<T>> accessed inside a tight loop\n   \
         = estimated overhead: ~14ns/access (x86-64 ref) → ~{}ms total last run",
        stats.binding_name, est
    );

    let source_str = file_set
        .get(stats.source_location.file_id)
        .map(|f| {
            let (line, _col) = f.line_col(stats.source_location.start);
            format!("{}:{}", f.path.display(), line)
        })
        .unwrap_or_default();

    KDiagnostic::new(
        KErrorCode::K0020,
        Severity::Warning,
        primary,
        explanation,
        "flagged for migration; no automatic fix applied",
    )
    .with_help(DiagHelp(
        "annotate the loop with @strict to get zero overhead inside the block".to_owned(),
    ))
    .with_run(CliSuggestion(format!("kobo migrate {source_str}")))
}

/// Render a K0021 diagnostic when a DiagOwner counter has saturated.
///
/// Emits: `warning[K0021]: DiagOwner borrow counter saturated — count understated`
pub fn render_k0021(stats: &DiagOwnerStats) -> KDiagnostic {
    use crate::codes::Severity;
    use crate::codes::KErrorCode;

    let primary = DiagLabel::primary(
        stats.source_location,
        "counter reached u64::MAX".to_owned(),
    );

    let explanation = format!(
        "`{}` borrow counter reached the maximum u64 value\n   \
         = reported borrow_count ({}) is a lower bound, not the exact count",
        stats.binding_name, stats.borrow_count
    );

    KDiagnostic::new(
        KErrorCode::K0021,
        Severity::Warning,
        primary,
        explanation,
        "advisory only — program continues to run correctly",
    )
}

#[cfg(test)]
mod diag_owner_tests {
    use super::*;
    use kobo_ir::{FileId, FileSet, KoboSpan};

    fn dummy_span() -> KoboSpan {
        KoboSpan::new(0, 0, FileId(0))
    }

    fn make_stats(borrow_count: u64, mut_borrow_count: u64, saturated: bool) -> DiagOwnerStats {
        DiagOwnerStats {
            source_location: dummy_span(),
            binding_name: "cache".to_owned(),
            borrow_count,
            mut_borrow_count,
            contention_count: 0,
            saturated,
            threshold: 10_000,
        }
    }

    #[test]
    fn estimated_ms_uses_max_of_counts() {
        let stats = make_stats(500_000, 200_000, false);
        // max(500_000, 200_000) * 14 / 1_000_000 = 7
        assert_eq!(stats.estimated_total_ms(), 7);
    }

    #[test]
    fn estimated_ms_zero_for_low_counts() {
        let stats = make_stats(100, 50, false);
        // 100 * 14 / 1_000_000 = 0 (integer division)
        assert_eq!(stats.estimated_total_ms(), 0);
    }

    #[test]
    fn render_k0020_correct_code() {
        let stats = make_stats(15_000, 5_000, false);
        let file_set = FileSet::new();
        let diag = render_k0020(&stats, &file_set);
        assert_eq!(diag.code, crate::codes::KErrorCode::K0020);
        assert_eq!(diag.severity, crate::codes::Severity::Warning);
    }

    #[test]
    fn render_k0020_label_contains_x86_ref() {
        let stats = make_stats(15_000, 5_000, false);
        let file_set = FileSet::new();
        let diag = render_k0020(&stats, &file_set);
        assert!(
            diag.primary.text.contains("x86-64 ref"),
            "label must mention x86-64 ref, got: {}",
            diag.primary.text
        );
    }

    #[test]
    fn render_k0021_correct_code_and_saturated_message() {
        let stats = make_stats(u64::MAX, 0, true);
        let diag = render_k0021(&stats);
        assert_eq!(diag.code, crate::codes::KErrorCode::K0021);
        assert_eq!(diag.severity, crate::codes::Severity::Warning);
        assert!(
            diag.primary.text.contains("u64::MAX"),
            "label should mention u64::MAX, got: {}",
            diag.primary.text
        );
    }

    #[test]
    fn render_k0020_has_help_and_run() {
        let stats = make_stats(20_000, 0, false);
        let file_set = FileSet::new();
        let diag = render_k0020(&stats, &file_set);
        assert!(diag.help.is_some(), "K0020 must carry a help suggestion");
        assert!(diag.run.is_some(), "K0020 must carry a run suggestion");
    }
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
