//! Simulated discrete clock for deterministic testing.
//!
//! The `SimClock` provides nanosecond-precision simulated time that advances
//! only when explicitly told to. This enables:
//!
//! - **Determinism**: Time doesn't advance randomly during execution
//! - **Compression**: Run years of simulated time in seconds of real time
//! - **Reproducibility**: Same sequence of events → same timing
//!
//! # Example
//!
//! ```
//! use vdb_sim::SimClock;
//!
//! let mut clock = SimClock::new();
//! assert_eq!(clock.now(), 0);
//!
//! clock.advance_by(1_000_000); // 1ms
//! assert_eq!(clock.now(), 1_000_000);
//!
//! clock.advance_to(5_000_000); // 5ms
//! assert_eq!(clock.now(), 5_000_000);
//! ```

/// Simulated clock with nanosecond precision.
///
/// Time only advances when explicitly requested, making all timing
/// deterministic and reproducible.
#[derive(Debug, Clone)]
pub struct SimClock {
    /// Current time in nanoseconds since simulation start.
    now_ns: u64,
}

impl SimClock {
    /// Creates a new clock starting at time zero.
    pub fn new() -> Self {
        Self { now_ns: 0 }
    }

    /// Creates a clock starting at the specified time.
    pub fn at(now_ns: u64) -> Self {
        Self { now_ns }
    }

    /// Returns the current simulated time in nanoseconds.
    #[inline]
    pub fn now(&self) -> u64 {
        self.now_ns
    }

    /// Returns the current time as milliseconds (for convenience).
    #[inline]
    pub fn now_ms(&self) -> u64 {
        self.now_ns / 1_000_000
    }

    /// Advances the clock by the specified number of nanoseconds.
    ///
    /// # Panics
    ///
    /// Debug builds panic on overflow.
    pub fn advance_by(&mut self, delta_ns: u64) {
        self.now_ns = self
            .now_ns
            .checked_add(delta_ns)
            .expect("clock overflow");
    }

    /// Advances the clock to the specified time.
    ///
    /// # Panics
    ///
    /// Debug builds panic if the target time is before the current time.
    pub fn advance_to(&mut self, target_ns: u64) {
        debug_assert!(
            target_ns >= self.now_ns,
            "cannot go back in time: current={}, target={}",
            self.now_ns,
            target_ns
        );
        self.now_ns = target_ns;
    }
}

impl Default for SimClock {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Time Conversion Helpers
// ============================================================================

/// Converts milliseconds to nanoseconds.
#[inline]
pub const fn ms_to_ns(ms: u64) -> u64 {
    ms * 1_000_000
}

/// Converts seconds to nanoseconds.
#[inline]
pub const fn sec_to_ns(sec: u64) -> u64 {
    sec * 1_000_000_000
}

/// Converts nanoseconds to milliseconds (truncating).
#[inline]
pub const fn ns_to_ms(ns: u64) -> u64 {
    ns / 1_000_000
}

/// Converts nanoseconds to seconds (truncating).
#[inline]
pub const fn ns_to_sec(ns: u64) -> u64 {
    ns / 1_000_000_000
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_starts_at_zero() {
        let clock = SimClock::new();
        assert_eq!(clock.now(), 0);
    }

    #[test]
    fn clock_starts_at_specified_time() {
        let clock = SimClock::at(1_000_000);
        assert_eq!(clock.now(), 1_000_000);
    }

    #[test]
    fn advance_by_works() {
        let mut clock = SimClock::new();

        clock.advance_by(1_000_000);
        assert_eq!(clock.now(), 1_000_000);

        clock.advance_by(500_000);
        assert_eq!(clock.now(), 1_500_000);
    }

    #[test]
    fn advance_to_works() {
        let mut clock = SimClock::new();

        clock.advance_to(5_000_000);
        assert_eq!(clock.now(), 5_000_000);

        clock.advance_to(10_000_000);
        assert_eq!(clock.now(), 10_000_000);
    }

    #[test]
    fn advance_to_same_time_is_ok() {
        let mut clock = SimClock::at(1_000_000);
        clock.advance_to(1_000_000); // No change
        assert_eq!(clock.now(), 1_000_000);
    }

    #[test]
    #[should_panic(expected = "cannot go back in time")]
    fn advance_to_past_panics() {
        let mut clock = SimClock::at(5_000_000);
        clock.advance_to(1_000_000); // Should panic
    }

    #[test]
    fn now_ms_conversion() {
        let clock = SimClock::at(5_500_000); // 5.5ms
        assert_eq!(clock.now_ms(), 5);
    }

    #[test]
    fn time_conversion_helpers() {
        assert_eq!(ms_to_ns(1), 1_000_000);
        assert_eq!(sec_to_ns(1), 1_000_000_000);
        assert_eq!(ns_to_ms(1_500_000), 1);
        assert_eq!(ns_to_sec(1_500_000_000), 1);
    }

    #[test]
    fn advance_by_zero_is_noop() {
        let mut clock = SimClock::at(1_000_000);
        clock.advance_by(0);
        assert_eq!(clock.now(), 1_000_000);
    }
}
