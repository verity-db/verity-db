//! Invariant checkers for simulation testing.
//!
//! Invariant checkers continuously verify correctness properties during
//! simulation. If an invariant is violated, the simulation can stop
//! immediately with a detailed error.
//!
//! # Available Checkers
//!
//! - [`HashChainChecker`]: Verifies hash chain integrity
//! - [`LogConsistencyChecker`]: Verifies log reads match committed writes
//! - [`LinearizabilityChecker`]: Verifies linearizable operation history
//! - [`ReplicaConsistencyChecker`]: Verifies byte-for-byte replica consistency

use vdb_crypto::ChainHash;

use crate::SimError;

// ============================================================================
// Invariant Result
// ============================================================================

/// Result of an invariant check.
#[derive(Debug, Clone)]
pub enum InvariantResult {
    /// The invariant holds.
    Ok,
    /// The invariant is violated.
    Violated {
        /// Name of the violated invariant.
        invariant: String,
        /// Description of the violation.
        message: String,
        /// Additional context.
        context: Vec<(String, String)>,
    },
}

impl InvariantResult {
    /// Returns true if the invariant holds.
    pub fn is_ok(&self) -> bool {
        matches!(self, InvariantResult::Ok)
    }

    /// Converts to a `SimError` if violated.
    pub fn into_error(self, time_ns: u64) -> Option<SimError> {
        match self {
            InvariantResult::Ok => None,
            InvariantResult::Violated { message, .. } => {
                Some(SimError::InvariantViolation { message, time_ns })
            }
        }
    }
}

// ============================================================================
// Invariant Checker Trait
// ============================================================================

/// Trait for invariant checkers.
///
/// Invariant checkers verify that correctness properties hold during simulation.
pub trait InvariantChecker {
    /// Returns the name of this checker.
    fn name(&self) -> &'static str;

    /// Resets the checker to its initial state.
    fn reset(&mut self);
}

// ============================================================================
// Hash Chain Checker
// ============================================================================

/// Verifies hash chain integrity.
///
/// The hash chain checker maintains the expected chain state and verifies
/// that each new record correctly links to the previous one.
#[derive(Debug)]
pub struct HashChainChecker {
    /// The last seen chain hash.
    last_hash: Option<ChainHash>,
    /// The last seen offset.
    last_offset: Option<u64>,
    /// Number of records checked.
    records_checked: u64,
}

impl HashChainChecker {
    /// Creates a new hash chain checker.
    pub fn new() -> Self {
        Self {
            last_hash: None,
            last_offset: None,
            records_checked: 0,
        }
    }

    /// Checks a record against the expected chain state.
    ///
    /// # Arguments
    ///
    /// * `offset` - The record's offset in the log
    /// * `prev_hash` - The record's claimed previous hash
    /// * `current_hash` - The hash of this record
    pub fn check_record(
        &mut self,
        offset: u64,
        prev_hash: &ChainHash,
        current_hash: &ChainHash,
    ) -> InvariantResult {
        // Check offset monotonicity
        if let Some(last_offset) = self.last_offset {
            if offset != last_offset + 1 {
                return InvariantResult::Violated {
                    invariant: "hash_chain_offset_monotonic".to_string(),
                    message: format!("offset gap: expected {}, got {}", last_offset + 1, offset),
                    context: vec![
                        ("last_offset".to_string(), last_offset.to_string()),
                        ("current_offset".to_string(), offset.to_string()),
                    ],
                };
            }
        } else if offset != 0 {
            // First record should be at offset 0
            return InvariantResult::Violated {
                invariant: "hash_chain_starts_at_zero".to_string(),
                message: format!("first record should be at offset 0, got {offset}"),
                context: vec![("offset".to_string(), offset.to_string())],
            };
        }

        // Check hash chain linkage
        if let Some(expected_prev) = &self.last_hash {
            if prev_hash != expected_prev {
                return InvariantResult::Violated {
                    invariant: "hash_chain_linkage".to_string(),
                    message: "hash chain broken: prev_hash doesn't match".to_string(),
                    context: vec![
                        ("offset".to_string(), offset.to_string()),
                        ("expected_prev".to_string(), format!("{expected_prev:?}")),
                        ("actual_prev".to_string(), format!("{prev_hash:?}")),
                    ],
                };
            }
        } else {
            // First record should have zero prev_hash
            let zero_hash = ChainHash::from_bytes(&[0u8; 32]);
            if *prev_hash != zero_hash {
                return InvariantResult::Violated {
                    invariant: "hash_chain_genesis".to_string(),
                    message: "first record should have zero prev_hash".to_string(),
                    context: vec![
                        ("offset".to_string(), offset.to_string()),
                        ("prev_hash".to_string(), format!("{prev_hash:?}")),
                    ],
                };
            }
        }

        // Update state
        self.last_hash = Some(*current_hash);
        self.last_offset = Some(offset);
        self.records_checked += 1;

        InvariantResult::Ok
    }

    /// Returns the number of records checked.
    pub fn records_checked(&self) -> u64 {
        self.records_checked
    }

    /// Returns the last verified offset.
    pub fn last_offset(&self) -> Option<u64> {
        self.last_offset
    }

    /// Returns the last verified hash.
    pub fn last_hash(&self) -> Option<&ChainHash> {
        self.last_hash.as_ref()
    }
}

impl Default for HashChainChecker {
    fn default() -> Self {
        Self::new()
    }
}

impl InvariantChecker for HashChainChecker {
    fn name(&self) -> &'static str {
        "HashChainChecker"
    }

    fn reset(&mut self) {
        self.last_hash = None;
        self.last_offset = None;
        self.records_checked = 0;
    }
}

// ============================================================================
// Log Consistency Checker
// ============================================================================

/// Verifies log consistency across multiple views.
///
/// This checker ensures that once a record is committed, it remains
/// consistent across all subsequent reads.
#[derive(Debug)]
pub struct LogConsistencyChecker {
    /// Known committed records: offset -> (hash, `payload_hash`)
    committed: std::collections::HashMap<u64, (ChainHash, [u8; 32])>,
}

impl LogConsistencyChecker {
    /// Creates a new log consistency checker.
    pub fn new() -> Self {
        Self {
            committed: std::collections::HashMap::new(),
        }
    }

    /// Records a committed entry.
    pub fn record_commit(&mut self, offset: u64, chain_hash: ChainHash, payload_hash: [u8; 32]) {
        self.committed.insert(offset, (chain_hash, payload_hash));
    }

    /// Verifies a read against known commits.
    pub fn verify_read(
        &self,
        offset: u64,
        chain_hash: &ChainHash,
        payload_hash: &[u8; 32],
    ) -> InvariantResult {
        if let Some((expected_chain, expected_payload)) = self.committed.get(&offset) {
            if chain_hash != expected_chain {
                return InvariantResult::Violated {
                    invariant: "log_consistency_chain_hash".to_string(),
                    message: "chain hash mismatch on read".to_string(),
                    context: vec![
                        ("offset".to_string(), offset.to_string()),
                        ("expected".to_string(), format!("{expected_chain:?}")),
                        ("actual".to_string(), format!("{chain_hash:?}")),
                    ],
                };
            }
            if payload_hash != expected_payload {
                return InvariantResult::Violated {
                    invariant: "log_consistency_payload".to_string(),
                    message: "payload hash mismatch on read".to_string(),
                    context: vec![
                        ("offset".to_string(), offset.to_string()),
                        ("expected".to_string(), hex::encode(expected_payload)),
                        ("actual".to_string(), hex::encode(payload_hash)),
                    ],
                };
            }
        }
        InvariantResult::Ok
    }

    /// Returns the number of committed entries tracked.
    pub fn committed_count(&self) -> usize {
        self.committed.len()
    }
}

impl Default for LogConsistencyChecker {
    fn default() -> Self {
        Self::new()
    }
}

impl InvariantChecker for LogConsistencyChecker {
    fn name(&self) -> &'static str {
        "LogConsistencyChecker"
    }

    fn reset(&mut self) {
        self.committed.clear();
    }
}

// ============================================================================
// Linearizability Checker
// ============================================================================

/// Operation type for linearizability checking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpType {
    /// Read operation: key -> observed value
    Read { key: u64, value: Option<u64> },
    /// Write operation: key -> new value
    Write { key: u64, value: u64 },
}

/// A recorded operation for linearizability checking.
#[derive(Debug, Clone)]
pub struct Operation {
    /// Unique operation ID.
    pub id: u64,
    /// Client that issued this operation.
    pub client_id: u64,
    /// Time when the operation was invoked (started).
    pub invoke_time: u64,
    /// Time when the operation completed (None if pending).
    pub response_time: Option<u64>,
    /// The operation type and arguments/result.
    pub op_type: OpType,
}

/// Verifies linearizability of operation history.
///
/// Linearizability requires that:
/// 1. Each operation appears to take effect atomically at some point
///    between its invocation and response.
/// 2. The resulting sequential history is legal (reads see latest writes).
///
/// This checker implements a simplified Wing-Gong style algorithm for
/// single-key operations. For multi-key transactions, a more sophisticated
/// approach would be needed.
#[derive(Debug)]
pub struct LinearizabilityChecker {
    /// Recorded operations.
    operations: Vec<Operation>,
    /// Next operation ID.
    next_op_id: u64,
}

impl LinearizabilityChecker {
    /// Creates a new linearizability checker.
    pub fn new() -> Self {
        Self {
            operations: Vec::new(),
            next_op_id: 0,
        }
    }

    /// Records an operation invocation (start).
    ///
    /// Returns the operation ID for later completion.
    pub fn invoke(&mut self, client_id: u64, invoke_time: u64, op_type: OpType) -> u64 {
        let id = self.next_op_id;
        self.next_op_id += 1;

        self.operations.push(Operation {
            id,
            client_id,
            invoke_time,
            response_time: None,
            op_type,
        });

        id
    }

    /// Records an operation response (completion).
    pub fn respond(&mut self, op_id: u64, response_time: u64) {
        if let Some(op) = self.operations.iter_mut().find(|o| o.id == op_id) {
            debug_assert!(
                op.response_time.is_none(),
                "operation {op_id} already completed"
            );
            debug_assert!(
                response_time >= op.invoke_time,
                "response time before invoke time"
            );
            op.response_time = Some(response_time);
        }
    }

    /// Checks if the recorded history is linearizable.
    ///
    /// This uses a brute-force approach suitable for small histories.
    /// For larger histories, more efficient algorithms exist (e.g., P-compositionality).
    pub fn check(&self) -> InvariantResult {
        // Filter to completed operations only
        let completed: Vec<_> = self
            .operations
            .iter()
            .filter(|op| op.response_time.is_some())
            .collect();

        if completed.is_empty() {
            return InvariantResult::Ok;
        }

        // Group operations by key
        let mut by_key: std::collections::HashMap<u64, Vec<&Operation>> =
            std::collections::HashMap::new();

        for op in &completed {
            let key = match &op.op_type {
                OpType::Read { key, .. } | OpType::Write { key, .. } => *key,
            };
            by_key.entry(key).or_default().push(op);
        }

        // Check linearizability for each key independently
        // (This works because our operations are single-key)
        for (key, ops) in by_key {
            if let Some(violation) = Self::check_single_key(key, &ops) {
                return violation;
            }
        }

        InvariantResult::Ok
    }

    /// Checks linearizability for operations on a single key.
    fn check_single_key(key: u64, ops: &[&Operation]) -> Option<InvariantResult> {
        // Try all possible linearization orders
        if Self::try_linearize(ops, 0, &mut vec![false; ops.len()], &mut Vec::new()) {
            None
        } else {
            Some(InvariantResult::Violated {
                invariant: "linearizability".to_string(),
                message: format!("no valid linearization found for key {key}"),
                context: vec![
                    ("key".to_string(), key.to_string()),
                    ("operation_count".to_string(), ops.len().to_string()),
                ],
            })
        }
    }

    /// Recursively tries to find a valid linearization.
    fn try_linearize(
        ops: &[&Operation],
        current_value: u64,
        used: &mut Vec<bool>,
        order: &mut Vec<usize>,
    ) -> bool {
        // Base case: all operations linearized
        if order.len() == ops.len() {
            return true;
        }

        // Try each unused operation that could be linearized next
        for i in 0..ops.len() {
            if used[i] {
                continue;
            }

            let op = ops[i];

            // Check if this operation can be linearized here
            // (its linearization point must be within its invoke-response interval
            // and after all previously linearized operations)
            if !Self::can_linearize_next(ops, order, i) {
                continue;
            }

            // Check if the operation is consistent with current state
            let (valid, new_value) = match &op.op_type {
                OpType::Read { value, .. } => {
                    // Read must see the current value
                    let expected = if current_value == 0 {
                        None
                    } else {
                        Some(current_value)
                    };
                    (*value == expected, current_value)
                }
                OpType::Write { value, .. } => {
                    // Write always succeeds
                    (true, *value)
                }
            };

            if valid {
                used[i] = true;
                order.push(i);

                if Self::try_linearize(ops, new_value, used, order) {
                    return true;
                }

                order.pop();
                used[i] = false;
            }
        }

        false
    }

    /// Checks if operation at index `next` can be linearized after the current order.
    fn can_linearize_next(ops: &[&Operation], order: &[usize], next: usize) -> bool {
        let next_op = ops[next];
        let next_invoke = next_op.invoke_time;
        let next_response = next_op.response_time.unwrap();

        // The linearization point must be after all previous operations' linearization points.
        // Since we don't track exact linearization points, we use the constraint that
        // the next operation's response must not be before any previous operation's invoke.
        for &prev_idx in order {
            let prev_op = ops[prev_idx];
            // If prev_op's response is before next_op's invoke, next must come after
            // (This is the "happens-before" relationship)
            if prev_op.response_time.unwrap() < next_invoke {
                // prev definitely happens before next, which is fine
                continue;
            }
            // If next_op's response is before prev_op's invoke, that would be a problem
            // because we're trying to put next after prev
            if next_response < prev_op.invoke_time {
                return false;
            }
        }

        true
    }

    /// Returns the number of recorded operations.
    pub fn operation_count(&self) -> usize {
        self.operations.len()
    }

    /// Returns the number of completed operations.
    pub fn completed_count(&self) -> usize {
        self.operations
            .iter()
            .filter(|op| op.response_time.is_some())
            .count()
    }
}

impl Default for LinearizabilityChecker {
    fn default() -> Self {
        Self::new()
    }
}

impl InvariantChecker for LinearizabilityChecker {
    fn name(&self) -> &'static str {
        "LinearizabilityChecker"
    }

    fn reset(&mut self) {
        self.operations.clear();
        self.next_op_id = 0;
    }
}

// ============================================================================
// Replica Consistency Checker
// ============================================================================

/// State of a single replica's log.
#[derive(Debug, Clone)]
pub struct ReplicaState {
    /// Replica identifier.
    pub replica_id: u64,
    /// Current log length (number of entries).
    pub log_length: u64,
    /// Hash of the log contents (for byte-for-byte comparison).
    pub log_hash: [u8; 32],
    /// Last update time.
    pub last_update_ns: u64,
}

/// Verifies byte-for-byte consistency across replicas.
///
/// Inspired by `TigerBeetle`'s replica consistency checking, this verifier ensures
/// that all replicas that have caught up to the same log position have identical
/// content. This detects:
///
/// - Byzantine failures (replicas diverging)
/// - Bugs in replication logic
/// - Storage corruption that went undetected
///
/// # How It Works
///
/// 1. Each replica reports its state (log length + content hash)
/// 2. When multiple replicas report the same log length, their hashes must match
/// 3. A violation indicates a critical consistency bug
#[derive(Debug)]
pub struct ReplicaConsistencyChecker {
    /// Known replica states: `replica_id` -> state
    replicas: std::collections::HashMap<u64, ReplicaState>,
    /// Consistency violations detected.
    violations: Vec<ConsistencyViolation>,
}

/// A detected consistency violation.
#[derive(Debug, Clone)]
pub struct ConsistencyViolation {
    /// Log length where divergence was detected.
    pub log_length: u64,
    /// Replicas that disagree.
    pub divergent_replicas: Vec<(u64, [u8; 32])>, // (replica_id, hash)
    /// Time when violation was detected.
    pub detected_at_ns: u64,
}

impl ReplicaConsistencyChecker {
    /// Creates a new replica consistency checker.
    pub fn new() -> Self {
        Self {
            replicas: std::collections::HashMap::new(),
            violations: Vec::new(),
        }
    }

    /// Updates the state of a replica.
    ///
    /// Returns a violation if this update reveals inconsistency.
    /// The replica state is always tracked, even when divergence is detected.
    pub fn update_replica(
        &mut self,
        replica_id: u64,
        log_length: u64,
        log_hash: [u8; 32],
        time_ns: u64,
    ) -> InvariantResult {
        // Check against other replicas at the same log length
        let mut violation_result = None;
        for (other_id, other_state) in &self.replicas {
            if *other_id == replica_id {
                continue;
            }

            if other_state.log_length == log_length && other_state.log_hash != log_hash {
                let violation = ConsistencyViolation {
                    log_length,
                    divergent_replicas: vec![
                        (*other_id, other_state.log_hash),
                        (replica_id, log_hash),
                    ],
                    detected_at_ns: time_ns,
                };
                self.violations.push(violation);

                violation_result = Some(InvariantResult::Violated {
                    invariant: "replica_consistency".to_string(),
                    message: format!(
                        "replicas {other_id} and {replica_id} diverge at log length {log_length}"
                    ),
                    context: vec![
                        ("log_length".to_string(), log_length.to_string()),
                        ("replica_a".to_string(), other_id.to_string()),
                        ("hash_a".to_string(), hex::encode(&other_state.log_hash)),
                        ("replica_b".to_string(), replica_id.to_string()),
                        ("hash_b".to_string(), hex::encode(&log_hash)),
                    ],
                });
                break;
            }
        }

        // Always update replica state (even on violation, to continue tracking)
        self.replicas.insert(
            replica_id,
            ReplicaState {
                replica_id,
                log_length,
                log_hash,
                last_update_ns: time_ns,
            },
        );

        violation_result.unwrap_or(InvariantResult::Ok)
    }

    /// Performs a full consistency check across all replicas.
    ///
    /// Groups replicas by log length and verifies hash consistency within each group.
    pub fn check_all(&self) -> InvariantResult {
        // Group replicas by log length
        let mut by_length: std::collections::HashMap<u64, Vec<&ReplicaState>> =
            std::collections::HashMap::new();

        for state in self.replicas.values() {
            by_length.entry(state.log_length).or_default().push(state);
        }

        // Check consistency within each group
        for (length, replicas) in by_length {
            if replicas.len() < 2 {
                continue;
            }

            let first_hash = &replicas[0].log_hash;
            for replica in &replicas[1..] {
                if &replica.log_hash != first_hash {
                    return InvariantResult::Violated {
                        invariant: "replica_consistency".to_string(),
                        message: format!(
                            "replicas diverge at log length {length}: {} vs {}",
                            replicas[0].replica_id, replica.replica_id
                        ),
                        context: vec![
                            ("log_length".to_string(), length.to_string()),
                            ("replica_a".to_string(), replicas[0].replica_id.to_string()),
                            ("hash_a".to_string(), hex::encode(first_hash)),
                            ("replica_b".to_string(), replica.replica_id.to_string()),
                            ("hash_b".to_string(), hex::encode(&replica.log_hash)),
                        ],
                    };
                }
            }
        }

        InvariantResult::Ok
    }

    /// Returns the number of replicas being tracked.
    pub fn replica_count(&self) -> usize {
        self.replicas.len()
    }

    /// Returns the number of violations detected.
    pub fn violation_count(&self) -> usize {
        self.violations.len()
    }

    /// Returns all detected violations.
    pub fn violations(&self) -> &[ConsistencyViolation] {
        &self.violations
    }

    /// Returns the state of a specific replica.
    pub fn get_replica(&self, replica_id: u64) -> Option<&ReplicaState> {
        self.replicas.get(&replica_id)
    }
}

impl Default for ReplicaConsistencyChecker {
    fn default() -> Self {
        Self::new()
    }
}

impl InvariantChecker for ReplicaConsistencyChecker {
    fn name(&self) -> &'static str {
        "ReplicaConsistencyChecker"
    }

    fn reset(&mut self) {
        self.replicas.clear();
        self.violations.clear();
    }
}

// ============================================================================
// Hex encoding helper (minimal, no external dep)
// ============================================================================

mod hex {
    pub fn encode(bytes: &[u8]) -> String {
        let mut s = String::with_capacity(bytes.len() * 2);
        for b in bytes {
            use std::fmt::Write;
            write!(s, "{b:02x}").expect("formatting cannot fail");
        }
        s
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use vdb_crypto::chain_hash;

    #[test]
    fn hash_chain_checker_valid_chain() {
        let mut checker = HashChainChecker::new();

        // Genesis record
        let hash0 = chain_hash(None, b"genesis");
        let zero_hash = ChainHash::from_bytes(&[0u8; 32]);
        let result = checker.check_record(0, &zero_hash, &hash0);
        assert!(result.is_ok());

        // Second record
        let hash1 = chain_hash(Some(&hash0), b"second");
        let result = checker.check_record(1, &hash0, &hash1);
        assert!(result.is_ok());

        // Third record
        let hash2 = chain_hash(Some(&hash1), b"third");
        let result = checker.check_record(2, &hash1, &hash2);
        assert!(result.is_ok());

        assert_eq!(checker.records_checked(), 3);
    }

    #[test]
    fn hash_chain_checker_detects_broken_chain() {
        let mut checker = HashChainChecker::new();

        // Genesis record
        let hash0 = chain_hash(None, b"genesis");
        let zero_hash = ChainHash::from_bytes(&[0u8; 32]);
        checker.check_record(0, &zero_hash, &hash0);

        // Second record with WRONG prev_hash
        let wrong_prev = ChainHash::from_bytes(&[1u8; 32]);
        let hash1 = chain_hash(Some(&hash0), b"second");
        let result = checker.check_record(1, &wrong_prev, &hash1);

        assert!(!result.is_ok());
        match result {
            InvariantResult::Violated { invariant, .. } => {
                assert_eq!(invariant, "hash_chain_linkage");
            }
            _ => panic!("expected violation"),
        }
    }

    #[test]
    fn hash_chain_checker_detects_offset_gap() {
        let mut checker = HashChainChecker::new();

        // Genesis record
        let hash0 = chain_hash(None, b"genesis");
        let zero_hash = ChainHash::from_bytes(&[0u8; 32]);
        checker.check_record(0, &zero_hash, &hash0);

        // Skip to offset 5 (should fail)
        let hash5 = chain_hash(Some(&hash0), b"skipped");
        let result = checker.check_record(5, &hash0, &hash5);

        assert!(!result.is_ok());
        match result {
            InvariantResult::Violated { invariant, .. } => {
                assert_eq!(invariant, "hash_chain_offset_monotonic");
            }
            _ => panic!("expected violation"),
        }
    }

    #[test]
    fn hash_chain_checker_first_record_must_be_zero() {
        let mut checker = HashChainChecker::new();

        // Try to start at offset 1
        let hash = chain_hash(None, b"wrong start");
        let zero_hash = ChainHash::from_bytes(&[0u8; 32]);
        let result = checker.check_record(1, &zero_hash, &hash);

        assert!(!result.is_ok());
        match result {
            InvariantResult::Violated { invariant, .. } => {
                assert_eq!(invariant, "hash_chain_starts_at_zero");
            }
            _ => panic!("expected violation"),
        }
    }

    #[test]
    fn hash_chain_checker_genesis_must_have_zero_prev() {
        let mut checker = HashChainChecker::new();

        // Genesis with non-zero prev_hash
        let hash = chain_hash(None, b"genesis");
        let non_zero_prev = ChainHash::from_bytes(&[1u8; 32]);
        let result = checker.check_record(0, &non_zero_prev, &hash);

        assert!(!result.is_ok());
        match result {
            InvariantResult::Violated { invariant, .. } => {
                assert_eq!(invariant, "hash_chain_genesis");
            }
            _ => panic!("expected violation"),
        }
    }

    #[test]
    fn hash_chain_checker_reset() {
        let mut checker = HashChainChecker::new();

        // Add some records
        let hash0 = chain_hash(None, b"genesis");
        let zero_hash = ChainHash::from_bytes(&[0u8; 32]);
        checker.check_record(0, &zero_hash, &hash0);

        assert_eq!(checker.records_checked(), 1);

        // Reset
        checker.reset();

        assert_eq!(checker.records_checked(), 0);
        assert!(checker.last_offset().is_none());
        assert!(checker.last_hash().is_none());
    }

    #[test]
    fn log_consistency_checker_basic() {
        let mut checker = LogConsistencyChecker::new();

        let hash = ChainHash::from_bytes(&[1u8; 32]);
        let payload_hash = [2u8; 32];

        checker.record_commit(0, hash.clone(), payload_hash);

        // Verify matching read
        let result = checker.verify_read(0, &hash, &payload_hash);
        assert!(result.is_ok());

        // Verify unknown offset (should pass - no record)
        let result = checker.verify_read(1, &hash, &payload_hash);
        assert!(result.is_ok());
    }

    #[test]
    fn log_consistency_checker_detects_mismatch() {
        let mut checker = LogConsistencyChecker::new();

        let hash = ChainHash::from_bytes(&[1u8; 32]);
        let payload_hash = [2u8; 32];

        checker.record_commit(0, hash, payload_hash);

        // Verify with wrong chain hash
        let wrong_hash = ChainHash::from_bytes(&[3u8; 32]);
        let result = checker.verify_read(0, &wrong_hash, &payload_hash);
        assert!(!result.is_ok());

        // Verify with wrong payload hash
        let wrong_payload = [4u8; 32];
        let result = checker.verify_read(0, &hash, &wrong_payload);
        assert!(!result.is_ok());
    }

    // ========================================================================
    // Linearizability Checker Tests
    // ========================================================================

    #[test]
    fn linearizability_checker_empty_history() {
        let checker = LinearizabilityChecker::new();
        assert!(checker.check().is_ok());
    }

    #[test]
    fn linearizability_checker_single_write() {
        let mut checker = LinearizabilityChecker::new();

        let op_id = checker.invoke(1, 100, OpType::Write { key: 1, value: 42 });
        checker.respond(op_id, 200);

        assert!(checker.check().is_ok());
    }

    #[test]
    fn linearizability_checker_write_then_read() {
        let mut checker = LinearizabilityChecker::new();

        // Write completes before read starts
        let w_id = checker.invoke(1, 100, OpType::Write { key: 1, value: 42 });
        checker.respond(w_id, 200);

        // Read sees the written value
        let r_id = checker.invoke(
            2,
            300,
            OpType::Read {
                key: 1,
                value: Some(42),
            },
        );
        checker.respond(r_id, 400);

        assert!(checker.check().is_ok());
    }

    #[test]
    fn linearizability_checker_read_before_write() {
        let mut checker = LinearizabilityChecker::new();

        // Read completes before write starts - should see None
        let r_id = checker.invoke(
            1,
            100,
            OpType::Read {
                key: 1,
                value: None,
            },
        );
        checker.respond(r_id, 200);

        let w_id = checker.invoke(2, 300, OpType::Write { key: 1, value: 42 });
        checker.respond(w_id, 400);

        assert!(checker.check().is_ok());
    }

    #[test]
    fn linearizability_checker_concurrent_valid() {
        let mut checker = LinearizabilityChecker::new();

        // Concurrent write and read - read could see either value
        // Write: [100, 300]
        let w_id = checker.invoke(1, 100, OpType::Write { key: 1, value: 42 });

        // Read overlaps with write: [200, 400]
        // Read sees 42 (linearization: write happens at 250, read at 350)
        let r_id = checker.invoke(
            2,
            200,
            OpType::Read {
                key: 1,
                value: Some(42),
            },
        );

        checker.respond(w_id, 300);
        checker.respond(r_id, 400);

        assert!(checker.check().is_ok());
    }

    #[test]
    fn linearizability_checker_concurrent_also_valid() {
        let mut checker = LinearizabilityChecker::new();

        // Same overlap but read sees None
        // (linearization: read happens at 150, write at 250)
        let w_id = checker.invoke(1, 100, OpType::Write { key: 1, value: 42 });
        let r_id = checker.invoke(
            2,
            200,
            OpType::Read {
                key: 1,
                value: None,
            },
        );

        checker.respond(w_id, 300);
        checker.respond(r_id, 400);

        // This should be valid too - read can be linearized before write
        assert!(checker.check().is_ok());
    }

    #[test]
    fn linearizability_checker_violation() {
        let mut checker = LinearizabilityChecker::new();

        // Write completes at 200
        let w_id = checker.invoke(1, 100, OpType::Write { key: 1, value: 42 });
        checker.respond(w_id, 200);

        // Read starts at 300 (after write completed) but sees None
        // This is NOT linearizable - write happened-before read
        let r_id = checker.invoke(
            2,
            300,
            OpType::Read {
                key: 1,
                value: None,
            },
        );
        checker.respond(r_id, 400);

        assert!(!checker.check().is_ok());
    }

    #[test]
    fn linearizability_checker_multiple_keys_independent() {
        let mut checker = LinearizabilityChecker::new();

        // Operations on different keys are independent
        let w1 = checker.invoke(1, 100, OpType::Write { key: 1, value: 10 });
        let w2 = checker.invoke(2, 100, OpType::Write { key: 2, value: 20 });
        checker.respond(w1, 200);
        checker.respond(w2, 200);

        let r1 = checker.invoke(
            1,
            300,
            OpType::Read {
                key: 1,
                value: Some(10),
            },
        );
        let r2 = checker.invoke(
            2,
            300,
            OpType::Read {
                key: 2,
                value: Some(20),
            },
        );
        checker.respond(r1, 400);
        checker.respond(r2, 400);

        assert!(checker.check().is_ok());
    }

    #[test]
    fn linearizability_checker_reset() {
        let mut checker = LinearizabilityChecker::new();

        let op_id = checker.invoke(1, 100, OpType::Write { key: 1, value: 42 });
        checker.respond(op_id, 200);

        assert_eq!(checker.operation_count(), 1);

        checker.reset();

        assert_eq!(checker.operation_count(), 0);
    }

    // ========================================================================
    // Replica Consistency Checker Tests
    // ========================================================================

    #[test]
    fn replica_consistency_single_replica() {
        let mut checker = ReplicaConsistencyChecker::new();

        let result = checker.update_replica(1, 100, [1u8; 32], 1000);
        assert!(result.is_ok());

        assert_eq!(checker.replica_count(), 1);
    }

    #[test]
    fn replica_consistency_matching_replicas() {
        let mut checker = ReplicaConsistencyChecker::new();

        // Two replicas at same length with same hash
        let hash = [42u8; 32];
        assert!(checker.update_replica(1, 100, hash, 1000).is_ok());
        assert!(checker.update_replica(2, 100, hash, 1000).is_ok());

        assert!(checker.check_all().is_ok());
    }

    #[test]
    fn replica_consistency_different_lengths_ok() {
        let mut checker = ReplicaConsistencyChecker::new();

        // Replicas at different lengths can have different hashes
        assert!(checker.update_replica(1, 100, [1u8; 32], 1000).is_ok());
        assert!(checker.update_replica(2, 200, [2u8; 32], 1000).is_ok());

        assert!(checker.check_all().is_ok());
    }

    #[test]
    fn replica_consistency_detects_divergence() {
        let mut checker = ReplicaConsistencyChecker::new();

        // Two replicas at same length with DIFFERENT hashes
        assert!(checker.update_replica(1, 100, [1u8; 32], 1000).is_ok());

        let result = checker.update_replica(2, 100, [2u8; 32], 1000);
        assert!(!result.is_ok());

        match result {
            InvariantResult::Violated { invariant, .. } => {
                assert_eq!(invariant, "replica_consistency");
            }
            _ => panic!("expected violation"),
        }

        assert_eq!(checker.violation_count(), 1);
    }

    #[test]
    fn replica_consistency_check_all_detects_divergence() {
        let mut checker = ReplicaConsistencyChecker::new();

        // Add replicas without checking (simulating batch update)
        checker.replicas.insert(
            1,
            ReplicaState {
                replica_id: 1,
                log_length: 100,
                log_hash: [1u8; 32],
                last_update_ns: 1000,
            },
        );
        checker.replicas.insert(
            2,
            ReplicaState {
                replica_id: 2,
                log_length: 100,
                log_hash: [2u8; 32],
                last_update_ns: 1000,
            },
        );

        assert!(!checker.check_all().is_ok());
    }

    #[test]
    fn replica_consistency_three_replicas() {
        let mut checker = ReplicaConsistencyChecker::new();

        let hash = [42u8; 32];
        assert!(checker.update_replica(1, 100, hash, 1000).is_ok());
        assert!(checker.update_replica(2, 100, hash, 1000).is_ok());
        assert!(checker.update_replica(3, 100, hash, 1000).is_ok());

        assert!(checker.check_all().is_ok());
        assert_eq!(checker.replica_count(), 3);
    }

    #[test]
    fn replica_consistency_reset() {
        let mut checker = ReplicaConsistencyChecker::new();

        checker.update_replica(1, 100, [1u8; 32], 1000);
        let _ = checker.update_replica(2, 100, [2u8; 32], 1000);

        assert_eq!(checker.replica_count(), 2);
        assert_eq!(checker.violation_count(), 1);

        checker.reset();

        assert_eq!(checker.replica_count(), 0);
        assert_eq!(checker.violation_count(), 0);
    }

    #[test]
    fn replica_consistency_get_replica() {
        let mut checker = ReplicaConsistencyChecker::new();

        checker.update_replica(1, 100, [42u8; 32], 1000);

        let state = checker.get_replica(1).expect("replica should exist");
        assert_eq!(state.replica_id, 1);
        assert_eq!(state.log_length, 100);
        assert_eq!(state.log_hash, [42u8; 32]);

        assert!(checker.get_replica(999).is_none());
    }
}
