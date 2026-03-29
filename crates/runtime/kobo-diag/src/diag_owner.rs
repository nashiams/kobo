use std::cell::Cell;
use std::cell::RefCell;
use std::rc::Rc;

/// Script-mode ownership wrapper with runtime borrow instrumentation.
///
/// Generated `.rs` files import this type. It ships as a standalone crate
/// on crates.io — it has no dependency on the Kobo compiler crates.
///
/// Counters are `u64` with saturating arithmetic. On saturation,
/// `saturated` is set to `true` and a one-time warning is emitted to stderr.
pub struct DiagOwner<T> {
    inner: Rc<RefCell<T>>,
    borrow_count: Cell<u64>,
    mut_borrow_count: Cell<u64>,
    contention_count: Cell<u64>,
    saturated: Cell<bool>,
    source_location: &'static str,
}

impl<T> DiagOwner<T> {
    pub fn new(value: T, source_location: &'static str) -> Self {
        Self {
            inner: Rc::new(RefCell::new(value)),
            borrow_count: Cell::new(0),
            mut_borrow_count: Cell::new(0),
            contention_count: Cell::new(0),
            saturated: Cell::new(false),
            source_location,
        }
    }

    pub fn borrow(&self) -> std::cell::Ref<'_, T> {
        let new_count = self.borrow_count.get().saturating_add(1);
        if new_count == u64::MAX {
            self.mark_saturated();
        }
        self.borrow_count.set(new_count);
        self.inner.borrow()
    }

    pub fn borrow_mut(&self) -> std::cell::RefMut<'_, T> {
        let new_count = self.mut_borrow_count.get().saturating_add(1);
        if new_count == u64::MAX {
            self.mark_saturated();
        }
        self.mut_borrow_count.set(new_count);
        self.inner.borrow_mut()
    }

    fn mark_saturated(&self) {
        if !self.saturated.get() {
            self.saturated.set(true);
            // §9 — one-time saturation warning
            eprintln!(
                "[kobo:K0021] borrow counter saturated at {} — counter capped at u64::MAX. Consider @strict.",
                self.source_location
            );
        }
    }
}

impl<T> Drop for DiagOwner<T> {
    fn drop(&mut self) {
        const HOT_THRESHOLD: u64 = 10_000;
        let borrows = self.borrow_count.get();
        let mut_borrows = self.mut_borrow_count.get();
        let contention = self.contention_count.get();
        let saturated = self.saturated.get();

        if contention > 0 || borrows > HOT_THRESHOLD {
            eprintln!(
                "[kobo:diag] {}: {} borrows, {} mut, {} contention{}",
                self.source_location,
                borrows,
                mut_borrows,
                contention,
                if saturated { " (SATURATED)" } else { "" }
            );
        }
    }
}
