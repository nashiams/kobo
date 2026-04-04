/// @strict capture analysis — P3 transform strict module.
///
/// Re-exports the three analysis functions used by `build_kir`.
pub mod boundary;
pub mod capture;
pub mod nesting;

pub use boundary::validate_strict_boundary;
pub use capture::analyze_strict_capture_set;
pub use nesting::flatten_nested_strict;
