//! Error types for simulation testing.

/// Errors that can occur during simulation.
#[derive(thiserror::Error, Debug)]
pub enum SimError {
    /// An invariant was violated during simulation.
    #[error("invariant violated: {message}")]
    InvariantViolation {
        /// Description of the violated invariant.
        message: String,
        /// The simulation time when the violation occurred.
        time_ns: u64,
    },

    /// The simulation exceeded its configured limits.
    #[error("simulation limit exceeded: {kind}")]
    LimitExceeded {
        /// What limit was exceeded.
        kind: LimitKind,
    },

    /// A simulated component failed in an unexpected way.
    #[error("simulated {component} failed: {reason}")]
    ComponentFailure {
        /// Which component failed.
        component: String,
        /// Why it failed.
        reason: String,
    },

    /// Hash chain verification failed.
    #[error("hash chain broken at offset {offset}: expected {expected}, got {actual}")]
    HashChainBroken {
        /// Offset where the chain broke.
        offset: u64,
        /// Expected hash (hex).
        expected: String,
        /// Actual hash (hex).
        actual: String,
    },

    /// Linearizability check failed.
    #[error("linearizability violation: {description}")]
    LinearizabilityViolation {
        /// Description of the violation.
        description: String,
    },

    /// Replica consistency check failed.
    #[error("replica inconsistency between {replica_a} and {replica_b}: {description}")]
    ReplicaInconsistency {
        /// First replica ID.
        replica_a: String,
        /// Second replica ID.
        replica_b: String,
        /// Description of the inconsistency.
        description: String,
    },
}

/// Kind of limit that was exceeded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LimitKind {
    /// Maximum simulation time exceeded.
    Time,
    /// Maximum number of events exceeded.
    Events,
    /// Maximum memory usage exceeded.
    Memory,
}

impl std::fmt::Display for LimitKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LimitKind::Time => write!(f, "max_time"),
            LimitKind::Events => write!(f, "max_events"),
            LimitKind::Memory => write!(f, "max_memory"),
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_display() {
        let err = SimError::InvariantViolation {
            message: "log not monotonic".to_string(),
            time_ns: 1_000_000,
        };
        assert!(err.to_string().contains("log not monotonic"));

        let err = SimError::LimitExceeded {
            kind: LimitKind::Time,
        };
        assert!(err.to_string().contains("max_time"));
    }
}
