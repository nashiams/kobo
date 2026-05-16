pub mod backend;
pub mod core;
pub mod coverage;
pub mod digest;
pub mod error;
pub mod harness;
pub mod harness_manifest;
pub mod lower;
pub mod network;
pub mod scheduler;
pub mod storage;

pub use core::{
    run_compiler_semantics, run_full_depth, run_full_depth_from_program,
    run_semantics_from_program, BoundaryDecision, BoundaryPolicyChoice, EngineMode,
    ExecutionDigest, FullDepthRun, ModeledBoundary, ReplayGuarantee, RuntimeObligationSummary,
    ScenarioCoverage, ScenarioEvent, ScenarioFailure, ScenarioOperation, ScenarioOptions,
};
pub use error::{Result, SimCoreError};
pub use harness_manifest::HarnessManifest;
