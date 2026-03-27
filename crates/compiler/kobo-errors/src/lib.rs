mod codes;
mod diagnostic;
mod format;

pub use codes::{KErrorCode, Severity};
pub use diagnostic::{
    CliSuggestion, DiagDecision, DiagHelp, DiagMessage, DiagnosticSpec, KDiagnostic,
    RendererDiagnostic,
};
pub use format::format_diagnostic;
pub use kobo_ir::KoboSpan;
