use kobo_ir::{FileSet, KoboSpan};

use crate::diagnostic::{CliSuggestion, DiagHelp, DiagLabel, KDiagnostic};
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
        "Move this hot loop into @strict, reuse owned data outside the loop, or keep the shared mutable shape with a measured reason.",
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

#[cfg(test)]
mod tests {
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
