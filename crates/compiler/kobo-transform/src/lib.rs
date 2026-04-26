mod box_reason;
mod builder;
mod cfg;
mod classify;
#[cfg(test)]
mod classify_edge_tests;
mod clone_elision;
mod copy_scan;
mod escape;
mod finalize;
mod hint;
pub mod lifetime_erase;
mod method_analysis;
mod options;
pub mod patterns;
mod small_clone;
pub(crate) mod strict;
pub mod strict_async;
#[cfg(test)]
mod strict_integration_tests;
mod tier_validate;
mod tiered;
mod transform;
pub(crate) mod warn_early;

pub use cfg::{compute_binding_liveness, stamp_cfg_and_liveness, BindingLiveness, CfgGraph};
pub use options::TransformOptions;
pub use transform::build_kir;
pub use warn_early::detect_warn_early;
