// DiagThreshold — reads KOBO_DIAG_THRESHOLD once per process via OnceLock.
//
// Contract C02: no kobo-* imports permitted.
// Trap 22: parse errors fall back to default with eprintln, never panic.

use std::sync::OnceLock;

static THRESHOLD_CACHE: OnceLock<u64> = OnceLock::new();

/// Hot-borrow threshold loaded from `KOBO_DIAG_THRESHOLD` at first use.
///
/// Default is 10 000. An invalid env-var value prints a one-time warning to
/// stderr and falls back to the default — it never panics.
pub struct DiagThreshold {
    pub borrow_count: u64,
}

impl DiagThreshold {
    /// Return the threshold, reading and caching `KOBO_DIAG_THRESHOLD` on the
    /// first call.
    pub fn read_env() -> Self {
        let borrow_count = *THRESHOLD_CACHE.get_or_init(|| {
            match std::env::var("KOBO_DIAG_THRESHOLD") {
                Ok(val) => Self::parse_threshold_str(&val),
                Err(_) => Self::default_threshold(),
            }
        });
        Self { borrow_count }
    }

    /// Parse a threshold string. Returns the default on failure.
    ///
    /// Exposed as `pub(crate)` for unit-testing the fallback path without
    /// relying on env-var isolation between test processes.
    pub fn parse_threshold_str(s: &str) -> u64 {
        match s.trim().parse::<u64>() {
            Ok(v) => v,
            Err(_) => {
                eprintln!(
                    "[kobo-diag] KOBO_DIAG_THRESHOLD {:?} is not a valid u64 — \
                     using default {}",
                    s,
                    Self::default_threshold(),
                );
                Self::default_threshold()
            }
        }
    }

    /// The compile-time default threshold.
    pub const fn default_threshold() -> u64 {
        10_000
    }
}
