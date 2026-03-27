/// Ownership tier assigned to a KIR node. Determines what wrapper type (if any)
/// `kobo-codegen` emits for the corresponding binding in the generated `.rs` file.
///
/// Variants are ordered roughly by cost: `PlainOwned` is zero overhead,
/// `ArcMutShared` is maximum overhead. `Undecided` is the initial state before
/// `kobo-migrate` runs.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub enum OwnershipTier {
    /// `T` — moved. No wrapper, no allocation. Zero overhead.
    PlainOwned,
    /// `Box<T>` — single owner, heap-allocated.
    BoxOwned,
    /// `Rc<T>` — read-only shared reference count. No mutation.
    RcShared,
    /// `Arc<T>` — read-only shared, thread-safe. No mutation.
    ArcShared,
    /// `Rc<RefCell<T>>` — mutable shared, single-threaded. Script-mode default.
    RcMutShared,
    /// `Arc<Mutex<T>>` — mutable shared, thread-safe. Requires `#[kobo::async_shared]`.
    ArcMutShared,
    /// `ScopedHandle<T>` — resource kind with enforced single ownership.
    Scoped,
    /// Not yet resolved. `kobo-migrate` produces a `SolutionMap` that resolves this.
    ///
    /// `kobo-codegen` panics in debug mode if it encounters `Undecided` without
    /// a corresponding `SolutionMap` entry.
    Undecided,
}

impl OwnershipTier {
    pub fn is_shared(&self) -> bool {
        matches!(
            self,
            OwnershipTier::RcShared
                | OwnershipTier::ArcShared
                | OwnershipTier::RcMutShared
                | OwnershipTier::ArcMutShared
        )
    }

    pub fn is_thread_safe(&self) -> bool {
        matches!(self, OwnershipTier::ArcShared | OwnershipTier::ArcMutShared)
    }

    pub fn is_decided(&self) -> bool {
        !matches!(self, OwnershipTier::Undecided)
    }
}
