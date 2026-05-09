mod codes;
mod diagnostic;
mod explain;
mod format;
mod json;
mod lsp;
mod registry;
mod render;

pub use codes::{resolve_severity, KErrorCode, KErrorMetadata, Severity};
pub use diagnostic::{
    CliSuggestion, DiagDecision, DiagExplanation, DiagHelp, DiagLabel, DiagLabelKind,
    DiagnosticNote, DiagnosticRelatedInfo, DiagnosticSuggestion, DiagnosticSuppression,
    DiagnosticUpstream, KDiagnostic, SuggestionApplicability, TextEdit,
};
pub use explain::{explain_code, unknown_code_message};
pub use format::{
    format_diagnostic, render_k0020, render_k0021, render_k0041, render_k0042, render_k0043,
    render_k0063, render_labeled_cross_boundary, render_related_info, render_span_compact,
    DiagOwnerStats,
};
pub use json::{
    diagnostic_to_json_value, DiagnosticJson, DiagnosticJsonRelated, DiagnosticJsonSpan,
    DiagnosticJsonSuggestion, DiagnosticJsonSuppression, DiagnosticJsonTextEdit,
    DiagnosticJsonUpstream,
};
pub use kobo_ir::KoboSpan;
pub use lsp::{DiagnosticLspPayload, LspPosition, LspRange, LspRelatedInformation};
pub use registry::{
    diagnostic_registry, DiagnosticCategory, DiagnosticRegistry, DiagnosticRegistryEntry,
    DiagnosticStatus, MachineEditPolicy, ModeBehavior, SeverityPolicy, SuggestionPolicy,
};
pub use render::{
    render_diagnostic_card, render_diagnostic_header_only, ColorMode, DiagnosticOutputFormat,
    DiagnosticRenderer,
};

/// Convenience alias for library-level error handling.
///
/// Use `KoboError` in library crates (kobo-ir, kobo-migrate, etc.).
/// Binary crates (kobo-cli) use `anyhow::Error` instead.
pub type KoboError = Box<dyn std::error::Error + Send + Sync>;
