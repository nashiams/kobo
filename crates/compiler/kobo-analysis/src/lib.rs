pub mod cancel_safety;
mod debug;
mod diagnostics;
pub mod handler_leak;
mod liveness;
mod ownership_facts;
pub mod parallel;
mod passes;
pub mod pipeline;
mod runner;
pub mod send_diagnostic;
pub mod service_signature;
pub mod split_borrow;
pub mod task_local;

pub use cancel_safety::{scan_source_cancel_safety, CancelSafetyWarning};
pub use diagnostics::{facts_to_diagnostics, facts_to_ownership_debt};
pub use handler_leak::{scan_source_handler_leaks, HandlerLeakWarning};
pub use ownership_facts::{BorrowFact, BorrowKind, MoveFact};
pub use parallel::{scan_source_parallel_warnings, ParallelWarning, ParallelWarningKind};
pub use pipeline::{check_pipeline_ordering, PipelineIssue, PipelineWarning};
pub use runner::{run_analysis, AnalysisFacts};
pub use send_diagnostic::{analyze_send_violations, SendDiagnostic, SpawnSite};
pub use service_signature::{
    scan_source_service_signature_warnings, ServiceSignatureWarning, ServiceSignatureWarningReason,
};
pub use split_borrow::{detect_split_borrow_sites, FieldAccess, FieldAccessKind, SplitBorrowSite};
pub use task_local::{
    scan_source_task_local_captures, scan_source_task_local_warnings, TaskLocalCapture,
    TaskLocalWarning, TaskLocalWarningKind,
};
