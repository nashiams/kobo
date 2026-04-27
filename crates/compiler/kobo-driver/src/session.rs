use std::path::PathBuf;

use kobo_errors::KDiagnostic;
use kobo_ir::{FileId, FileSet, FileSetBuilder, KoboMode, KoboSpan, NodeIdGen};

use crate::config::KoboConfig;

/// Accumulates all state for a single compilation session.
///
/// Owned by the caller (CLI or LSP). Passed mutably through every pipeline phase.
pub struct CompileSession {
    pub config: KoboConfig,
    file_set_builder: FileSetBuilder,
    pub id_gen: NodeIdGen,
    pub diagnostics: Vec<KDiagnostic>,
    /// True when DiagOwner instrumentation is active for this session.
    /// Checked mode forces this to true regardless of KOBO_DIAG env var [Contract R03].
    /// Script mode still respects KOBO_DIAG=1 for opt-in instrumentation.
    pub diag_enabled: bool,
    /// Byte-offset spans of functions annotated with `#[kobo::relax]` in the source file.
    /// Used at render time to suppress `Severity::Warning` diagnostics in checked mode [G5].
    pub relaxed_fn_ranges: Vec<KoboSpan>,
    /// True when the mode was set by a CLI flag. CLI overrides file attribute [S-26].
    pub cli_mode_override: bool,
    /// Struct names annotated with `#[kobo::engine]` in the source. Used by the
    /// solver to impose PlainOwned ceiling constraints on engine resources [S-3].
    pub engine_struct_names: Vec<String>,
}

impl CompileSession {
    pub fn new(config: KoboConfig) -> Self {
        // Mode check before env var — KOBO_DIAG=0 does NOT override checked mode [Trap 1].
        let diag_enabled =
            config.mode.diag_always_active() || std::env::var("KOBO_DIAG").as_deref() == Ok("1");
        Self {
            config,
            file_set_builder: FileSetBuilder::new(),
            id_gen: NodeIdGen::new(),
            diagnostics: Vec::new(),
            diag_enabled,
            relaxed_fn_ranges: Vec::new(),
            cli_mode_override: false,
            engine_struct_names: Vec::new(),
        }
    }

    /// The compile mode for this session. Single source of truth is config.mode [Trap 15].
    pub fn mode(&self) -> KoboMode {
        self.config.mode
    }

    pub fn file_set(&self) -> &FileSet {
        self.file_set_builder.as_file_set()
    }

    pub(crate) fn register_source_file(&mut self, path: PathBuf, source: String) -> FileId {
        self.file_set_builder.add_file(path, source)
    }

    /// Appends a diagnostic to the session accumulator.
    pub fn push_diagnostic(&mut self, d: KDiagnostic) {
        self.diagnostics.push(d);
    }

    /// Returns `true` if any `Severity::Error` diagnostic has been accumulated.
    pub fn has_errors(&self) -> bool {
        use kobo_errors::Severity;
        self.diagnostics
            .iter()
            .any(|d| d.severity == Severity::Error)
    }
}

/// Returns `true` when `span` falls within any of the relaxed function byte-offset ranges.
///
/// Used by the rendering path to suppress `Severity::Warning` diagnostics inside
/// `#[kobo::relax]`-annotated functions in checked mode [G5].
pub fn is_inside_relaxed_fn(span: KoboSpan, relaxed_fn_ranges: &[KoboSpan]) -> bool {
    relaxed_fn_ranges.iter().any(|fn_span| {
        fn_span.file_id == span.file_id && span.start >= fn_span.start && span.end <= fn_span.end
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::KoboConfig;

    fn session_with_mode(mode: KoboMode) -> CompileSession {
        let config = KoboConfig {
            mode,
            ..Default::default()
        };
        // Force KOBO_DIAG to unset for deterministic results
        std::env::remove_var("KOBO_DIAG");
        CompileSession::new(config)
    }

    #[test]
    fn script_mode_no_env_diag_disabled() {
        std::env::remove_var("KOBO_DIAG");
        let session = session_with_mode(KoboMode::Script);
        assert!(
            !session.diag_enabled,
            "script+no-env must be diag_enabled=false"
        );
    }

    #[test]
    fn script_mode_kobo_diag_1_enables_diag() {
        std::env::set_var("KOBO_DIAG", "1");
        let config = KoboConfig {
            mode: KoboMode::Script,
            ..Default::default()
        };
        let session = CompileSession::new(config);
        std::env::remove_var("KOBO_DIAG");
        assert!(
            session.diag_enabled,
            "script+KOBO_DIAG=1 must be diag_enabled=true"
        );
    }

    #[test]
    fn checked_mode_no_env_diag_enabled() {
        std::env::remove_var("KOBO_DIAG");
        let session = session_with_mode(KoboMode::Checked);
        assert!(
            session.diag_enabled,
            "checked mode must be diag_enabled=true regardless of env"
        );
    }

    #[test]
    fn checked_mode_kobo_diag_1_still_enabled() {
        std::env::set_var("KOBO_DIAG", "1");
        let config = KoboConfig {
            mode: KoboMode::Checked,
            ..Default::default()
        };
        let session = CompileSession::new(config);
        std::env::remove_var("KOBO_DIAG");
        assert!(session.diag_enabled);
    }

    #[test]
    fn checked_mode_kobo_diag_0_still_enabled() {
        // KOBO_DIAG=0 must NOT override checked mode [Contract R03 / Trap 1].
        std::env::set_var("KOBO_DIAG", "0");
        let config = KoboConfig {
            mode: KoboMode::Checked,
            ..Default::default()
        };
        let session = CompileSession::new(config);
        std::env::remove_var("KOBO_DIAG");
        assert!(
            session.diag_enabled,
            "checked mode is authoritative — KOBO_DIAG=0 must have no effect"
        );
    }

    #[test]
    fn strict_mode_diag_disabled() {
        std::env::remove_var("KOBO_DIAG");
        let session = session_with_mode(KoboMode::Strict);
        assert!(
            !session.diag_enabled,
            "strict mode has no Rc/RefCell wrappers — no DiagOwner"
        );
    }

    #[test]
    fn mode_accessor_returns_config_mode() {
        let session = session_with_mode(KoboMode::Checked);
        assert_eq!(session.mode(), KoboMode::Checked);
    }
}

// ---------------------------------------------------------------------------
// G5: is_inside_relaxed_fn filter tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod test_relax_filter {
    use kobo_ir::{FileId, KoboSpan};

    use super::is_inside_relaxed_fn;

    fn span(start: u32, end: u32) -> KoboSpan {
        KoboSpan::new(start, end, FileId(0))
    }

    fn fn_range(start: u32, end: u32) -> KoboSpan {
        KoboSpan::new(start, end, FileId(0))
    }

    fn other_file_span(start: u32, end: u32) -> KoboSpan {
        KoboSpan::new(start, end, FileId(1))
    }

    #[test]
    fn test_relax_filter_span_inside_range_returns_true() {
        // Diagnostic span fully inside a relaxed fn range → should be suppressed.
        let diag_span = span(10, 20);
        let ranges = [fn_range(5, 50)];
        assert!(is_inside_relaxed_fn(diag_span, &ranges));
    }

    #[test]
    fn test_relax_filter_span_outside_range_returns_false() {
        // Diagnostic span outside the relaxed fn range → not suppressed.
        let diag_span = span(60, 70);
        let ranges = [fn_range(5, 50)];
        assert!(!is_inside_relaxed_fn(diag_span, &ranges));
    }

    #[test]
    fn test_relax_filter_exact_boundary_inclusive() {
        // Span exactly equal to fn range is inside (start == fn_start, end == fn_end).
        let diag_span = span(5, 50);
        let ranges = [fn_range(5, 50)];
        assert!(is_inside_relaxed_fn(diag_span, &ranges));
    }

    #[test]
    fn test_relax_filter_span_starts_at_fn_start() {
        // Span that starts exactly at fn_start and ends inside → inside.
        let diag_span = span(5, 15);
        let ranges = [fn_range(5, 50)];
        assert!(is_inside_relaxed_fn(diag_span, &ranges));
    }

    #[test]
    fn test_relax_filter_span_ends_at_fn_end() {
        // Span that ends exactly at fn_end → inside.
        let diag_span = span(40, 50);
        let ranges = [fn_range(5, 50)];
        assert!(is_inside_relaxed_fn(diag_span, &ranges));
    }

    #[test]
    fn test_relax_filter_span_before_range_returns_false() {
        // Span ends before fn range starts → outside.
        let diag_span = span(1, 4);
        let ranges = [fn_range(5, 50)];
        assert!(!is_inside_relaxed_fn(diag_span, &ranges));
    }

    #[test]
    fn test_relax_filter_span_after_range_returns_false() {
        // Span starts after fn range ends → outside.
        let diag_span = span(51, 60);
        let ranges = [fn_range(5, 50)];
        assert!(!is_inside_relaxed_fn(diag_span, &ranges));
    }

    #[test]
    fn test_relax_filter_empty_ranges_always_false() {
        // No relaxed fn ranges → nothing is suppressed.
        let diag_span = span(10, 20);
        assert!(!is_inside_relaxed_fn(diag_span, &[]));
    }

    #[test]
    fn test_relax_filter_span_in_second_range_returns_true() {
        // Two relaxed fn ranges; span is in the second → suppressed.
        let diag_span = span(60, 70);
        let ranges = [fn_range(5, 50), fn_range(55, 100)];
        assert!(is_inside_relaxed_fn(diag_span, &ranges));
    }

    #[test]
    fn test_relax_filter_different_file_id_not_suppressed() {
        // Same byte offsets but different file_id → not inside range.
        let diag_span = other_file_span(10, 20); // FileId(1)
        let ranges = [fn_range(5, 50)]; // FileId(0)
        assert!(!is_inside_relaxed_fn(diag_span, &ranges));
    }
}
