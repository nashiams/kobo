use std::io;

#[derive(Debug, thiserror::Error)]
pub enum SimCoreError {
    #[error("source-based sim-core lowering is not available in modeled-island mode; use run_semantics_from_program")]
    SourceLoweringUnavailable,
    #[error("source-based full-depth execution is not available; build a ScenarioProgram through kobo-driver")]
    SourceFullDepthUnavailable,
    #[error("generated harness failed to compile: {stderr}")]
    HarnessCompileFailed { stderr: String },
    #[error("sim-core I/O failed while {operation}: {source}")]
    Io {
        operation: &'static str,
        #[source]
        source: io::Error,
    },
    #[error("sim-core JSON failed while {operation}: {source}")]
    Json {
        operation: &'static str,
        #[source]
        source: serde_json::Error,
    },
    #[error("generated Rust did not contain modeled boundary {boundary}")]
    ModeledBoundaryMissing { boundary: &'static str },
}

pub type Result<T> = std::result::Result<T, SimCoreError>;

impl SimCoreError {
    pub(crate) fn io(operation: &'static str, source: io::Error) -> Self {
        Self::Io { operation, source }
    }

    pub(crate) fn json(operation: &'static str, source: serde_json::Error) -> Self {
        Self::Json { operation, source }
    }
}
