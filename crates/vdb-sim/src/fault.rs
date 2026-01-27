//! Advanced fault injection for simulation testing.
//!
//! This module provides sophisticated fault injection patterns inspired by
//! `FoundationDB` and `TigerBeetle`'s testing approaches.
//!
//! # Fault Types
//!
//! ## Network Faults
//!
//! - **Swizzle-clogging**: Randomly clog/unclog network connections to simulate
//!   intermittent connectivity issues.
//! - **Asymmetric partitions**: One-way network failures where A can reach B
//!   but B cannot reach A.
//!
//! ## Gray Failures
//!
//! Gray failures are partial failures that are harder to detect than complete
//! failures:
//!
//! - **Slow responses**: Node responds but with high latency
//! - **Intermittent failures**: Node fails some requests but not others
//! - **Partial functionality**: Node can read but not write, or vice versa
//!
//! ## Storage Faults
//!
//! - **Seen but corrupt**: Data was written but corrupted on disk
//! - **Not seen**: Write was lost (e.g., crashed before fsync)
//! - **Phantom writes**: Write appears successful but data is wrong

use crate::rng::SimRng;

// ============================================================================
// Swizzle-Clogging (Network)
// ============================================================================

/// Controls swizzle-clogging behavior for a network link.
///
/// Swizzle-clogging randomly clogs and unclogs network connections,
/// simulating intermittent network issues like congestion or flaky links.
#[derive(Debug, Clone)]
pub struct SwizzleClogger {
    /// Probability of transitioning from unclogged to clogged (per check).
    pub clog_probability: f64,
    /// Probability of transitioning from clogged to unclogged (per check).
    pub unclog_probability: f64,
    /// Current clogged state per link: (from, to) -> clogged flag
    clogged_links: std::collections::HashMap<(u64, u64), bool>,
    /// When clogged, messages are delayed by this factor (1.0 = no change).
    pub delay_factor: f64,
    /// When clogged, probability of dropping messages.
    pub clogged_drop_probability: f64,
}

impl SwizzleClogger {
    /// Creates a new swizzle-clogger with the given parameters.
    pub fn new(
        clog_probability: f64,
        unclog_probability: f64,
        delay_factor: f64,
        clogged_drop_probability: f64,
    ) -> Self {
        debug_assert!(
            (0.0..=1.0).contains(&clog_probability),
            "clog_probability must be 0.0 to 1.0"
        );
        debug_assert!(
            (0.0..=1.0).contains(&unclog_probability),
            "unclog_probability must be 0.0 to 1.0"
        );
        debug_assert!(delay_factor >= 1.0, "delay_factor must be >= 1.0");
        debug_assert!(
            (0.0..=1.0).contains(&clogged_drop_probability),
            "clogged_drop_probability must be 0.0 to 1.0"
        );

        Self {
            clog_probability,
            unclog_probability,
            clogged_links: std::collections::HashMap::new(),
            delay_factor,
            clogged_drop_probability,
        }
    }

    /// Creates a mild swizzle-clogger (10% clog, 50% unclog, 2x delay).
    pub fn mild() -> Self {
        Self::new(0.1, 0.5, 2.0, 0.1)
    }

    /// Creates an aggressive swizzle-clogger (30% clog, 20% unclog, 10x delay).
    pub fn aggressive() -> Self {
        Self::new(0.3, 0.2, 10.0, 0.5)
    }

    /// Checks if a link is currently clogged.
    pub fn is_clogged(&self, from: u64, to: u64) -> bool {
        *self.clogged_links.get(&(from, to)).unwrap_or(&false)
    }

    /// Updates the clog state for a link and returns whether it changed.
    pub fn update(&mut self, from: u64, to: u64, rng: &mut SimRng) -> bool {
        let currently_clogged = self.is_clogged(from, to);
        let new_state = if currently_clogged {
            // Currently clogged - maybe unclog
            !rng.next_bool_with_probability(self.unclog_probability)
        } else {
            // Currently unclogged - maybe clog
            rng.next_bool_with_probability(self.clog_probability)
        };

        let changed = currently_clogged != new_state;
        self.clogged_links.insert((from, to), new_state);
        changed
    }

    /// Applies clogging effects to a message delay.
    ///
    /// Returns the adjusted delay and whether to drop the message.
    #[allow(clippy::cast_sign_loss, clippy::cast_precision_loss)]
    pub fn apply(&self, from: u64, to: u64, delay_ns: u64, rng: &mut SimRng) -> (u64, bool) {
        if self.is_clogged(from, to) {
            let adjusted_delay = (delay_ns as f64 * self.delay_factor) as u64;
            let should_drop = rng.next_bool_with_probability(self.clogged_drop_probability);
            (adjusted_delay, should_drop)
        } else {
            (delay_ns, false)
        }
    }

    /// Returns the number of currently clogged links.
    pub fn clogged_count(&self) -> usize {
        self.clogged_links.values().filter(|&&v| v).count()
    }

    /// Resets all clog states.
    pub fn reset(&mut self) {
        self.clogged_links.clear();
    }
}

impl Default for SwizzleClogger {
    fn default() -> Self {
        Self::mild()
    }
}

// ============================================================================
// Gray Failure Injection
// ============================================================================

/// Mode of gray failure for a node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrayFailureMode {
    /// Node is healthy (no gray failure).
    Healthy,
    /// Node responds slowly (high latency).
    Slow {
        /// Latency multiplier (e.g., 10.0 = 10x normal latency).
        latency_multiplier: u32,
    },
    /// Node fails intermittently.
    Intermittent {
        /// Probability of failure per operation.
        failure_probability_percent: u8,
    },
    /// Node can only perform certain operations.
    PartialFunction {
        /// Can the node perform reads?
        can_read: bool,
        /// Can the node perform writes?
        can_write: bool,
    },
    /// Node is completely unresponsive (but not crashed).
    Unresponsive,
}

/// Gray failure injector for nodes.
///
/// Gray failures are partial failures that are harder to detect than
/// complete failures. They include slow responses, intermittent failures,
/// and partial functionality.
#[derive(Debug)]
pub struct GrayFailureInjector {
    /// Failure mode per node.
    node_modes: std::collections::HashMap<u64, GrayFailureMode>,
    /// Probability of transitioning to a gray failure mode.
    pub failure_probability: f64,
    /// Probability of recovering from a gray failure.
    pub recovery_probability: f64,
}

impl GrayFailureInjector {
    /// Creates a new gray failure injector.
    pub fn new(failure_probability: f64, recovery_probability: f64) -> Self {
        debug_assert!(
            (0.0..=1.0).contains(&failure_probability),
            "failure_probability must be 0.0 to 1.0"
        );
        debug_assert!(
            (0.0..=1.0).contains(&recovery_probability),
            "recovery_probability must be 0.0 to 1.0"
        );

        Self {
            node_modes: std::collections::HashMap::new(),
            failure_probability,
            recovery_probability,
        }
    }

    /// Gets the current failure mode for a node.
    pub fn get_mode(&self, node_id: u64) -> GrayFailureMode {
        self.node_modes
            .get(&node_id)
            .copied()
            .unwrap_or(GrayFailureMode::Healthy)
    }

    /// Sets the failure mode for a node.
    pub fn set_mode(&mut self, node_id: u64, mode: GrayFailureMode) {
        self.node_modes.insert(node_id, mode);
    }

    /// Updates failure states for all nodes.
    ///
    /// Returns list of nodes whose state changed.
    pub fn update_all(
        &mut self,
        node_ids: &[u64],
        rng: &mut SimRng,
    ) -> Vec<(u64, GrayFailureMode, GrayFailureMode)> {
        let mut changes = Vec::new();

        for &node_id in node_ids {
            let old_mode = self.get_mode(node_id);
            let new_mode = if old_mode == GrayFailureMode::Healthy {
                // Maybe enter a failure mode
                if rng.next_bool_with_probability(self.failure_probability) {
                    Self::random_failure_mode(rng)
                } else {
                    GrayFailureMode::Healthy
                }
            } else {
                // Maybe recover
                if rng.next_bool_with_probability(self.recovery_probability) {
                    GrayFailureMode::Healthy
                } else {
                    old_mode
                }
            };

            if old_mode != new_mode {
                self.set_mode(node_id, new_mode);
                changes.push((node_id, old_mode, new_mode));
            }
        }

        changes
    }

    /// Generates a random failure mode.
    fn random_failure_mode(rng: &mut SimRng) -> GrayFailureMode {
        match rng.next_usize(4) {
            0 => GrayFailureMode::Slow {
                latency_multiplier: (rng.next_usize(10) + 2) as u32, // 2x to 11x
            },
            1 => GrayFailureMode::Intermittent {
                failure_probability_percent: (rng.next_usize(50) + 10) as u8, // 10% to 59%
            },
            2 => GrayFailureMode::PartialFunction {
                can_read: rng.next_bool(),
                can_write: rng.next_bool(),
            },
            3 => GrayFailureMode::Unresponsive,
            _ => unreachable!(),
        }
    }

    /// Checks if an operation should succeed for a node.
    ///
    /// Returns whether to proceed and the latency multiplier.
    pub fn check_operation(
        &self,
        node_id: u64,
        is_write: bool,
        rng: &mut SimRng,
    ) -> (bool, u32) {
        match self.get_mode(node_id) {
            GrayFailureMode::Healthy => (true, 1),
            GrayFailureMode::Slow { latency_multiplier } => (true, latency_multiplier),
            GrayFailureMode::Intermittent {
                failure_probability_percent,
            } => {
                let should_fail =
                    rng.next_bool_with_probability(f64::from(failure_probability_percent) / 100.0);
                (!should_fail, 1)
            }
            GrayFailureMode::PartialFunction {
                can_read,
                can_write,
            } => {
                let can_proceed = if is_write { can_write } else { can_read };
                (can_proceed, 1)
            }
            GrayFailureMode::Unresponsive => (false, 1),
        }
    }

    /// Returns the number of nodes in a gray failure state.
    pub fn failing_count(&self) -> usize {
        self.node_modes
            .values()
            .filter(|&&m| m != GrayFailureMode::Healthy)
            .count()
    }

    /// Resets all nodes to healthy state.
    pub fn reset(&mut self) {
        self.node_modes.clear();
    }
}

impl Default for GrayFailureInjector {
    fn default() -> Self {
        Self::new(0.05, 0.2) // 5% chance of failure, 20% chance of recovery
    }
}

// ============================================================================
// Enhanced Storage Faults
// ============================================================================

/// Type of storage fault.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageFaultType {
    /// Write was never persisted (crashed before fsync).
    NotSeen,
    /// Write was persisted but data is corrupted.
    SeenButCorrupt,
    /// Write appears successful but contains wrong data.
    PhantomWrite,
    /// Read returns stale data (from before a write).
    StaleRead,
}

/// Fault state for a storage block.
#[derive(Debug, Clone)]
pub struct BlockFaultState {
    /// Type of fault affecting this block.
    pub fault_type: Option<StorageFaultType>,
    /// Original data before corruption (for debugging).
    pub original_checksum: Option<u32>,
    /// Time when fault was injected.
    pub injected_at_ns: u64,
}

/// Enhanced storage fault injector.
///
/// This injector distinguishes between different types of storage faults,
/// which is critical for Protocol-Aware Recovery (PAR).
#[derive(Debug)]
pub struct StorageFaultInjector {
    /// Fault states per block.
    block_faults: std::collections::HashMap<u64, BlockFaultState>,
    /// Probability of "not seen" fault (lost write).
    pub not_seen_probability: f64,
    /// Probability of "seen but corrupt" fault.
    pub corrupt_probability: f64,
    /// Probability of phantom write fault.
    pub phantom_probability: f64,
    /// Probability of stale read fault.
    pub stale_read_probability: f64,
}

impl StorageFaultInjector {
    /// Creates a new storage fault injector.
    pub fn new(
        not_seen_probability: f64,
        corrupt_probability: f64,
        phantom_probability: f64,
        stale_read_probability: f64,
    ) -> Self {
        debug_assert!(
            (0.0..=1.0).contains(&not_seen_probability),
            "not_seen_probability must be 0.0 to 1.0"
        );
        debug_assert!(
            (0.0..=1.0).contains(&corrupt_probability),
            "corrupt_probability must be 0.0 to 1.0"
        );
        debug_assert!(
            (0.0..=1.0).contains(&phantom_probability),
            "phantom_probability must be 0.0 to 1.0"
        );
        debug_assert!(
            (0.0..=1.0).contains(&stale_read_probability),
            "stale_read_probability must be 0.0 to 1.0"
        );

        Self {
            block_faults: std::collections::HashMap::new(),
            not_seen_probability,
            corrupt_probability,
            phantom_probability,
            stale_read_probability,
        }
    }

    /// Creates a conservative fault injector (low fault rates).
    pub fn conservative() -> Self {
        Self::new(0.001, 0.001, 0.0001, 0.001)
    }

    /// Creates an aggressive fault injector for stress testing.
    pub fn aggressive() -> Self {
        Self::new(0.01, 0.01, 0.001, 0.01)
    }

    /// Decides if a write should be affected by a fault.
    ///
    /// Returns the fault type if one should be injected, or None for success.
    pub fn check_write(&mut self, block_id: u64, time_ns: u64, rng: &mut SimRng) -> Option<StorageFaultType> {
        // Check each fault type in order
        if rng.next_bool_with_probability(self.not_seen_probability) {
            self.inject_fault(block_id, StorageFaultType::NotSeen, time_ns);
            return Some(StorageFaultType::NotSeen);
        }

        if rng.next_bool_with_probability(self.corrupt_probability) {
            self.inject_fault(block_id, StorageFaultType::SeenButCorrupt, time_ns);
            return Some(StorageFaultType::SeenButCorrupt);
        }

        if rng.next_bool_with_probability(self.phantom_probability) {
            self.inject_fault(block_id, StorageFaultType::PhantomWrite, time_ns);
            return Some(StorageFaultType::PhantomWrite);
        }

        // Clear any previous fault
        self.block_faults.remove(&block_id);
        None
    }

    /// Decides if a read should be affected by a fault.
    pub fn check_read(&self, block_id: u64, rng: &mut SimRng) -> Option<StorageFaultType> {
        // Check if block has an existing fault
        if let Some(state) = self.block_faults.get(&block_id) {
            return state.fault_type;
        }

        // Random stale read
        if rng.next_bool_with_probability(self.stale_read_probability) {
            return Some(StorageFaultType::StaleRead);
        }

        None
    }

    /// Injects a fault for a specific block.
    pub fn inject_fault(&mut self, block_id: u64, fault_type: StorageFaultType, time_ns: u64) {
        self.block_faults.insert(
            block_id,
            BlockFaultState {
                fault_type: Some(fault_type),
                original_checksum: None,
                injected_at_ns: time_ns,
            },
        );
    }

    /// Clears faults for a specific block.
    pub fn clear_fault(&mut self, block_id: u64) {
        self.block_faults.remove(&block_id);
    }

    /// Gets the fault state for a block.
    pub fn get_fault(&self, block_id: u64) -> Option<&BlockFaultState> {
        self.block_faults.get(&block_id)
    }

    /// Returns the count of blocks with each fault type.
    pub fn fault_counts(&self) -> FaultCounts {
        let mut counts = FaultCounts::default();
        for state in self.block_faults.values() {
            if let Some(fault_type) = state.fault_type {
                match fault_type {
                    StorageFaultType::NotSeen => counts.not_seen += 1,
                    StorageFaultType::SeenButCorrupt => counts.corrupt += 1,
                    StorageFaultType::PhantomWrite => counts.phantom += 1,
                    StorageFaultType::StaleRead => counts.stale += 1,
                }
            }
        }
        counts
    }

    /// Resets all fault states.
    pub fn reset(&mut self) {
        self.block_faults.clear();
    }
}

impl Default for StorageFaultInjector {
    fn default() -> Self {
        Self::conservative()
    }
}

/// Counts of each fault type.
#[derive(Debug, Clone, Default)]
pub struct FaultCounts {
    /// Number of "not seen" faults.
    pub not_seen: usize,
    /// Number of "seen but corrupt" faults.
    pub corrupt: usize,
    /// Number of phantom write faults.
    pub phantom: usize,
    /// Number of stale read faults.
    pub stale: usize,
}

// ============================================================================
// Combined Fault Injector
// ============================================================================

/// Comprehensive fault injection configuration.
///
/// Combines all fault injection capabilities into a single configurable interface.
#[derive(Debug)]
pub struct FaultInjector {
    /// Network swizzle-clogging.
    pub swizzle: SwizzleClogger,
    /// Gray failure injection.
    pub gray_failures: GrayFailureInjector,
    /// Storage fault injection.
    pub storage_faults: StorageFaultInjector,
    /// Whether fault injection is enabled.
    pub enabled: bool,
}

impl FaultInjector {
    /// Creates a new fault injector with the given components.
    pub fn new(
        swizzle: SwizzleClogger,
        gray_failures: GrayFailureInjector,
        storage_faults: StorageFaultInjector,
    ) -> Self {
        Self {
            swizzle,
            gray_failures,
            storage_faults,
            enabled: true,
        }
    }

    /// Creates a disabled fault injector.
    pub fn disabled() -> Self {
        Self {
            swizzle: SwizzleClogger::default(),
            gray_failures: GrayFailureInjector::default(),
            storage_faults: StorageFaultInjector::default(),
            enabled: false,
        }
    }

    /// Creates a mild fault injector suitable for basic testing.
    pub fn mild() -> Self {
        Self {
            swizzle: SwizzleClogger::mild(),
            gray_failures: GrayFailureInjector::new(0.02, 0.3),
            storage_faults: StorageFaultInjector::conservative(),
            enabled: true,
        }
    }

    /// Creates an aggressive fault injector for stress testing.
    pub fn aggressive() -> Self {
        Self {
            swizzle: SwizzleClogger::aggressive(),
            gray_failures: GrayFailureInjector::new(0.1, 0.1),
            storage_faults: StorageFaultInjector::aggressive(),
            enabled: true,
        }
    }

    /// Enables or disables fault injection.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// Resets all fault states.
    pub fn reset(&mut self) {
        self.swizzle.reset();
        self.gray_failures.reset();
        self.storage_faults.reset();
    }
}

impl Default for FaultInjector {
    fn default() -> Self {
        Self::mild()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swizzle_clogger_basic() {
        let clogger = SwizzleClogger::new(0.5, 0.5, 2.0, 0.3);
        assert!(!clogger.is_clogged(1, 2));
        assert_eq!(clogger.clogged_count(), 0);
    }

    #[test]
    fn swizzle_clogger_update() {
        let mut clogger = SwizzleClogger::new(1.0, 0.0, 2.0, 0.0); // Always clog
        let mut rng = SimRng::new(42);

        clogger.update(1, 2, &mut rng);
        assert!(clogger.is_clogged(1, 2));
    }

    #[test]
    fn swizzle_clogger_apply() {
        let mut clogger = SwizzleClogger::new(1.0, 0.0, 2.0, 0.0);
        let mut rng = SimRng::new(42);

        // Clog the link
        clogger.update(1, 2, &mut rng);

        // Apply to delay
        let (delay, drop) = clogger.apply(1, 2, 1000, &mut rng);
        assert_eq!(delay, 2000); // 2x delay
        assert!(!drop); // 0% drop probability
    }

    #[test]
    fn gray_failure_healthy_by_default() {
        let injector = GrayFailureInjector::default();
        assert_eq!(injector.get_mode(1), GrayFailureMode::Healthy);
    }

    #[test]
    fn gray_failure_set_mode() {
        let mut injector = GrayFailureInjector::default();
        injector.set_mode(
            1,
            GrayFailureMode::Slow {
                latency_multiplier: 5,
            },
        );

        assert_eq!(
            injector.get_mode(1),
            GrayFailureMode::Slow {
                latency_multiplier: 5
            }
        );
    }

    #[test]
    fn gray_failure_check_operation() {
        let mut injector = GrayFailureInjector::default();
        let mut rng = SimRng::new(42);

        // Healthy node
        let (proceed, mult) = injector.check_operation(1, false, &mut rng);
        assert!(proceed);
        assert_eq!(mult, 1);

        // Slow node
        injector.set_mode(
            1,
            GrayFailureMode::Slow {
                latency_multiplier: 10,
            },
        );
        let (proceed, mult) = injector.check_operation(1, false, &mut rng);
        assert!(proceed);
        assert_eq!(mult, 10);

        // Unresponsive node
        injector.set_mode(1, GrayFailureMode::Unresponsive);
        let (proceed, _) = injector.check_operation(1, false, &mut rng);
        assert!(!proceed);
    }

    #[test]
    fn gray_failure_partial_function() {
        let mut injector = GrayFailureInjector::default();
        let mut rng = SimRng::new(42);

        injector.set_mode(
            1,
            GrayFailureMode::PartialFunction {
                can_read: true,
                can_write: false,
            },
        );

        let (can_read, _) = injector.check_operation(1, false, &mut rng);
        let (can_write, _) = injector.check_operation(1, true, &mut rng);

        assert!(can_read);
        assert!(!can_write);
    }

    #[test]
    fn storage_fault_conservative() {
        let injector = StorageFaultInjector::conservative();
        assert!(injector.not_seen_probability < 0.01);
        assert!(injector.corrupt_probability < 0.01);
    }

    #[test]
    fn storage_fault_injection() {
        let mut injector = StorageFaultInjector::new(1.0, 0.0, 0.0, 0.0); // Always not_seen
        let mut rng = SimRng::new(42);

        let fault = injector.check_write(1, 1000, &mut rng);
        assert_eq!(fault, Some(StorageFaultType::NotSeen));

        let state = injector.get_fault(1).unwrap();
        assert_eq!(state.fault_type, Some(StorageFaultType::NotSeen));
        assert_eq!(state.injected_at_ns, 1000);
    }

    #[test]
    fn storage_fault_counts() {
        let mut injector = StorageFaultInjector::default();
        injector.inject_fault(1, StorageFaultType::NotSeen, 1000);
        injector.inject_fault(2, StorageFaultType::SeenButCorrupt, 2000);
        injector.inject_fault(3, StorageFaultType::NotSeen, 3000);

        let counts = injector.fault_counts();
        assert_eq!(counts.not_seen, 2);
        assert_eq!(counts.corrupt, 1);
        assert_eq!(counts.phantom, 0);
    }

    #[test]
    fn fault_injector_disabled() {
        let injector = FaultInjector::disabled();
        assert!(!injector.enabled);
    }

    #[test]
    fn fault_injector_reset() {
        let mut injector = FaultInjector::aggressive();
        let mut rng = SimRng::new(42);

        // Create some faults
        injector.swizzle.update(1, 2, &mut rng);
        injector
            .gray_failures
            .set_mode(1, GrayFailureMode::Unresponsive);
        injector
            .storage_faults
            .inject_fault(1, StorageFaultType::NotSeen, 1000);

        // Reset
        injector.reset();

        assert_eq!(injector.swizzle.clogged_count(), 0);
        assert_eq!(injector.gray_failures.failing_count(), 0);
        assert_eq!(injector.storage_faults.fault_counts().not_seen, 0);
    }
}
