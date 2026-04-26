//! Guard counter for deterministic `__kobo_guard_{N}` name generation.
//!
//! Contract C08: one instance per function, shared across ALL @strict blocks.

/// Monotonically-incrementing counter for guard name generation.
///
/// One instance per function — shared across ALL @strict blocks within that
/// function so that guard names never collide (Trap 16).
pub struct StrictGuardCounter {
    next_n: usize,
}

impl StrictGuardCounter {
    pub fn new() -> Self {
        Self { next_n: 0 }
    }

    /// Allocate the next guard index and advance the counter.
    pub fn next(&mut self) -> usize {
        let n = self.next_n;
        self.next_n += 1;
        n
    }
}

impl Default for StrictGuardCounter {
    fn default() -> Self {
        Self::new()
    }
}
