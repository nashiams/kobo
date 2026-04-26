mod codes;
mod diagnostic;
mod format;

pub use codes::{resolve_severity, KErrorCode, KErrorMetadata, Severity};
pub use diagnostic::{
    CliSuggestion, DiagDecision, DiagExplanation, DiagHelp, DiagLabel, DiagLabelKind, KDiagnostic,
};
pub use format::{
    format_diagnostic, render_k0020, render_k0021,
    render_k0041, render_k0042, render_k0043, render_k0063,
    render_labeled_cross_boundary,
    DiagOwnerStats,
};
pub use kobo_ir::KoboSpan;

/// Convenience alias for library-level error handling.
///
/// Use `KoboError` in library crates (kobo-ir, kobo-migrate, etc.).
/// Binary crates (kobo-cli) use `anyhow::Error` instead.
pub type KoboError = Box<dyn std::error::Error + Send + Sync>;
