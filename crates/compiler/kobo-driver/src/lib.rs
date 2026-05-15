mod config;
mod errors;
mod filesystem;
mod multi_file;
mod pipeline;
mod query;
mod rustc;
mod session;
pub mod test_utils;

pub use config::{
    load_config, load_config_for, parse_kobo_config, ConfigError, KoboConfig, KoboMode,
};
pub use errors::DriverError;
pub use filesystem::{
    map_path_for, output_path_for, read_kobo_file, rs_path_for, write_map_file, write_rs_file,
    IoError,
};
pub use multi_file::{run_build_pipeline, BuildOutput};
pub use pipeline::{
    apply_lifetime_erasure, effective_mode, extract_before_borrow_rewrite,
    lifetime_erasure_debt_report, run_and_compile, run_and_compile_with_lifetime_erasure,
    run_check_pipeline, run_codegen_pipeline, run_kir_phase, run_pipeline,
    run_pipeline_ordering_check, apply_error_policy_sites, CodegenArtifacts, ErrorPolicySite,
};
pub use query::{
    CodegenOutput as QueryCodegenOutput, DiagnosticOutput, KirOutput, ParsedOutput, QueryMetrics,
    QuerySession, SolverOutput,
};
pub use rustc::binary_path_for;
pub use session::{is_inside_relaxed_fn, CompileSession};
