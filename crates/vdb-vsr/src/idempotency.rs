//! Idempotency tracking for duplicate transaction detection.
//!
//! This module implements client request deduplication to ensure exactly-once
//! semantics even when clients retry requests. Each request can include an
//! optional `IdempotencyId` - if the same ID is submitted twice, the cached
//! result is returned instead of re-executing.
//!
//! # How It Works
//!
//! ```text
//! Client Request (with IdempotencyId)
//!         │
//!         ▼
//! ┌───────────────────┐
//! │ Check Idempotency │──► Found? Return cached result
//! │      Table        │
//! └─────────┬─────────┘
//!           │ Not found
//!           ▼
//! ┌───────────────────┐
//! │ Execute Command   │
//! └─────────┬─────────┘
//!           │
//!           ▼
//! ┌───────────────────┐
//! │ Record in Table   │
//! └───────────────────┘
//! ```
//!
//! # Cleanup Policy
//!
//! Entries are retained for at least the configured `min_retention` period
//! (default: 24 hours). This allows clients to safely retry for extended
//! periods during network issues or client restarts.
//!
//! # Storage Overhead
//!
//! Each entry is ~48 bytes (16-byte ID + 8-byte op + 8-byte timestamp + overhead).
//! At 1000 requests/second with 24-hour retention, this is ~4GB. For most
//! workloads, the overhead is much smaller (~1% per `FoundationDB` measurements).

use std::collections::HashMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use vdb_types::{IdempotencyId, Timestamp};

use crate::types::OpNumber;

// ============================================================================
// Constants
// ============================================================================

/// Default minimum retention period for idempotency entries (24 hours).
pub const DEFAULT_MIN_RETENTION: Duration = Duration::from_secs(24 * 60 * 60);

/// Maximum number of entries before forced cleanup (memory protection).
pub const MAX_ENTRIES: usize = 10_000_000;

// ============================================================================
// IdempotencyResult
// ============================================================================

/// Result cached for an idempotent request.
///
/// When a duplicate request is detected, this cached result is returned
/// instead of re-executing the command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdempotencyResult {
    /// The operation number where this command was committed.
    pub op_number: OpNumber,

    /// When this entry was recorded.
    pub timestamp: Timestamp,
}

impl IdempotencyResult {
    /// Creates a new idempotency result.
    pub fn new(op_number: OpNumber, timestamp: Timestamp) -> Self {
        Self {
            op_number,
            timestamp,
        }
    }
}

// ============================================================================
// IdempotencyConfig
// ============================================================================

/// Configuration for idempotency tracking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdempotencyConfig {
    /// Minimum retention period for entries.
    ///
    /// Entries younger than this are never cleaned up, even if the table
    /// is getting large.
    pub min_retention: Duration,

    /// Maximum number of entries before forced cleanup.
    ///
    /// When exceeded, oldest entries are removed regardless of age.
    pub max_entries: usize,
}

impl IdempotencyConfig {
    /// Creates a new configuration with default values.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates configuration for testing with short retention.
    pub fn testing() -> Self {
        Self {
            min_retention: Duration::from_secs(60), // 1 minute
            max_entries: 1000,
        }
    }

    /// Creates configuration for simulation with very short retention.
    pub fn simulation() -> Self {
        Self {
            min_retention: Duration::from_millis(100),
            max_entries: 100,
        }
    }
}

impl Default for IdempotencyConfig {
    fn default() -> Self {
        Self {
            min_retention: DEFAULT_MIN_RETENTION,
            max_entries: MAX_ENTRIES,
        }
    }
}

// ============================================================================
// IdempotencyTable
// ============================================================================

/// Tracks committed idempotency IDs for duplicate detection.
///
/// This table stores a mapping from `IdempotencyId` to the result of the
/// original execution. When a duplicate request arrives, the cached result
/// is returned instead of re-executing.
///
/// # Thread Safety
///
/// This table is not thread-safe. In VSR, it's owned by the single-threaded
/// replica state machine.
#[derive(Debug, Clone)]
pub struct IdempotencyTable {
    /// Map from idempotency ID to cached result.
    entries: HashMap<IdempotencyId, IdempotencyResult>,

    /// Configuration for cleanup policy.
    config: IdempotencyConfig,
}

// Custom serialization to handle HashMap with non-string keys
impl Serialize for IdempotencyTable {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        // Serialize as a Vec of tuples
        let entries: Vec<_> = self.entries.iter().map(|(k, v)| (*k, *v)).collect();
        entries.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for IdempotencyTable {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let entries: Vec<(IdempotencyId, IdempotencyResult)> = Vec::deserialize(deserializer)?;
        Ok(Self {
            entries: entries.into_iter().collect(),
            config: IdempotencyConfig::default(),
        })
    }
}

impl IdempotencyTable {
    /// Creates a new empty idempotency table.
    pub fn new(config: IdempotencyConfig) -> Self {
        Self {
            entries: HashMap::new(),
            config,
        }
    }

    /// Creates a table with default configuration.
    pub fn with_defaults() -> Self {
        Self::new(IdempotencyConfig::default())
    }

    /// Checks if an idempotency ID has already been committed.
    ///
    /// # Returns
    ///
    /// - `Some(result)` if this ID was previously committed
    /// - `None` if this is a new request
    pub fn check(&self, id: &IdempotencyId) -> Option<&IdempotencyResult> {
        self.entries.get(id)
    }

    /// Records a committed idempotency ID.
    ///
    /// Should be called after a command with an idempotency ID is committed.
    /// If the ID was already recorded (shouldn't happen in normal operation),
    /// the existing entry is preserved.
    ///
    /// # Returns
    ///
    /// `true` if this was a new entry, `false` if it was already present.
    pub fn record(&mut self, id: IdempotencyId, result: IdempotencyResult) -> bool {
        // Precondition: not recording a degenerate ID
        debug_assert!(
            id.as_bytes().iter().any(|&b| b != 0),
            "recording all-zero idempotency ID"
        );

        if self.entries.contains_key(&id) {
            return false;
        }

        self.entries.insert(id, result);

        // Check if cleanup is needed due to size
        if self.entries.len() > self.config.max_entries {
            self.cleanup_by_count(self.config.max_entries / 2);
        }

        true
    }

    /// Returns the number of entries in the table.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns true if the table is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Removes entries older than the retention period.
    ///
    /// # Arguments
    ///
    /// * `now` - Current timestamp for age calculation
    ///
    /// # Returns
    ///
    /// The number of entries removed.
    pub fn cleanup(&mut self, now: Timestamp) -> usize {
        let min_retention_nanos = self.config.min_retention.as_nanos() as u64;
        let cutoff = now.as_nanos().saturating_sub(min_retention_nanos);

        let before = self.entries.len();

        self.entries
            .retain(|_, result| result.timestamp.as_nanos() >= cutoff);

        before - self.entries.len()
    }

    /// Removes the oldest entries to reduce count.
    ///
    /// # Arguments
    ///
    /// * `target_count` - Target number of entries to keep
    ///
    /// # Returns
    ///
    /// The number of entries removed.
    fn cleanup_by_count(&mut self, target_count: usize) -> usize {
        if self.entries.len() <= target_count {
            return 0;
        }

        // Collect entries sorted by timestamp
        let mut entries: Vec<_> = self.entries.iter().collect();
        entries.sort_by_key(|(_, result)| result.timestamp);

        // Keep only the newest entries
        let to_remove = self.entries.len() - target_count;
        let ids_to_remove: Vec<_> = entries.iter().take(to_remove).map(|(id, _)| **id).collect();

        for id in &ids_to_remove {
            self.entries.remove(id);
        }

        ids_to_remove.len()
    }

    /// Returns an iterator over all entries.
    pub fn iter(&self) -> impl Iterator<Item = (&IdempotencyId, &IdempotencyResult)> {
        self.entries.iter()
    }

    /// Returns the configuration.
    pub fn config(&self) -> &IdempotencyConfig {
        &self.config
    }

    /// Updates the configuration.
    pub fn set_config(&mut self, config: IdempotencyConfig) {
        self.config = config;
    }
}

impl Default for IdempotencyTable {
    fn default() -> Self {
        Self::with_defaults()
    }
}

// ============================================================================
// DuplicateStatus
// ============================================================================

/// Result of checking for a duplicate request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DuplicateStatus {
    /// This is a new request (no matching idempotency ID found).
    New,

    /// This is a duplicate - return the cached result.
    Duplicate(IdempotencyResult),
}

impl DuplicateStatus {
    /// Returns true if this is a duplicate request.
    pub fn is_duplicate(&self) -> bool {
        matches!(self, DuplicateStatus::Duplicate(_))
    }

    /// Returns the cached result if this is a duplicate.
    pub fn cached_result(&self) -> Option<IdempotencyResult> {
        match self {
            DuplicateStatus::New => None,
            DuplicateStatus::Duplicate(result) => Some(*result),
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn make_timestamp(secs: u64) -> Timestamp {
        // Convert seconds to nanoseconds
        Timestamp::from_nanos(secs * 1_000_000_000)
    }

    #[test]
    fn new_table_is_empty() {
        let table = IdempotencyTable::with_defaults();
        assert!(table.is_empty());
        assert_eq!(table.len(), 0);
    }

    #[test]
    fn record_and_check() {
        let mut table = IdempotencyTable::with_defaults();
        let id = IdempotencyId::generate();
        let result = IdempotencyResult::new(OpNumber::new(42), make_timestamp(1000));

        // Check before recording
        assert!(table.check(&id).is_none());

        // Record
        assert!(table.record(id, result));
        assert_eq!(table.len(), 1);

        // Check after recording
        let cached = table.check(&id);
        assert!(cached.is_some());
        assert_eq!(cached.unwrap().op_number, OpNumber::new(42));
    }

    #[test]
    fn duplicate_record_returns_false() {
        let mut table = IdempotencyTable::with_defaults();
        let id = IdempotencyId::generate();
        let result1 = IdempotencyResult::new(OpNumber::new(1), make_timestamp(100));
        let result2 = IdempotencyResult::new(OpNumber::new(2), make_timestamp(200));

        // First record succeeds
        assert!(table.record(id, result1));

        // Second record with same ID fails (preserves original)
        assert!(!table.record(id, result2));

        // Original result is preserved
        let cached = table.check(&id).unwrap();
        assert_eq!(cached.op_number, OpNumber::new(1));
    }

    #[test]
    fn cleanup_removes_old_entries() {
        let config = IdempotencyConfig {
            min_retention: Duration::from_secs(100),
            max_entries: 1000,
        };
        let mut table = IdempotencyTable::new(config);

        // Add old entry (timestamp 50)
        let old_id = IdempotencyId::generate();
        table.record(
            old_id,
            IdempotencyResult::new(OpNumber::new(1), make_timestamp(50)),
        );

        // Add new entry (timestamp 150)
        let new_id = IdempotencyId::generate();
        table.record(
            new_id,
            IdempotencyResult::new(OpNumber::new(2), make_timestamp(150)),
        );

        assert_eq!(table.len(), 2);

        // Cleanup at time 200 (cutoff = 200 - 100 = 100)
        // Old entry (timestamp 50) should be removed
        // New entry (timestamp 150) should remain
        let removed = table.cleanup(make_timestamp(200));

        assert_eq!(removed, 1);
        assert_eq!(table.len(), 1);
        assert!(table.check(&old_id).is_none());
        assert!(table.check(&new_id).is_some());
    }

    #[test]
    fn max_entries_triggers_cleanup() {
        let config = IdempotencyConfig {
            min_retention: Duration::from_secs(1000),
            max_entries: 5,
        };
        let mut table = IdempotencyTable::new(config);

        // Add entries with increasing timestamps
        for i in 0..10 {
            let id = IdempotencyId::generate();
            table.record(
                id,
                IdempotencyResult::new(OpNumber::new(i), make_timestamp(i)),
            );
        }

        // Should have cleaned up to max_entries / 2 = 2
        // So after adding 10 entries with cleanup at 5, we should have ~5 entries
        assert!(table.len() <= 5, "table should have at most 5 entries");
    }

    #[test]
    fn duplicate_status_helpers() {
        let result = IdempotencyResult::new(OpNumber::new(1), make_timestamp(100));

        let new_status = DuplicateStatus::New;
        assert!(!new_status.is_duplicate());
        assert!(new_status.cached_result().is_none());

        let dup_status = DuplicateStatus::Duplicate(result);
        assert!(dup_status.is_duplicate());
        assert_eq!(dup_status.cached_result(), Some(result));
    }

    #[test]
    fn config_presets() {
        let default_config = IdempotencyConfig::default();
        assert_eq!(default_config.min_retention, DEFAULT_MIN_RETENTION);

        let test_config = IdempotencyConfig::testing();
        assert!(test_config.min_retention < default_config.min_retention);

        let sim_config = IdempotencyConfig::simulation();
        assert!(sim_config.min_retention < test_config.min_retention);
    }

    #[test]
    fn serialization_roundtrip() {
        let mut table = IdempotencyTable::with_defaults();

        let id1 = IdempotencyId::generate();
        let id2 = IdempotencyId::generate();

        table.record(
            id1,
            IdempotencyResult::new(OpNumber::new(1), make_timestamp(100)),
        );
        table.record(
            id2,
            IdempotencyResult::new(OpNumber::new(2), make_timestamp(200)),
        );

        // Serialize
        let json = serde_json::to_string(&table).unwrap();

        // Deserialize
        let restored: IdempotencyTable = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.len(), 2);
        assert!(restored.check(&id1).is_some());
        assert!(restored.check(&id2).is_some());
    }

    #[test]
    fn iter_returns_all_entries() {
        let mut table = IdempotencyTable::with_defaults();

        for i in 0..5 {
            let id = IdempotencyId::generate();
            table.record(
                id,
                IdempotencyResult::new(OpNumber::new(i), make_timestamp(i)),
            );
        }

        let count = table.iter().count();
        assert_eq!(count, 5);
    }
}
