mod box_reason;
mod builder;
mod cfg;
mod classify;
mod clone_elision;
mod escape;
mod finalize;
mod hint;
mod options;
mod small_clone;
mod tier_validate;
mod tiered;
mod transform;

pub use cfg::CfgGraph;
pub use options::TransformOptions;
pub use transform::build_kir;
