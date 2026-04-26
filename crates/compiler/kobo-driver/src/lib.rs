mod config;
mod errors;
mod filesystem;
mod multi_file;
mod pipeline;
mod rustc;
mod session;
pub mod test_utils;

pub use config::{load_config, load_config_for, parse_kobo_config, ConfigError, KoboConfig, KoboMode};
pub use errors::DriverError;
pub use filesystem::{
    map_path_for, output_path_for, read_kobo_file, rs_path_for, write_map_file, write_rs_file,
    IoError,
};
pub use multi_file::{run_build_pipeline, BuildOutput};
pub use pipeline::{
    apply_lifetime_erasure, effective_mode, run_and_compile, run_check_pipeline,
    run_codegen_pipeline, run_kir_phase, run_pipeline, run_pipeline_ordering_check, CodegenArtifacts,
};
pub use rustc::binary_path_for;
pub use session::{is_inside_relaxed_fn, CompileSession};
