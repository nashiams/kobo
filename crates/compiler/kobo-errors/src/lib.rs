mod codes;
mod diagnostic;
mod format;

pub use codes::{KErrorCode, KErrorMetadata, Severity};
pub use diagnostic::{
    CliSuggestion, DiagDecision, DiagExplanation, DiagHelp, DiagLabel, DiagLabelKind,
    KDiagnostic,
};
pub use format::format_diagnostic;
pub use kobo_ir::KoboSpan;
