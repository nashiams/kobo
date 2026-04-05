/// @strict capture analysis — P3 transform strict module.
///
/// Re-exports the analysis functions used by `build_kir`.
mod alias_scan;
pub mod boundary;
pub mod capture;
mod capture_visitor;
mod closure_scan;
mod labeled_scan;
mod move_scan;
pub mod nesting;
pub(crate) mod span_convert;

pub use boundary::validate_strict_boundary;
pub use capture::analyze_strict_capture_set;
pub use nesting::flatten_nested_strict;
