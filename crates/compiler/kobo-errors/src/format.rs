use kobo_ir::{FileEntry, FileSet, KoboSpan, StrictBoundaryFact, StrictBoundaryViolation};

use crate::diagnostic::{
    CliSuggestion, DiagHelp, DiagLabel, DiagLabelKind, DiagnosticRelatedInfo, KDiagnostic,
};

const HUMAN_GUIDANCE_WIDTH: usize = 100;

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
    push_section(&mut rendered, "What Kobo found:", card_finding(diagnostic));
    push_section(
        &mut rendered,
        "Why this matters:",
        &diagnostic.explanation.0,
    );
    push_section(&mut rendered, "Try this:", &diagnostic.decision.0);
    push_more_section(&mut rendered, diagnostic);

    rendered
}

fn card_finding(diagnostic: &KDiagnostic) -> &str {
    diagnostic
        .finding
        .as_deref()
        .unwrap_or(&diagnostic.primary.text)
}

pub fn render_related_info(file_set: &FileSet, related: &DiagnosticRelatedInfo) -> String {
    let mut rendered = String::new();
    rendered.push_str("related: ");
    rendered.push_str(&render_span_compact(file_set, related.span));
    if !related.message.is_empty() {
        rendered.push_str(": ");
        rendered.push_str(&related.message);
    }
    rendered
}

pub fn render_span_compact(file_set: &FileSet, span: KoboSpan) -> String {
    let Some(file) = file_set.get(span.file_id) else {
        return format!("<unknown>:{}..{}", span.start, span.end);
    };

    if !can_render_span_start(file, span) {
        return format!("{}:{}..{}", file.path.display(), span.start, span.end);
    }

    let (line, column) = file.line_col(span.start);
    format!("{}:{line}:{column}", file.path.display())
}

fn render_label_group(file_set: &FileSet, diagnostic: &KDiagnostic) -> String {
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

fn push_section(rendered: &mut String, heading: &str, value: &str) {
    let value = value.trim();
    if value.is_empty() {
        return;
    }

    rendered.push('\n');
    rendered.push_str(heading);
    rendered.push('\n');
    push_wrapped_multiline_text(rendered, value);
}

fn push_wrapped_multiline_text(rendered: &mut String, value: &str) {
    for (index, line) in value.lines().enumerate() {
        if index > 0 {
            rendered.push('\n');
        }

        if line.trim().is_empty() {
            continue;
        }

        push_wrapped_text(rendered, "  ", "  ", line.trim());
    }
}

fn push_wrapped_text(rendered: &mut String, prefix: &str, continuation: &str, value: &str) {
    let mut line = prefix.to_owned();
    for word in value.split_whitespace() {
        let separator_width = usize::from(!line.ends_with(' '));
        let projected_width = line.chars().count() + separator_width + word.chars().count();
        if projected_width > HUMAN_GUIDANCE_WIDTH && line.trim() != prefix.trim() {
            rendered.push_str(line.trim_end());
            rendered.push('\n');
            line.clear();
            line.push_str(continuation);
            line.push_str(word);
        } else {
            if !line.ends_with(' ') {
                line.push(' ');
            }
            line.push_str(word);
        }
    }
    rendered.push_str(line.trim_end());
}

fn push_more_section(rendered: &mut String, diagnostic: &KDiagnostic) {
    rendered.push('\n');
    rendered.push_str("More:");
    rendered.push('\n');
    push_wrapped_text(
        rendered,
        "  ",
        "  ",
        &format!(
            "Run `kobo explain {}` for examples and deeper context.",
            diagnostic.code
        ),
    );

    if let Some(help) = diagnostic.help.as_ref().filter(|help| !help.is_empty()) {
        rendered.push('\n');
        push_wrapped_text(rendered, "  Also: ", "  ", &help.0);
    }

    if let Some(run) = diagnostic.run.as_ref().filter(|run| !run.is_empty()) {
        rendered.push('\n');
        push_wrapped_text(rendered, "  Rerun: ", "  ", &run.0);
    }
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
    use crate::codes::KErrorCode;
    use crate::codes::Severity;

    let label_text = format!(
        "{} borrow calls (x86-64 ref)",
        stats.borrow_count.max(stats.mut_borrow_count)
    );
    let primary = DiagLabel::primary(stats.source_location, label_text);

    let est = stats.estimated_total_ms();
    let explanation = format!(
        "`{}` uses shared mutable ownership inside a tight loop; estimated overhead is about 14ns per access on the x86-64 reference machine, or about {}ms in the last run",
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
    use crate::codes::KErrorCode;
    use crate::codes::Severity;

    let primary = DiagLabel::primary(stats.source_location, "counter reached u64::MAX".to_owned());

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
        "advisory only; program continues to run correctly",
    )
}

// ---------------------------------------------------------------------------
// @strict boundary diagnostics (K0041 / K0042 / K0043 / K0063) — P4 v0.5
// ---------------------------------------------------------------------------

/// Render a K0041 diagnostic: active Rc aliases at @strict block entry.
///
/// Contract C03: Severity::Error.
pub fn render_k0041(fact: &StrictBoundaryFact) -> KDiagnostic {
    use crate::codes::{KErrorCode, Severity};

    let alias_sites = match &fact.violation {
        StrictBoundaryViolation::ActiveAliases { alias_sites, .. } => alias_sites.as_slice(),
        _ => panic!("render_k0041 called with wrong violation type"),
    };

    let primary = DiagLabel::primary(fact.block_span, "active aliases at @strict block entry");

    let alias_count = alias_sites.len();
    let explanation = format!(
        "value has {alias_count} active alias{} when the @strict block starts; \
         all aliases must end before the boundary is entered",
        if alias_count == 1 { "" } else { "es" }
    );

    let mut diag = KDiagnostic::new(
        KErrorCode::K0041,
        Severity::Error,
        primary,
        explanation,
        "drop or explicitly clone-then-drop aliases before the @strict { } boundary",
    );

    for &alias_span in alias_sites {
        diag.secondary
            .push(DiagLabel::secondary(alias_span, "alias created here"));
    }

    diag
}

/// Render a K0042 diagnostic: closure captures Rc<RefCell<T>> across @strict boundary.
///
/// Contract C03: Severity::Error.
pub fn render_k0042(fact: &StrictBoundaryFact) -> KDiagnostic {
    use crate::codes::{KErrorCode, Severity};

    let closure_span = match &fact.violation {
        StrictBoundaryViolation::ClosureCapture { closure_span, .. } => *closure_span,
        _ => panic!("render_k0042 called with wrong violation type"),
    };

    let primary = DiagLabel::primary(
        closure_span,
        "closure captures a shared mutable binding across @strict boundary",
    );

    let explanation =
        "a closure defined inside an @strict block captures a shared mutable binding; \
         calling the closure later could keep the strict borrow alive after the block ends";

    let mut diag = KDiagnostic::new(
        KErrorCode::K0042,
        Severity::Error,
        primary,
        explanation,
        "extract the closure outside the @strict block, or refactor to avoid capturing \
         the guarded value",
    );

    diag.secondary
        .push(DiagLabel::secondary(fact.block_span, "@strict block here"));
    diag
}

/// Render a K0043 diagnostic: value moved inside @strict block.
///
/// Contract C03: Severity::Error.
pub fn render_k0043(fact: &StrictBoundaryFact) -> KDiagnostic {
    use crate::codes::{KErrorCode, Severity};

    let move_site = match &fact.violation {
        StrictBoundaryViolation::MovedInside { move_site, .. } => *move_site,
        _ => panic!("render_k0043 called with wrong violation type"),
    };

    let primary = DiagLabel::primary(move_site, "value moved here");

    let explanation = "a binding is moved inside an @strict block; the boundary guard is dropped \
         on exit from the block and the moved value cannot be restored to its outer ownership shape";

    let mut diag = KDiagnostic::new(
        KErrorCode::K0043,
        Severity::Error,
        primary,
        explanation,
        "clone the value before moving, or drop the guard before the move statement",
    );

    diag.secondary
        .push(DiagLabel::secondary(fact.block_span, "@strict block"));
    diag
}

/// Render a K0063 diagnostic: @strict block inside async fn.
///
/// Contract C03: Severity::Error.
pub fn render_k0063(fact: &StrictBoundaryFact) -> KDiagnostic {
    use crate::codes::{KErrorCode, Severity};

    let async_fn_span = match &fact.violation {
        StrictBoundaryViolation::AsyncContext { async_fn_span } => *async_fn_span,
        _ => panic!("render_k0063 called with wrong violation type"),
    };

    let primary = DiagLabel::primary(fact.block_span, "this strict borrow starts here");

    let explanation =
        "Kobo needs strict borrows to end before the function can pause or be cancelled.";

    let mut diag = KDiagnostic::new(
        KErrorCode::K0063,
        Severity::Error,
        primary,
        explanation,
        "1. Use `@strict async fn` if the whole function should follow Kobo's async strict rules.\n\
         2. Or move this strict work into a small non-async helper.",
    )
    .with_finding("This strict block runs inside an async function.");

    diag.secondary.push(DiagLabel::secondary(
        async_fn_span,
        "async functions can pause at `.await`",
    ));
    diag
}

/// Render a labeled break/continue crossing @strict boundary diagnostic.
///
/// Render K0044 for labeled break/continue across an @strict boundary.
/// Contract C03: Severity::Error.
pub fn render_labeled_cross_boundary(fact: &StrictBoundaryFact) -> KDiagnostic {
    use crate::codes::{KErrorCode, Severity};

    let (label, break_span) = match &fact.violation {
        StrictBoundaryViolation::LabeledCrossBoundary {
            label,
            break_or_continue_span,
            ..
        } => (label.as_str(), *break_or_continue_span),
        _ => panic!("render_labeled_cross_boundary called with wrong violation type"),
    };

    let primary = DiagLabel::primary(
        break_span,
        format!("labeled `{label}` crosses @strict boundary"),
    );

    let explanation = format!(
        "break or continue to label `{label}` would exit the @strict block without \
         dropping borrow guards in the correct order; this is not allowed in v0.5"
    );

    let mut diag = KDiagnostic::new(
        KErrorCode::K0044,
        Severity::Error,
        primary,
        explanation,
        "restructure the code to avoid labeled break/continue across @strict boundaries",
    );

    diag.secondary
        .push(DiagLabel::secondary(fact.block_span, "@strict block here"));
    diag
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
        assert!(rendered.contains("What Kobo found:"));
        assert!(rendered.contains("Why this matters:"));
        assert!(rendered.contains("Try this:"));
        assert!(rendered.contains("More:"));
        assert!(rendered.contains("clone explicitly at the call site"));
        assert!(rendered.contains("Rerun:"));
        assert!(rendered.contains("src/main.kobo:2"));
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

// ---------------------------------------------------------------------------
// P4 @strict renderer tests — written FAILING first (TDD)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod strict_renderer_tests {
    use kobo_ir::{FileId, KirNodeId, KoboSpan, StrictBoundaryFact, StrictBoundaryViolation};

    use crate::codes::{KErrorCode, Severity};

    use super::{render_k0041, render_k0042, render_k0043, render_k0063};

    fn span(s: u32, e: u32) -> KoboSpan {
        KoboSpan::new(s, e, FileId(0))
    }

    fn node(id: u32) -> KirNodeId {
        KirNodeId(id)
    }

    // ── K0041 ──────────────────────────────────────────────────────────────

    /// Test 1: render_k0041 produces K0041 with Severity::Error (Contract C03).
    #[test]
    fn test_render_k0041_code_and_severity() {
        let fact = StrictBoundaryFact {
            block_span: span(10, 50),
            violation: StrictBoundaryViolation::ActiveAliases {
                binding_id: node(1),
                alias_sites: vec![span(5, 8)],
            },
        };
        let diag = render_k0041(&fact);
        assert_eq!(diag.code, KErrorCode::K0041, "K0041 code expected");
        assert_eq!(
            diag.severity,
            Severity::Error,
            "K0041 must use Severity::Error (C03)"
        );
    }

    /// Test 2: render_k0041 primary span is the @strict block_span.
    #[test]
    fn test_render_k0041_primary_span_is_block_span() {
        let block_span = span(10, 50);
        let fact = StrictBoundaryFact {
            block_span,
            violation: StrictBoundaryViolation::ActiveAliases {
                binding_id: node(2),
                alias_sites: vec![span(3, 6), span(7, 9)],
            },
        };
        let diag = render_k0041(&fact);
        assert_eq!(
            diag.primary.span, block_span,
            "K0041 primary label must span the @strict block"
        );
        // Secondary labels cover each alias site
        assert_eq!(
            diag.secondary.len(),
            2,
            "one secondary label per alias site"
        );
    }

    // ── K0042 ──────────────────────────────────────────────────────────────

    /// Test 3: render_k0042 produces K0042 with Severity::Error (Contract C03).
    #[test]
    fn test_render_k0042_code_and_severity() {
        let fact = StrictBoundaryFact {
            block_span: span(20, 80),
            violation: StrictBoundaryViolation::ClosureCapture {
                closure_span: span(30, 60),
                captured_binding_id: node(3),
                is_move_closure: false,
                captures: vec![],
            },
        };
        let diag = render_k0042(&fact);
        assert_eq!(diag.code, KErrorCode::K0042, "K0042 code expected");
        assert_eq!(
            diag.severity,
            Severity::Error,
            "K0042 must use Severity::Error (C03)"
        );
    }

    /// Test 4: render_k0042 primary span is the closure_span; block appears as secondary.
    #[test]
    fn test_render_k0042_has_secondary_block_label() {
        let block_span = span(20, 80);
        let closure_span = span(30, 60);
        let fact = StrictBoundaryFact {
            block_span,
            violation: StrictBoundaryViolation::ClosureCapture {
                closure_span,
                captured_binding_id: node(4),
                is_move_closure: false,
                captures: vec![],
            },
        };
        let diag = render_k0042(&fact);
        assert_eq!(
            diag.primary.span, closure_span,
            "K0042 primary label must span the closure"
        );
        assert!(
            diag.secondary.iter().any(|s| s.span == block_span),
            "K0042 must have a secondary label at the @strict block_span"
        );
    }

    // ── K0043 ──────────────────────────────────────────────────────────────

    /// Test 5: render_k0043 produces K0043 with Severity::Error (Contract C03).
    #[test]
    fn test_render_k0043_code_and_severity() {
        let fact = StrictBoundaryFact {
            block_span: span(10, 90),
            violation: StrictBoundaryViolation::MovedInside {
                binding_id: node(5),
                move_site: span(40, 55),
            },
        };
        let diag = render_k0043(&fact);
        assert_eq!(diag.code, KErrorCode::K0043, "K0043 code expected");
        assert_eq!(
            diag.severity,
            Severity::Error,
            "K0043 must use Severity::Error (C03)"
        );
    }

    /// Test 6: render_k0043 primary span is the move_site.
    #[test]
    fn test_render_k0043_primary_span_is_move_site() {
        let move_site = span(40, 55);
        let fact = StrictBoundaryFact {
            block_span: span(10, 90),
            violation: StrictBoundaryViolation::MovedInside {
                binding_id: node(6),
                move_site,
            },
        };
        let diag = render_k0043(&fact);
        assert_eq!(
            diag.primary.span, move_site,
            "K0043 primary label must span the move site"
        );
    }

    // ── K0063 ──────────────────────────────────────────────────────────────

    /// Test 7: render_k0063 produces K0063 with Severity::Error (Contract C03).
    #[test]
    fn test_render_k0063_code_and_severity() {
        let fact = StrictBoundaryFact {
            block_span: span(50, 100),
            violation: StrictBoundaryViolation::AsyncContext {
                async_fn_span: span(0, 120),
            },
        };
        let diag = render_k0063(&fact);
        assert_eq!(diag.code, KErrorCode::K0063, "K0063 code expected");
        assert_eq!(
            diag.severity,
            Severity::Error,
            "K0063 must use Severity::Error (C03)"
        );
    }
}
