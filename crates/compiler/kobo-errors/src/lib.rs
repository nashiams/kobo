mod codes;
mod diagnostic;
mod format;

pub use codes::{KErrorCode, KErrorMetadata, Severity};
pub use diagnostic::{
    CliSuggestion, DiagDecision, DiagExplanation, DiagHelp, DiagLabel, DiagLabelKind, KDiagnostic,
};
pub use format::{format_diagnostic, render_k0020, render_k0021, DiagOwnerStats};
pub use kobo_ir::KoboSpan;
