//! # vdb-sim: Deterministic Simulation Testing for `VerityDB`
//!
//! This crate provides a VOPR-style (Viewstamped Operation Replication)
//! simulation harness for testing `VerityDB` under controlled conditions.
//!
//! ## Philosophy
//!
//! Inspired by `FoundationDB`'s "trillion CPU-hour" simulation testing and
//! `TigerBeetle`'s approach, this harness enables:
//!
//! - **Reproducibility**: Same seed → same execution → same bugs
//! - **Time compression**: Run years of simulated time in seconds
//! - **Fault injection**: Network partitions, storage failures, crashes
//! - **Invariant checking**: Verify correctness properties continuously
//!
//! ## Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                    Simulation Harness                            │
//! │  ┌─────────────┐   ┌──────────────┐   ┌─────────────────────┐   │
//! │  │ SimClock    │   │ EventQueue   │   │ SimRng              │   │
//! │  │ (discrete)  │   │ (scheduler)  │   │ (deterministic)     │   │
//! │  └─────────────┘   └──────────────┘   └─────────────────────┘   │
//! │                                                                   │
//! │  ┌─────────────────────────────────────────────────────────────┐ │
//! │  │                    Simulated Components                      │ │
//! │  │  SimNetwork    SimStorage    SimNode    FaultInjector       │ │
//! │  └─────────────────────────────────────────────────────────────┘ │
//! │                                                                   │
//! │  ┌─────────────────────────────────────────────────────────────┐ │
//! │  │                    Invariant Checkers                        │ │
//! │  │  HashChainChecker  LinearizabilityChecker  ConsistencyChecker│ │
//! │  └─────────────────────────────────────────────────────────────┘ │
//! └─────────────────────────────────────────────────────────────────┘
//! ```
//!
//! ## Quick Start
//!
//! ```ignore
//! use vdb_sim::{Simulation, SimConfig};
//!
//! // Create a simulation with a specific seed
//! let config = SimConfig::default().with_seed(12345);
//! let mut sim = Simulation::new(config);
//!
//! // Run until completion or failure
//! let result = sim.run();
//! assert!(result.is_ok());
//! ```
//!
//! ## Key Concepts
//!
//! - **`SimClock`**: Discrete simulated time (nanosecond precision)
//! - **`EventQueue`**: Priority queue of scheduled events
//! - **`SimRng`**: Deterministic random number generator (seeded)
//! - **`FaultInjector`**: Configurable failure injection
//! - **`InvariantChecker`**: Continuous correctness verification

mod clock;
mod error;
mod event;
mod fault;
mod invariant;
mod network;
mod rng;
mod storage;

pub use clock::{SimClock, ms_to_ns, ns_to_ms, ns_to_sec, sec_to_ns};
pub use error::SimError;
pub use event::{Event, EventId, EventKind, EventQueue};
pub use fault::{
    BlockFaultState, FaultCounts, FaultInjector, GrayFailureInjector, GrayFailureMode,
    StorageFaultInjector, StorageFaultType, SwizzleClogger,
};
pub use invariant::{
    ConsistencyViolation, HashChainChecker, InvariantChecker, InvariantResult,
    LinearizabilityChecker, LogConsistencyChecker, OpType, Operation, ReplicaConsistencyChecker,
    ReplicaState,
};
pub use network::{
    Message, MessageId, NetworkConfig, NetworkStats, Partition, RejectReason, SendResult,
    SimNetwork,
};
pub use rng::SimRng;
pub use storage::{
    FsyncResult, ReadResult, SimStorage, StorageConfig, StorageStats, WriteFailure, WriteResult,
};

// ============================================================================
// Simulation Configuration
// ============================================================================

/// Configuration for a simulation run.
#[derive(Debug, Clone)]
pub struct SimConfig {
    /// Seed for the deterministic RNG.
    pub seed: u64,
    /// Maximum simulation time (nanoseconds).
    pub max_time_ns: u64,
    /// Maximum number of events to process.
    pub max_events: u64,
    /// Whether to enable detailed tracing.
    pub trace_enabled: bool,
}

impl Default for SimConfig {
    fn default() -> Self {
        Self {
            seed: 0,
            max_time_ns: 60 * 1_000_000_000, // 60 seconds of simulated time
            max_events: 1_000_000,
            trace_enabled: false,
        }
    }
}

impl SimConfig {
    /// Creates a new configuration with the specified seed.
    pub fn with_seed(mut self, seed: u64) -> Self {
        self.seed = seed;
        self
    }

    /// Sets the maximum simulation time.
    pub fn with_max_time_ns(mut self, max_time_ns: u64) -> Self {
        self.max_time_ns = max_time_ns;
        self
    }

    /// Sets the maximum number of events.
    pub fn with_max_events(mut self, max_events: u64) -> Self {
        self.max_events = max_events;
        self
    }

    /// Enables detailed tracing.
    pub fn with_tracing(mut self) -> Self {
        self.trace_enabled = true;
        self
    }
}

// ============================================================================
// Simulation Context
// ============================================================================

/// The main simulation context that coordinates all simulated components.
///
/// This is the entry point for running deterministic simulations.
pub struct Simulation {
    /// Configuration for this simulation run.
    config: SimConfig,
    /// Simulated clock (discrete time).
    clock: SimClock,
    /// Event scheduler.
    events: EventQueue,
    /// Deterministic random number generator.
    rng: SimRng,
    /// Number of events processed.
    events_processed: u64,
}

impl Simulation {
    /// Creates a new simulation with the given configuration.
    pub fn new(config: SimConfig) -> Self {
        let clock = SimClock::new();
        let events = EventQueue::new();
        let rng = SimRng::new(config.seed);

        Self {
            config,
            clock,
            events,
            rng,
            events_processed: 0,
        }
    }

    /// Returns a reference to the simulated clock.
    pub fn clock(&self) -> &SimClock {
        &self.clock
    }

    /// Returns a mutable reference to the simulated clock.
    pub fn clock_mut(&mut self) -> &mut SimClock {
        &mut self.clock
    }

    /// Returns a reference to the event queue.
    pub fn events(&self) -> &EventQueue {
        &self.events
    }

    /// Returns a mutable reference to the event queue.
    pub fn events_mut(&mut self) -> &mut EventQueue {
        &mut self.events
    }

    /// Returns a reference to the RNG.
    pub fn rng(&self) -> &SimRng {
        &self.rng
    }

    /// Returns a mutable reference to the RNG.
    pub fn rng_mut(&mut self) -> &mut SimRng {
        &mut self.rng
    }

    /// Returns the current simulation time in nanoseconds.
    pub fn now(&self) -> u64 {
        self.clock.now()
    }

    /// Returns the configuration.
    pub fn config(&self) -> &SimConfig {
        &self.config
    }

    /// Schedules an event at the given time.
    pub fn schedule(&mut self, time_ns: u64, kind: EventKind) -> EventId {
        self.events.schedule(time_ns, kind)
    }

    /// Schedules an event after a delay from the current time.
    pub fn schedule_after(&mut self, delay_ns: u64, kind: EventKind) -> EventId {
        let time_ns = self.clock.now() + delay_ns;
        self.events.schedule(time_ns, kind)
    }

    /// Processes the next event in the queue.
    ///
    /// Returns `None` if the queue is empty or the simulation has exceeded limits.
    pub fn step(&mut self) -> Option<Event> {
        // Check limits
        if self.events_processed >= self.config.max_events {
            return None;
        }

        // Get next event
        let event = self.events.pop()?;

        // Check time limit
        if event.time_ns > self.config.max_time_ns {
            // Put the event back and stop
            self.events.schedule(event.time_ns, event.kind);
            return None;
        }

        // Advance clock to event time
        self.clock.advance_to(event.time_ns);
        self.events_processed += 1;

        Some(event)
    }

    /// Runs the simulation until completion or an error occurs.
    ///
    /// Returns a summary of the simulation run.
    pub fn run(&mut self) -> Result<SimSummary, SimError> {
        while self.step().is_some() {
            // Events are processed in step()
            // Subclasses/extensions would override to handle events
        }

        Ok(SimSummary {
            events_processed: self.events_processed,
            final_time_ns: self.clock.now(),
            seed: self.config.seed,
        })
    }

    /// Returns the number of events processed so far.
    pub fn events_processed(&self) -> u64 {
        self.events_processed
    }
}

// ============================================================================
// Simulation Summary
// ============================================================================

/// Summary of a completed simulation run.
#[derive(Debug, Clone)]
pub struct SimSummary {
    /// Total number of events processed.
    pub events_processed: u64,
    /// Final simulation time (nanoseconds).
    pub final_time_ns: u64,
    /// Seed used for this run.
    pub seed: u64,
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simulation_basic_lifecycle() {
        let config = SimConfig::default().with_seed(42);
        let mut sim = Simulation::new(config);

        // Schedule some events
        sim.schedule(1_000_000, EventKind::Custom(1)); // 1ms
        sim.schedule(2_000_000, EventKind::Custom(2)); // 2ms
        sim.schedule(500_000, EventKind::Custom(0)); // 0.5ms

        // Events should come out in order
        let e1 = sim.step().expect("should have event");
        assert_eq!(e1.time_ns, 500_000);

        let e2 = sim.step().expect("should have event");
        assert_eq!(e2.time_ns, 1_000_000);

        let e3 = sim.step().expect("should have event");
        assert_eq!(e3.time_ns, 2_000_000);

        // No more events
        assert!(sim.step().is_none());
    }

    #[test]
    fn simulation_respects_time_limit() {
        let config = SimConfig::default()
            .with_seed(42)
            .with_max_time_ns(1_000_000); // 1ms limit

        let mut sim = Simulation::new(config);

        sim.schedule(500_000, EventKind::Custom(1)); // 0.5ms - OK
        sim.schedule(2_000_000, EventKind::Custom(2)); // 2ms - over limit

        let e1 = sim.step().expect("first event should process");
        assert_eq!(e1.time_ns, 500_000);

        // Second event exceeds limit
        assert!(sim.step().is_none());
    }

    #[test]
    fn simulation_respects_event_limit() {
        let config = SimConfig::default().with_seed(42).with_max_events(2);

        let mut sim = Simulation::new(config);

        sim.schedule(1_000, EventKind::Custom(1));
        sim.schedule(2_000, EventKind::Custom(2));
        sim.schedule(3_000, EventKind::Custom(3));

        assert!(sim.step().is_some());
        assert!(sim.step().is_some());
        assert!(sim.step().is_none()); // Limit reached
    }

    #[test]
    fn simulation_run_to_completion() {
        let config = SimConfig::default().with_seed(123);
        let mut sim = Simulation::new(config);

        sim.schedule(1_000_000, EventKind::Custom(1));
        sim.schedule(2_000_000, EventKind::Custom(2));

        let summary = sim.run().expect("should complete");

        assert_eq!(summary.events_processed, 2);
        assert_eq!(summary.final_time_ns, 2_000_000);
        assert_eq!(summary.seed, 123);
    }

    #[test]
    fn schedule_after_uses_current_time() {
        let config = SimConfig::default().with_seed(0);
        let mut sim = Simulation::new(config);

        // Advance time by processing an event
        sim.schedule(1_000_000, EventKind::Custom(1));
        sim.step();

        // Now schedule relative to current time
        sim.schedule_after(500_000, EventKind::Custom(2));

        let event = sim.step().expect("should have event");
        assert_eq!(event.time_ns, 1_500_000); // 1ms + 0.5ms
    }
}
