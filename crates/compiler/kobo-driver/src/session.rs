use std::path::PathBuf;

use kobo_errors::KDiagnostic;
use kobo_ir::{FileId, FileSet, FileSetBuilder, NodeIdGen};

use crate::config::KoboConfig;

/// Accumulates all state for a single compilation session.
///
/// Owned by the caller (CLI or LSP). Passed mutably through every pipeline phase.
pub struct CompileSession {
    pub config: KoboConfig,
    file_set_builder: FileSetBuilder,
    pub id_gen: NodeIdGen,
    pub diagnostics: Vec<KDiagnostic>,
    /// `true` when `KOBO_DIAG=1` was set in the process environment at session
    /// creation time. Controls whether codegen wraps `RcMutShared` bindings in
    /// `DiagOwner::new(…)` and whether the `diag` feature is activated.
    pub diag_enabled: bool,
}

impl CompileSession {
    pub fn new(config: KoboConfig) -> Self {
        let diag_enabled = std::env::var("KOBO_DIAG").as_deref() == Ok("1");
        Self {
            config,
            file_set_builder: FileSetBuilder::new(),
            id_gen: NodeIdGen::new(),
            diagnostics: Vec::new(),
            diag_enabled,
        }
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
