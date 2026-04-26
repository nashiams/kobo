/// @strict block lowering for v0.5.
///
/// Contracts enforced here:
/// - C02: guard all exits (normal, ?, break, continue)
/// - C04: no .borrow()/.borrow_mut() inside the rewritten block body
/// - C06: @strict keyword / #[__kobo_strict] never appear in output
/// - C08: guard names are `__kobo_guard_{N}` where N comes from StrictGuardCounter
mod block;
mod cf;
mod func;
mod guard;

pub use block::lower_strict_block;
pub use func::lower_strict_fn;
pub use guard::StrictGuardCounter;
