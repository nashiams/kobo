use kobo_errors::KDiagnostic;
use kobo_ir::{FileSet, NodeIdGen};

use crate::config::KoboConfig;

/// Accumulates all state for a single compilation session.
///
/// Owned by the caller (CLI or LSP). Passed mutably through every pipeline phase.
pub struct CompileSession {
    pub config: KoboConfig,
    pub file_set: FileSet,
    pub id_gen: NodeIdGen,
    pub diagnostics: Vec<KDiagnostic>,
}

impl CompileSession {
    pub fn new(config: KoboConfig) -> Self {
        Self {
            config,
            file_set: FileSet::new(),
            id_gen: NodeIdGen::new(),
            diagnostics: Vec::new(),
        }
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
