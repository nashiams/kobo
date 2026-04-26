// DiagOwner<T> — script-mode ownership wrapper with optional borrow instrumentation.
//
// Two compile-time paths:
//   #[cfg(feature = "diag")]     — instrumented: counts borrows, detects aliasing, reports on Drop
//   #[cfg(not(feature = "diag"))] — zero-cost:    thin wrapper, no counters, no heap beyond T itself
//
// The runtime gate (`KOBO_DIAG=1`) controls whether Drop emits output.
// The feature gate controls whether counters exist at all.
//
// Contract C01: counter fields must be INSIDE an Rc alongside the RefCell value
// so that all clones share a single counter set. A field on the outer struct is
// NOT shared across Rc::clone. This implementation uses a separate Rc<DiagCounters>
// for the counter allocation, keeping it alive as long as any DiagOwner or any
// live DiagRef/DiagRefMut guard exists.
//
// Contract C02: this file may not import any kobo-* compiler crate.
// Contract C06: every counter increment uses saturating_add, never +=.

#[cfg(feature = "diag")]
use std::cell::Ref;
#[cfg(feature = "diag")]
use std::cell::RefMut;
use std::cell::{Cell, RefCell};
use std::ops::{Deref, DerefMut};
use std::rc::Rc;

use crate::thresholds::DiagThreshold;

// ---------------------------------------------------------------------------
// Shared counter allocation (instrumented path only)
// ---------------------------------------------------------------------------

/// Borrow counters allocated inside an `Rc` alongside the `RefCell` value.
///
/// All clones of a `DiagOwner` share a single `Rc<DiagCounters>` so the totals
/// reflect every borrow across every clone — not just the one that drops last.
#[cfg(feature = "diag")]
pub struct DiagCounters {
    pub borrow_count: Cell<u64>,
    pub mut_borrow_count: Cell<u64>,
    pub contention_count: Cell<u64>,
    /// Number of currently live `DiagRef`/`DiagRefMut` guards.
    pub active_borrows: Cell<u32>,
    pub saturated: Cell<bool>,
}

#[cfg(feature = "diag")]
impl DiagCounters {
    fn new() -> Rc<Self> {
        Rc::new(Self {
            borrow_count: Cell::new(0),
            mut_borrow_count: Cell::new(0),
            contention_count: Cell::new(0),
            active_borrows: Cell::new(0),
            saturated: Cell::new(false),
        })
    }

    /// Increment a u64 counter using saturating arithmetic. Sets `saturated`
    /// when the counter reaches u64::MAX. Contract C06.
    fn saturating_increment(counter: &Cell<u64>, saturated: &Cell<bool>) {
        let prev = counter.get();
        let next = prev.saturating_add(1);
        counter.set(next);
        if next == u64::MAX && prev != u64::MAX {
            saturated.set(true);
        }
    }

    /// Increment active_borrows using saturating arithmetic.
    fn increment_active(&self) {
        let prev = self.active_borrows.get();
        self.active_borrows.set(prev.saturating_add(1));
    }
}

// ---------------------------------------------------------------------------
// Guard types (instrumented path only)
// ---------------------------------------------------------------------------

/// Immutable borrow guard. Decrements `active_borrows` on drop.
///
/// `_counters` holds a clone of the `Rc<DiagCounters>` to keep the allocation
/// alive for the duration of the borrow, making the raw pointer safe.
#[cfg(feature = "diag")]
pub struct DiagRef<'a, V> {
    inner: Ref<'a, V>,
    /// Raw pointer into `DiagCounters.active_borrows`. Valid while `_counters` is alive.
    active_borrows: *const Cell<u32>,
    /// Keeps the `Rc<DiagCounters>` allocation alive for the guard's lifetime.
    _counters: Rc<DiagCounters>,
}

#[cfg(feature = "diag")]
impl<'a, V> Drop for DiagRef<'a, V> {
    fn drop(&mut self) {
        // SAFETY: `_counters` is still alive (fields drop in declaration order,
        // and we read the pointer before `_counters` is dropped by the implicit
        // struct drop glue that runs after this fn returns).
        unsafe { (*self.active_borrows).set((*self.active_borrows).get().saturating_sub(1)) }
    }
}

#[cfg(feature = "diag")]
impl<'a, V> Deref for DiagRef<'a, V> {
    type Target = V;
    fn deref(&self) -> &V {
        &self.inner
    }
}

/// Mutable borrow guard. Decrements `active_borrows` on drop.
#[cfg(feature = "diag")]
pub struct DiagRefMut<'a, V> {
    inner: RefMut<'a, V>,
    active_borrows: *const Cell<u32>,
    _counters: Rc<DiagCounters>,
}

#[cfg(feature = "diag")]
impl<'a, V> Drop for DiagRefMut<'a, V> {
    fn drop(&mut self) {
        // SAFETY: same reasoning as DiagRef::drop.
        unsafe { (*self.active_borrows).set((*self.active_borrows).get().saturating_sub(1)) }
    }
}

#[cfg(feature = "diag")]
impl<'a, V> Deref for DiagRefMut<'a, V> {
    type Target = V;
    fn deref(&self) -> &V {
        &self.inner
    }
}

#[cfg(feature = "diag")]
impl<'a, V> DerefMut for DiagRefMut<'a, V> {
    fn deref_mut(&mut self) -> &mut V {
        &mut self.inner
    }
}

// ---------------------------------------------------------------------------
// DiagOwner — instrumented path
// ---------------------------------------------------------------------------

/// Instrumented wrapper for `Rc<RefCell<T>>` bindings in diag mode.
///
/// Counters are shared across all clones via `Rc<DiagCounters>`.
/// `source_location` is a `&'static str` baked at codegen time — never heap-allocated.
/// Contract C01, C09.
#[cfg(feature = "diag")]
pub struct DiagOwner<T> {
    inner: T,
    counters: Rc<DiagCounters>,
    source_location: &'static str,
}

#[cfg(feature = "diag")]
impl<T> DiagOwner<T> {
    pub fn new(inner: T, source_location: &'static str) -> Self {
        Self {
            inner,
            counters: DiagCounters::new(),
            source_location,
        }
    }

    pub fn inner(&self) -> &T {
        &self.inner
    }

    #[cfg(test)]
    pub fn counters(&self) -> &Rc<DiagCounters> {
        &self.counters
    }
}

/// `borrow()` and `borrow_mut()` are only defined for `T = Rc<RefCell<V>>`.
/// All other method dispatch (Deref/DerefMut) works on any T.
#[cfg(feature = "diag")]
impl<V> DiagOwner<Rc<RefCell<V>>> {
    pub fn borrow(&self) -> DiagRef<'_, V> {
        self.counters.increment_active();
        DiagCounters::saturating_increment(&self.counters.borrow_count, &self.counters.saturated);
        let active_borrows_ptr: *const Cell<u32> = &self.counters.active_borrows;
        DiagRef {
            inner: self.inner.borrow(),
            active_borrows: active_borrows_ptr,
            _counters: Rc::clone(&self.counters),
        }
    }

    pub fn borrow_mut(&self) -> DiagRefMut<'_, V> {
        // Contention: borrow_mut attempted while ≥1 immutable borrow is live.
        // Count the attempt BEFORE calling inner.borrow_mut() so the counter is
        // visible even if RefCell panics. Trap 2: count the attempt, not success.
        if self.counters.active_borrows.get() > 0 {
            DiagCounters::saturating_increment(
                &self.counters.contention_count,
                &self.counters.saturated,
            );
        }
        // May panic here if there is an active immutable borrow.
        // active_borrows and mut_borrow_count are only incremented AFTER success
        // to avoid leaking the counter if the inner borrow panics.
        let inner_mut = self.inner.borrow_mut();
        DiagCounters::saturating_increment(
            &self.counters.mut_borrow_count,
            &self.counters.saturated,
        );
        self.counters.increment_active();
        let active_borrows_ptr: *const Cell<u32> = &self.counters.active_borrows;
        DiagRefMut {
            inner: inner_mut,
            active_borrows: active_borrows_ptr,
            _counters: Rc::clone(&self.counters),
        }
    }
}

#[cfg(feature = "diag")]
impl<T: Clone> Clone for DiagOwner<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            counters: Rc::clone(&self.counters),
            source_location: self.source_location,
        }
    }
}

#[cfg(feature = "diag")]
impl<T> Deref for DiagOwner<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.inner
    }
}

#[cfg(feature = "diag")]
impl<T> DerefMut for DiagOwner<T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.inner
    }
}

#[cfg(feature = "diag")]
impl<T> Drop for DiagOwner<T> {
    fn drop(&mut self) {
        // Runtime gate: emit when either of these is true:
        //   (a) KOBO_DIAG=1   — explicit opt-in in script mode
        //   (b) KOBO_CHECKED_MODE=1 — set by the driver when running a checked-mode binary
        //                              checked mode always emits DiagOwner output [G1-§1.4]
        // We do NOT check the feature gate here — feature="diag" controls counter existence;
        // this gate controls whether the output is suppressed at runtime.
        let explicit_opt_in = std::env::var("KOBO_DIAG").as_deref() == Ok("1");
        let checked_mode = std::env::var("KOBO_CHECKED_MODE").as_deref() == Ok("1");
        if !explicit_opt_in && !checked_mode {
            return;
        }

        // Only emit when this is the last owner (avoids double-reporting on clone).
        if Rc::strong_count(&self.counters) > 1 {
            return;
        }

        let threshold = DiagThreshold::read_env().borrow_count;
        let borrow_count = self.counters.borrow_count.get();
        let mut_borrow_count = self.counters.mut_borrow_count.get();
        let contention_count = self.counters.contention_count.get();
        let saturated = self.counters.saturated.get();

        let is_hot = borrow_count > threshold || mut_borrow_count > threshold;
        if !is_hot && contention_count == 0 {
            return;
        }

        // Structured [kobo-diag] output block. Format is a STABLE CONTRACT
        // frozen from v0.4.0 — do not change field names or order. (Trap 23)
        eprintln!(
            "\n[kobo-diag] {loc} — (Rc<RefCell<T>>)",
            loc = self.source_location,
        );
        eprintln!("  borrow_count:        {borrow_count}");
        eprintln!("  mut_borrow_count:    {mut_borrow_count}");
        eprintln!("  contention_count:    {contention_count}");
        eprintln!("  saturated:           {saturated}");
        eprintln!("  source_location:     {}", self.source_location);

        if is_hot {
            eprintln!("  → hot path detected — mut_borrow_count > threshold ({threshold})");
            eprintln!(
                "  → run `kobo perf {loc}` to see K0020 diagnostic",
                loc = self
                    .source_location
                    .trim_end_matches(|c: char| c.is_ascii_digit())
                    .trim_end_matches(':'),
            );
        }

        if contention_count > 0 {
            eprintln!(
                "  → aliasing detected — borrow_mut called while immutable borrow was live ({contention_count} times)"
            );
            eprintln!(
                "  → note: contention here means RefCell aliasing conflict, not thread contention"
            );
        }

        if saturated {
            eprintln!("  → note[K0021]: counter saturated — reported count is a lower bound; true count is higher");
        }
    }
}

// ---------------------------------------------------------------------------
// DiagOwner — zero-cost path
// ---------------------------------------------------------------------------

/// Zero-cost wrapper for non-diag builds. No counters, no heap beyond T itself.
///
/// `source_location` adds one pointer-width field (&'static str data pointer + len)
/// but no heap allocation — it is a string literal.
#[cfg(not(feature = "diag"))]
pub struct DiagOwner<T> {
    inner: T,
    source_location: &'static str,
}

#[cfg(not(feature = "diag"))]
impl<T> DiagOwner<T> {
    pub fn new(inner: T, source_location: &'static str) -> Self {
        Self {
            inner,
            source_location,
        }
    }

    pub fn inner(&self) -> &T {
        &self.inner
    }
}

#[cfg(not(feature = "diag"))]
impl<V> DiagOwner<Rc<RefCell<V>>> {
    pub fn borrow(&self) -> std::cell::Ref<'_, V> {
        self.inner.borrow()
    }

    pub fn borrow_mut(&self) -> std::cell::RefMut<'_, V> {
        self.inner.borrow_mut()
    }
}

#[cfg(not(feature = "diag"))]
impl<T: Clone> Clone for DiagOwner<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            source_location: self.source_location,
        }
    }
}

#[cfg(not(feature = "diag"))]
impl<T> Deref for DiagOwner<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.inner
    }
}

#[cfg(not(feature = "diag"))]
impl<T> DerefMut for DiagOwner<T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.inner
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    // Helper: create a DiagOwner<Rc<RefCell<i32>>> for testing.
    fn make_owner(value: i32) -> DiagOwner<Rc<RefCell<i32>>> {
        DiagOwner::new(Rc::new(RefCell::new(value)), "test.kobo:1")
    }

    // --- zero-cost path tests (always run) ---

    #[test]
    fn zero_cost_path_size_assertion() {
        #[cfg(not(feature = "diag"))]
        {
            // In zero-cost mode the wrapper adds only source_location (&'static str).
            assert_eq!(
                size_of::<DiagOwner<Rc<RefCell<i32>>>>(),
                size_of::<Rc<RefCell<i32>>>() + size_of::<&'static str>(),
            );
        }
    }

    // --- instrumented path tests (only compile with feature="diag") ---

    #[cfg(feature = "diag")]
    mod instrumented {
        use super::*;

        #[test]
        fn new_diag_owner_borrow_5_times_below_threshold_no_output() {
            let owner = make_owner(42);
            for _ in 0..5 {
                let _ = owner.borrow();
            }
            assert_eq!(owner.counters().borrow_count.get(), 5);
            assert_eq!(owner.counters().contention_count.get(), 0);
        }

        #[test]
        fn saturating_no_overflow() {
            let owner = make_owner(0);
            // Set counter near u64::MAX directly
            owner.counters().borrow_count.set(u64::MAX - 1);
            // One more borrow saturates
            let _ = owner.borrow();
            assert_eq!(owner.counters().borrow_count.get(), u64::MAX);
            assert!(
                owner.counters().saturated.get(),
                "saturated must be true at u64::MAX"
            );
            // Additional borrow must not wrap to 0
            let _ = owner.borrow();
            assert_eq!(
                owner.counters().borrow_count.get(),
                u64::MAX,
                "must not wrap past u64::MAX"
            );
        }

        #[test]
        fn contention_borrow_then_borrow_mut() {
            let owner = make_owner(0);
            // Hold an immutable borrow, then attempt a mutable borrow.
            // RefCell panics on the aliasing violation; we catch it to inspect the counter.
            let guard = owner.borrow();
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _ = owner.borrow_mut(); // will panic — RefCell aliasing violation
            }));
            // contention_count was incremented before inner.borrow_mut() panicked.
            assert_eq!(
                owner.counters().contention_count.get(),
                1,
                "contention_count must be 1 when borrow_mut is attempted while a borrow is live"
            );
            // active_borrows was NOT incremented (borrow_mut failed before that).
            assert_eq!(
                owner.counters().active_borrows.get(),
                1,
                "active_borrows must still be 1"
            );
            assert!(
                result.is_err(),
                "borrow_mut must panic with active immutable borrow"
            );
            drop(guard); // clean up
            assert_eq!(owner.counters().active_borrows.get(), 0);
        }

        #[test]
        fn active_borrows_reset_after_guard_drop() {
            let owner = make_owner(0);
            {
                let _guard = owner.borrow();
                assert_eq!(owner.counters().active_borrows.get(), 1);
            }
            assert_eq!(
                owner.counters().active_borrows.get(),
                0,
                "active_borrows must be 0 after DiagRef drops"
            );
        }

        #[test]
        fn sequential_borrows_no_contention() {
            let owner = make_owner(0);
            {
                let _guard = owner.borrow();
            }
            // Guard dropped before next borrow.
            {
                let _guard2 = owner.borrow_mut();
            }
            assert_eq!(
                owner.counters().contention_count.get(),
                0,
                "sequential borrows must not count as contention"
            );
            assert_eq!(owner.counters().active_borrows.get(), 0);
        }

        #[test]
        fn clone_shares_counters() {
            let owner = make_owner(0);
            let clone = owner.clone();
            // Borrow on clone — should show up in original's counter set.
            let _ = clone.borrow();
            assert_eq!(
                owner.counters().borrow_count.get(),
                1,
                "borrow on clone must increment shared counter"
            );
        }

        #[test]
        fn borrow_above_threshold_tracked_correctly() {
            let owner = make_owner(0);
            for _ in 0..20 {
                let _ = owner.borrow();
            }
            assert_eq!(owner.counters().borrow_count.get(), 20);
        }

        #[test]
        fn threshold_env_override() {
            // KOBO_DIAG_THRESHOLD is read by DiagThreshold::read_env via OnceLock.
            // We cannot reliably test env-var parsing in single-threaded tests without
            // process isolation. This test verifies the default threshold is 10_000.
            let threshold = DiagThreshold::default_threshold();
            assert_eq!(threshold, 10_000);
        }

        #[test]
        fn threshold_env_invalid_falls_back_to_default() {
            // When KOBO_DIAG_THRESHOLD is not set, default is 10_000.
            let threshold = DiagThreshold::parse_threshold_str("abc");
            assert_eq!(
                threshold, 10_000,
                "invalid string must fall back to 10_000 default"
            );
        }
    }
}
