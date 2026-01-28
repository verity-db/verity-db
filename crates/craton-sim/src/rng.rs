//! Deterministic random number generation for simulation testing.
//!
//! The `SimRng` wraps a seedable RNG to ensure reproducibility. Given the
//! same seed, the simulation will produce the exact same sequence of
//! "random" values.
//!
//! # Example
//!
//! ```
//! use craton_sim::SimRng;
//!
//! let mut rng = SimRng::new(42);
//!
//! // Generate deterministic "random" values
//! let a = rng.next_u64();
//! let b = rng.next_u64();
//!
//! // Same seed produces same sequence
//! let mut rng2 = SimRng::new(42);
//! assert_eq!(rng2.next_u64(), a);
//! assert_eq!(rng2.next_u64(), b);
//! ```

use rand::rngs::SmallRng;
use rand::{Rng, SeedableRng};

/// Deterministic random number generator for simulation.
///
/// Wraps `SmallRng` for fast, reproducible random number generation.
/// The same seed always produces the same sequence of values.
#[derive(Debug)]
pub struct SimRng {
    /// The underlying RNG.
    inner: SmallRng,
    /// The seed used to create this RNG.
    seed: u64,
}

impl SimRng {
    /// Creates a new RNG with the specified seed.
    pub fn new(seed: u64) -> Self {
        Self {
            inner: SmallRng::seed_from_u64(seed),
            seed,
        }
    }

    /// Returns the seed used to create this RNG.
    pub fn seed(&self) -> u64 {
        self.seed
    }

    /// Generates a random `u64`.
    #[inline]
    pub fn next_u64(&mut self) -> u64 {
        // `gen` is a reserved keyword in Rust 2024, use raw identifier
        self.inner.r#gen()
    }

    /// Generates a random `u32`.
    #[inline]
    pub fn next_u32(&mut self) -> u32 {
        self.inner.r#gen()
    }

    /// Generates a random `bool`.
    #[inline]
    pub fn next_bool(&mut self) -> bool {
        self.inner.r#gen()
    }

    /// Generates a random `f64` in the range `[0.0, 1.0)`.
    #[inline]
    pub fn next_f64(&mut self) -> f64 {
        self.inner.r#gen()
    }

    /// Generates a random `bool` with the given probability of being `true`.
    ///
    /// # Arguments
    ///
    /// * `probability` - Probability of returning `true` (0.0 to 1.0)
    #[inline]
    pub fn next_bool_with_probability(&mut self, probability: f64) -> bool {
        debug_assert!(
            (0.0..=1.0).contains(&probability),
            "probability must be 0.0 to 1.0"
        );
        self.next_f64() < probability
    }

    /// Generates a random `usize` in the range `[0, max)`.
    ///
    /// # Panics
    ///
    /// Panics if `max` is 0.
    #[inline]
    pub fn next_usize(&mut self, max: usize) -> usize {
        debug_assert!(max > 0, "max must be > 0");
        self.inner.gen_range(0..max)
    }

    /// Generates a random `u64` in the range `[min, max)`.
    ///
    /// # Panics
    ///
    /// Panics if `min >= max`.
    #[inline]
    pub fn next_u64_range(&mut self, min: u64, max: u64) -> u64 {
        debug_assert!(min < max, "min must be < max");
        self.inner.gen_range(min..max)
    }

    /// Generates a random delay in nanoseconds within the given range.
    ///
    /// Useful for simulating network latency, disk I/O, etc.
    ///
    /// # Arguments
    ///
    /// * `min_ns` - Minimum delay in nanoseconds
    /// * `max_ns` - Maximum delay in nanoseconds (exclusive)
    #[inline]
    pub fn delay_ns(&mut self, min_ns: u64, max_ns: u64) -> u64 {
        self.next_u64_range(min_ns, max_ns)
    }

    /// Selects a random element from a slice.
    ///
    /// # Panics
    ///
    /// Panics if the slice is empty.
    pub fn choose<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        debug_assert!(!items.is_empty(), "cannot choose from empty slice");
        let idx = self.next_usize(items.len());
        &items[idx]
    }

    /// Shuffles a slice in place using the Fisher-Yates algorithm.
    pub fn shuffle<T>(&mut self, items: &mut [T]) {
        for i in (1..items.len()).rev() {
            let j = self.next_usize(i + 1);
            items.swap(i, j);
        }
    }

    /// Fills a byte slice with random bytes.
    pub fn fill_bytes(&mut self, dest: &mut [u8]) {
        self.inner.fill(dest);
    }

    /// Forks a new RNG with a derived seed.
    ///
    /// Useful for creating sub-RNGs for different simulation components
    /// while maintaining determinism.
    pub fn fork(&mut self) -> SimRng {
        let new_seed = self.next_u64();
        SimRng::new(new_seed)
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_same_sequence() {
        let mut rng1 = SimRng::new(12345);
        let mut rng2 = SimRng::new(12345);

        for _ in 0..100 {
            assert_eq!(rng1.next_u64(), rng2.next_u64());
        }
    }

    #[test]
    fn different_seed_different_sequence() {
        let mut rng1 = SimRng::new(12345);
        let mut rng2 = SimRng::new(54321);

        // Highly unlikely to be equal (but technically possible)
        let same = (0..10).all(|_| rng1.next_u64() == rng2.next_u64());
        assert!(!same);
    }

    #[test]
    fn seed_is_preserved() {
        let rng = SimRng::new(42);
        assert_eq!(rng.seed(), 42);
    }

    #[test]
    fn next_usize_in_range() {
        let mut rng = SimRng::new(0);

        for _ in 0..100 {
            let val = rng.next_usize(10);
            assert!(val < 10);
        }
    }

    #[test]
    fn next_u64_range_in_range() {
        let mut rng = SimRng::new(0);

        for _ in 0..100 {
            let val = rng.next_u64_range(50, 100);
            assert!((50..100).contains(&val));
        }
    }

    #[test]
    fn next_bool_with_probability() {
        let mut rng = SimRng::new(0);

        // With probability 1.0, should always be true
        for _ in 0..100 {
            assert!(rng.next_bool_with_probability(1.0));
        }

        // With probability 0.0, should always be false
        for _ in 0..100 {
            assert!(!rng.next_bool_with_probability(0.0));
        }
    }

    #[test]
    fn choose_selects_from_slice() {
        let mut rng = SimRng::new(0);
        let items = vec![1, 2, 3, 4, 5];

        for _ in 0..100 {
            let chosen = rng.choose(&items);
            assert!(items.contains(chosen));
        }
    }

    #[test]
    fn shuffle_is_deterministic() {
        let mut items1 = vec![1, 2, 3, 4, 5];
        let mut items2 = vec![1, 2, 3, 4, 5];

        let mut rng1 = SimRng::new(42);
        let mut rng2 = SimRng::new(42);

        rng1.shuffle(&mut items1);
        rng2.shuffle(&mut items2);

        assert_eq!(items1, items2);
    }

    #[test]
    fn fill_bytes_deterministic() {
        let mut rng1 = SimRng::new(42);
        let mut rng2 = SimRng::new(42);

        let mut buf1 = [0u8; 32];
        let mut buf2 = [0u8; 32];

        rng1.fill_bytes(&mut buf1);
        rng2.fill_bytes(&mut buf2);

        assert_eq!(buf1, buf2);
    }

    #[test]
    fn fork_produces_deterministic_child() {
        let mut parent1 = SimRng::new(42);
        let mut parent2 = SimRng::new(42);

        let mut child1 = parent1.fork();
        let mut child2 = parent2.fork();

        for _ in 0..50 {
            assert_eq!(child1.next_u64(), child2.next_u64());
        }
    }

    #[test]
    fn fork_advances_parent() {
        let mut rng1 = SimRng::new(42);
        let mut rng2 = SimRng::new(42);

        // Fork from rng1
        let _child = rng1.fork();

        // rng1 has advanced, so next value differs from rng2
        assert_ne!(rng1.next_u64(), rng2.next_u64());
    }

    #[test]
    fn delay_ns_in_range() {
        let mut rng = SimRng::new(0);

        for _ in 0..100 {
            let delay = rng.delay_ns(1_000_000, 10_000_000);
            assert!((1_000_000..10_000_000).contains(&delay));
        }
    }
}
