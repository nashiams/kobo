/// @strict block lowering.
///
/// Invariants enforced here:
/// - guard all exits (normal, ?, break, continue)
/// - no `.borrow()` / `.borrow_mut()` inside the rewritten block body
/// - @strict keyword / #[__kobo_strict] never appear in output
/// - guard names are `__kobo_guard_{N}` where N comes from StrictGuardCounter
mod block;
mod cf;
mod func;
mod guard;

pub use block::lower_strict_block;
pub use func::lower_strict_fn;
pub use guard::StrictGuardCounter;
