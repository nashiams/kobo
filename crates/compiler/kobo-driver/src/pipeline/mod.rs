#![allow(clippy::result_unit_err)]

mod analysis;
mod codegen;
mod compile;
mod parse;
mod solver;
mod util;

pub use analysis::{run_check_pipeline, run_pipeline_ordering_check};
pub use codegen::{run_codegen_pipeline, run_pipeline, CodegenArtifacts};
pub use compile::{run_and_compile, run_and_compile_with_lifetime_erasure};
pub use parse::run_kir_phase;
pub use util::{
    apply_lifetime_erasure, effective_mode, extract_before_borrow_rewrite,
    lifetime_erasure_debt_report,
};
