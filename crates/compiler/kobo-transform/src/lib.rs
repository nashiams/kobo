mod box_reason;
mod builder;
mod cfg;
mod classify;
mod clone_elision;
mod copy_scan;
mod escape;
mod finalize;
mod hint;
mod method_analysis;
mod options;
pub mod patterns;
mod small_clone;
pub mod strict_async;
mod tier_validate;
mod tiered;
mod transform;
pub(crate) mod warn_early;
pub(crate) mod strict;
#[cfg(test)]
mod strict_integration_tests;

pub use cfg::CfgGraph;
pub use options::TransformOptions;
pub use transform::build_kir;
pub use warn_early::detect_warn_early;
