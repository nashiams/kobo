#![allow(clippy::result_unit_err)]

pub(crate) mod analysis;
pub(crate) mod codegen;
mod compile;
mod parse;
mod solver;
mod support;

pub use analysis::{run_check_pipeline, run_pipeline_ordering_check};
pub use codegen::{
    apply_error_policy_sites, run_codegen_pipeline, run_pipeline, CodegenArtifacts, ErrorPolicySite,
};
pub use compile::{run_and_compile, run_and_compile_with_lifetime_erasure};
pub use parse::run_kir_phase;
pub use support::{
    apply_lifetime_erasure, effective_guarantee_policy, extract_before_borrow_rewrite,
    lifetime_erasure_debt_report,
};
