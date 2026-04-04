mod diag_owner;
pub(crate) mod thresholds;

pub use diag_owner::DiagOwner;

#[cfg(feature = "diag")]
pub use diag_owner::{DiagCounters, DiagRef, DiagRefMut};
