use std::path::PathBuf;

use kobo_errors::KDiagnostic;
use kobo_ir::{
    FileId, FileSet, FileSetBuilder, GuaranteePolicy, GuaranteeProfile, KoboSpan, NodeIdGen,
};

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
    /// Checked guarantee profile forces this to true regardless of KOBO_DIAG.
    /// Dev profile still respects KOBO_DIAG=1 for opt-in instrumentation.
    pub diag_enabled: bool,
    /// Byte-offset spans of functions annotated with `#[kobo::relax]` in the source file.
    /// Used at render time to suppress `Severity::Warning` diagnostics in checked mode.
    pub relaxed_fn_ranges: Vec<KoboSpan>,
    /// True when the guarantee profile was set by a CLI flag.
    pub cli_profile_override: bool,
    /// Struct names annotated with `#[kobo::engine]` in the source. Used by the
    /// solver to impose PlainOwned ceiling constraints on engine resources.
    pub engine_struct_names: Vec<String>,
    /// Parser-recovered source regions skipped by downstream diagnostic phases.
    pub poisoned_spans: Vec<KoboSpan>,
    /// Diagnostics hidden because they overlap parser-poisoned source regions.
    pub suppressed_diagnostics: Vec<KDiagnostic>,
}

pub enum PoisonStatus {
    Visible,
    TrimRelatedPoison,
    Suppress { poison: Option<KoboSpan> },
}

impl CompileSession {
    pub fn new(config: KoboConfig) -> Self {
        // Checked mode remains authoritative even when KOBO_DIAG=0 is set.
        let diag_enabled = config.guarantee_policy.diag_always_active()
            || std::env::var("KOBO_DIAG").as_deref() == Ok("1");
        Self {
            config,
            file_set_builder: FileSetBuilder::new(),
            id_gen: NodeIdGen::new(),
            diagnostics: Vec::new(),
            diag_enabled,
            relaxed_fn_ranges: Vec::new(),
            cli_profile_override: false,
            engine_struct_names: Vec::new(),
            poisoned_spans: Vec::new(),
            suppressed_diagnostics: Vec::new(),
        }
    }

    pub fn guarantee_policy(&self) -> &GuaranteePolicy {
        &self.config.guarantee_policy
    }

    pub fn guarantee_profile(&self) -> GuaranteeProfile {
        self.config.guarantee_policy.profile()
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

    pub fn push_suppressed_diagnostic(&mut self, diagnostic: KDiagnostic) {
        self.suppressed_diagnostics.push(diagnostic);
    }

    pub fn visible_diagnostics(&self) -> impl Iterator<Item = &KDiagnostic> {
        self.diagnostics.iter()
    }

    pub fn is_poisoned(&self, span: KoboSpan) -> bool {
        self.poisoned_spans
            .iter()
            .any(|poison| poison.overlaps(span))
    }

    pub fn diagnostic_poison_status(&self, diagnostic: &KDiagnostic) -> PoisonStatus {
        if self.is_poisoned(diagnostic.primary.span) {
            return PoisonStatus::Suppress {
                poison: self
                    .poisoned_spans
                    .iter()
                    .copied()
                    .find(|poison| poison.overlaps(diagnostic.primary.span)),
            };
        }

        let touches_poison = diagnostic
            .secondary
            .iter()
            .any(|label| self.is_poisoned(label.span))
            || diagnostic
                .related
                .iter()
                .any(|related| self.is_poisoned(related.span))
            || diagnostic
                .suggestions
                .iter()
                .flat_map(|suggestion| suggestion.edits.iter())
                .any(|edit| self.is_poisoned(edit.span));

        if touches_poison {
            PoisonStatus::TrimRelatedPoison
        } else {
            PoisonStatus::Visible
        }
    }

    pub fn suppress_diagnostics_from(&mut self, visible_start: usize) {
        if self.poisoned_spans.is_empty() || visible_start >= self.diagnostics.len() {
            return;
        }

        let pending = self.diagnostics.split_off(visible_start);
        for diagnostic in pending {
            match self.diagnostic_poison_status(&diagnostic) {
                PoisonStatus::Visible => self.push_diagnostic(diagnostic),
                PoisonStatus::TrimRelatedPoison => {
                    let trimmed =
                        diagnostic.with_poisoned_related_info_trimmed(&self.poisoned_spans);
                    self.push_diagnostic(trimmed);
                }
                PoisonStatus::Suppress { poison } => {
                    let suppressed = match poison {
                        Some(span) => diagnostic.with_suppressed_by(span),
                        None => diagnostic,
                    };
                    self.push_suppressed_diagnostic(suppressed);
                }
            }
        }
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
/// `#[kobo::relax]`-annotated functions in checked mode.
pub fn is_inside_relaxed_fn(span: KoboSpan, relaxed_fn_ranges: &[KoboSpan]) -> bool {
    relaxed_fn_ranges.iter().any(|fn_span| {
        fn_span.file_id == span.file_id && span.start >= fn_span.start && span.end <= fn_span.end
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::config::KoboConfig;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn session_with_profile(profile: GuaranteeProfile) -> CompileSession {
        let config = KoboConfig {
            guarantee_policy: GuaranteePolicy::for_profile(profile),
            ..Default::default()
        };
        // Force KOBO_DIAG to unset for deterministic results
        std::env::remove_var("KOBO_DIAG");
        CompileSession::new(config)
    }

    #[test]
    fn dev_profile_no_env_diag_disabled() {
        let _guard = ENV_LOCK.lock().expect("env lock should not be poisoned");
        std::env::remove_var("KOBO_DIAG");
        let session = session_with_profile(GuaranteeProfile::Dev);
        assert!(
            !session.diag_enabled,
            "script+no-env must be diag_enabled=false"
        );
    }

    #[test]
    fn dev_profile_kobo_diag_1_enables_diag() {
        let _guard = ENV_LOCK.lock().expect("env lock should not be poisoned");
        std::env::set_var("KOBO_DIAG", "1");
        let config = KoboConfig {
            guarantee_policy: GuaranteePolicy::for_profile(GuaranteeProfile::Dev),
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
    fn checked_profile_no_env_diag_enabled() {
        let _guard = ENV_LOCK.lock().expect("env lock should not be poisoned");
        std::env::remove_var("KOBO_DIAG");
        let session = session_with_profile(GuaranteeProfile::Checked);
        assert!(
            session.diag_enabled,
            "checked mode must be diag_enabled=true regardless of env"
        );
    }

    #[test]
    fn checked_profile_kobo_diag_1_still_enabled() {
        std::env::set_var("KOBO_DIAG", "1");
        let config = KoboConfig {
            guarantee_policy: GuaranteePolicy::for_profile(GuaranteeProfile::Checked),
            ..Default::default()
        };
        let session = CompileSession::new(config);
        std::env::remove_var("KOBO_DIAG");
        assert!(session.diag_enabled);
    }

    #[test]
    fn checked_profile_kobo_diag_0_still_enabled() {
        // KOBO_DIAG=0 must not override checked mode.
        std::env::set_var("KOBO_DIAG", "0");
        let config = KoboConfig {
            guarantee_policy: GuaranteePolicy::for_profile(GuaranteeProfile::Checked),
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
    fn release_profile_diag_disabled() {
        std::env::remove_var("KOBO_DIAG");
        let session = session_with_profile(GuaranteeProfile::Release);
        assert!(
            !session.diag_enabled,
            "strict mode has no Rc/RefCell wrappers — no DiagOwner"
        );
    }

    #[test]
    fn policy_accessor_returns_config_policy() {
        let session = session_with_profile(GuaranteeProfile::Checked);
        assert_eq!(session.guarantee_profile(), GuaranteeProfile::Checked);
    }
}

// ---------------------------------------------------------------------------
// is_inside_relaxed_fn filter tests
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
