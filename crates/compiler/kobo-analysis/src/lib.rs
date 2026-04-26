pub mod cancel_safety;
mod debug;
mod diagnostics;
mod liveness;
mod ownership_facts;
mod passes;
pub mod pipeline;
mod runner;
pub mod send_diagnostic;
pub mod split_borrow;

pub use cancel_safety::{scan_source_cancel_safety, CancelSafetyWarning};
pub use diagnostics::facts_to_diagnostics;
pub use ownership_facts::{BorrowFact, BorrowKind, MoveFact};
pub use pipeline::{check_pipeline_ordering, PipelineIssue, PipelineWarning};
pub use runner::{run_analysis, AnalysisFacts};
pub use send_diagnostic::{analyze_send_violations, SendDiagnostic, SpawnSite};
pub use split_borrow::{
    detect_split_borrow_sites, FieldAccess, FieldAccessKind, SplitBorrowSite,
};
