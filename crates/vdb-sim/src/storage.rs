//! Simulated storage for deterministic testing.
//!
//! The `SimStorage` models disk I/O with configurable:
//! - Write latency
//! - Read latency
//! - Write failures (corruption, partial writes)
//! - Read failures (bit flips, missing data)
//! - Fsync failures
//!
//! All behavior is deterministic based on the simulation's RNG seed.

use std::collections::HashMap;

use crate::rng::SimRng;

// ============================================================================
// Storage Configuration
// ============================================================================

/// Configuration for simulated storage behavior.
#[derive(Debug, Clone)]
pub struct StorageConfig {
    /// Minimum write latency in nanoseconds.
    pub min_write_latency_ns: u64,
    /// Maximum write latency in nanoseconds.
    pub max_write_latency_ns: u64,
    /// Minimum read latency in nanoseconds.
    pub min_read_latency_ns: u64,
    /// Maximum read latency in nanoseconds.
    pub max_read_latency_ns: u64,
    /// Probability of write failure (0.0 to 1.0).
    pub write_failure_probability: f64,
    /// Probability of read corruption (bit flip) (0.0 to 1.0).
    pub read_corruption_probability: f64,
    /// Probability of fsync failure (0.0 to 1.0).
    pub fsync_failure_probability: f64,
    /// Probability of partial write (0.0 to 1.0).
    pub partial_write_probability: f64,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            min_write_latency_ns: 10_000,     // 10μs
            max_write_latency_ns: 1_000_000,  // 1ms
            min_read_latency_ns: 5_000,       // 5μs
            max_read_latency_ns: 500_000,     // 0.5ms
            write_failure_probability: 0.0,
            read_corruption_probability: 0.0,
            fsync_failure_probability: 0.0,
            partial_write_probability: 0.0,
        }
    }
}

impl StorageConfig {
    /// Creates a reliable storage configuration (no failures).
    pub fn reliable() -> Self {
        Self::default()
    }

    /// Creates a storage configuration with write failures.
    pub fn with_write_failures(probability: f64) -> Self {
        Self {
            write_failure_probability: probability,
            ..Self::default()
        }
    }

    /// Creates a storage configuration with read corruption.
    pub fn with_corruption(probability: f64) -> Self {
        Self {
            read_corruption_probability: probability,
            ..Self::default()
        }
    }

    /// Creates a slow storage configuration.
    pub fn slow() -> Self {
        Self {
            min_write_latency_ns: 1_000_000,   // 1ms
            max_write_latency_ns: 50_000_000,  // 50ms
            min_read_latency_ns: 500_000,      // 0.5ms
            max_read_latency_ns: 10_000_000,   // 10ms
            ..Self::default()
        }
    }
}

// ============================================================================
// Storage Operations
// ============================================================================

/// Result of a storage write operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteResult {
    /// Write completed successfully.
    Success {
        /// Latency in nanoseconds.
        latency_ns: u64,
        /// Bytes written.
        bytes_written: usize,
    },
    /// Write failed completely.
    Failed {
        /// Latency until failure.
        latency_ns: u64,
        /// Reason for failure.
        reason: WriteFailure,
    },
    /// Partial write (torn write).
    Partial {
        /// Latency in nanoseconds.
        latency_ns: u64,
        /// Bytes actually written.
        bytes_written: usize,
        /// Total bytes requested.
        bytes_requested: usize,
    },
}

/// Reason for write failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteFailure {
    /// Disk is full.
    DiskFull,
    /// I/O error.
    IoError,
    /// Permission denied.
    PermissionDenied,
}

/// Result of a storage read operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadResult {
    /// Read completed successfully.
    Success {
        /// Latency in nanoseconds.
        latency_ns: u64,
        /// Data read.
        data: Vec<u8>,
    },
    /// Data was corrupted (bit flip detected or injected).
    Corrupted {
        /// Latency in nanoseconds.
        latency_ns: u64,
        /// Corrupted data.
        data: Vec<u8>,
        /// Original data (for debugging).
        original: Vec<u8>,
    },
    /// Read failed (data not found).
    NotFound {
        /// Latency in nanoseconds.
        latency_ns: u64,
    },
}

/// Result of an fsync operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FsyncResult {
    /// Fsync completed successfully.
    Success {
        /// Latency in nanoseconds.
        latency_ns: u64,
    },
    /// Fsync failed.
    Failed {
        /// Latency in nanoseconds.
        latency_ns: u64,
    },
}

// ============================================================================
// Simulated Storage
// ============================================================================

/// Simulated block storage for deterministic testing.
///
/// Models a simple key-value block store with configurable failure injection.
#[derive(Debug)]
pub struct SimStorage {
    /// Storage configuration.
    config: StorageConfig,
    /// Stored data blocks, keyed by address.
    blocks: HashMap<u64, Vec<u8>>,
    /// Pending writes (not yet fsynced).
    pending_writes: HashMap<u64, Vec<u8>>,
    /// Whether there are unfsynced writes.
    dirty: bool,
    /// Statistics.
    stats: StorageStats,
}

/// Storage statistics for monitoring.
#[derive(Debug, Clone, Default)]
pub struct StorageStats {
    /// Total write operations.
    pub writes: u64,
    /// Successful writes.
    pub writes_successful: u64,
    /// Failed writes.
    pub writes_failed: u64,
    /// Partial writes.
    pub writes_partial: u64,
    /// Total read operations.
    pub reads: u64,
    /// Successful reads.
    pub reads_successful: u64,
    /// Corrupted reads.
    pub reads_corrupted: u64,
    /// Not found reads.
    pub reads_not_found: u64,
    /// Total fsync operations.
    pub fsyncs: u64,
    /// Successful fsyncs.
    pub fsyncs_successful: u64,
    /// Failed fsyncs.
    pub fsyncs_failed: u64,
    /// Total bytes written.
    pub bytes_written: u64,
    /// Total bytes read.
    pub bytes_read: u64,
}

impl SimStorage {
    /// Creates a new simulated storage with the given configuration.
    pub fn new(config: StorageConfig) -> Self {
        Self {
            config,
            blocks: HashMap::new(),
            pending_writes: HashMap::new(),
            dirty: false,
            stats: StorageStats::default(),
        }
    }

    /// Creates storage with default (reliable) configuration.
    pub fn reliable() -> Self {
        Self::new(StorageConfig::reliable())
    }

    /// Writes data to the given address.
    ///
    /// The write is buffered until `fsync` is called.
    pub fn write(&mut self, address: u64, data: Vec<u8>, rng: &mut SimRng) -> WriteResult {
        self.stats.writes += 1;
        let data_len = data.len();

        // Calculate latency
        let latency_ns = rng.delay_ns(
            self.config.min_write_latency_ns,
            self.config.max_write_latency_ns,
        );

        // Check for write failure
        if self.config.write_failure_probability > 0.0
            && rng.next_bool_with_probability(self.config.write_failure_probability)
        {
            self.stats.writes_failed += 1;
            return WriteResult::Failed {
                latency_ns,
                reason: WriteFailure::IoError,
            };
        }

        // Check for partial write
        if self.config.partial_write_probability > 0.0
            && rng.next_bool_with_probability(self.config.partial_write_probability)
        {
            self.stats.writes_partial += 1;
            // Write only a portion of the data
            let partial_len = if data_len > 1 {
                rng.next_usize(data_len - 1) + 1
            } else {
                0
            };
            let partial_data = data[..partial_len].to_vec();
            self.pending_writes.insert(address, partial_data);
            self.dirty = true;
            self.stats.bytes_written += partial_len as u64;

            return WriteResult::Partial {
                latency_ns,
                bytes_written: partial_len,
                bytes_requested: data_len,
            };
        }

        // Successful write (to pending buffer)
        self.stats.writes_successful += 1;
        self.stats.bytes_written += data_len as u64;
        self.pending_writes.insert(address, data);
        self.dirty = true;

        WriteResult::Success {
            latency_ns,
            bytes_written: data_len,
        }
    }

    /// Reads data from the given address.
    pub fn read(&mut self, address: u64, rng: &mut SimRng) -> ReadResult {
        self.stats.reads += 1;

        // Calculate latency
        let latency_ns = rng.delay_ns(
            self.config.min_read_latency_ns,
            self.config.max_read_latency_ns,
        );

        // Check pending writes first (read-your-writes consistency)
        let data = self
            .pending_writes
            .get(&address)
            .or_else(|| self.blocks.get(&address));

        if let Some(data) = data {
            let data = data.clone();
            self.stats.bytes_read += data.len() as u64;

            // Check for read corruption
            if self.config.read_corruption_probability > 0.0
                && rng.next_bool_with_probability(self.config.read_corruption_probability)
            {
                self.stats.reads_corrupted += 1;
                let mut corrupted = data.clone();
                if !corrupted.is_empty() {
                    // Flip a random bit
                    let byte_idx = rng.next_usize(corrupted.len());
                    let bit_idx = rng.next_usize(8);
                    corrupted[byte_idx] ^= 1 << bit_idx;
                }
                return ReadResult::Corrupted {
                    latency_ns,
                    data: corrupted,
                    original: data,
                };
            }

            self.stats.reads_successful += 1;
            ReadResult::Success { latency_ns, data }
        } else {
            self.stats.reads_not_found += 1;
            ReadResult::NotFound { latency_ns }
        }
    }

    /// Flushes pending writes to durable storage.
    pub fn fsync(&mut self, rng: &mut SimRng) -> FsyncResult {
        self.stats.fsyncs += 1;

        // Calculate latency (fsync is typically slow)
        let latency_ns = rng.delay_ns(
            self.config.min_write_latency_ns * 10,
            self.config.max_write_latency_ns * 10,
        );

        // Check for fsync failure
        if self.config.fsync_failure_probability > 0.0
            && rng.next_bool_with_probability(self.config.fsync_failure_probability)
        {
            self.stats.fsyncs_failed += 1;
            // On fsync failure, pending writes are lost
            self.pending_writes.clear();
            self.dirty = false;
            return FsyncResult::Failed { latency_ns };
        }

        // Move pending writes to durable storage
        for (address, data) in self.pending_writes.drain() {
            self.blocks.insert(address, data);
        }
        self.dirty = false;
        self.stats.fsyncs_successful += 1;

        FsyncResult::Success { latency_ns }
    }

    /// Simulates a crash - loses all pending (unfsynced) writes.
    pub fn crash(&mut self) {
        self.pending_writes.clear();
        self.dirty = false;
    }

    /// Returns whether there are unfsynced writes.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Returns the number of stored blocks.
    pub fn block_count(&self) -> usize {
        self.blocks.len()
    }

    /// Returns storage statistics.
    pub fn stats(&self) -> &StorageStats {
        &self.stats
    }

    /// Returns the configuration.
    pub fn config(&self) -> &StorageConfig {
        &self.config
    }

    /// Checks if an address exists in durable storage.
    pub fn exists(&self, address: u64) -> bool {
        self.blocks.contains_key(&address)
    }

    /// Deletes a block from storage.
    pub fn delete(&mut self, address: u64) -> bool {
        self.pending_writes.remove(&address);
        self.blocks.remove(&address).is_some()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_and_read_basic() {
        let mut storage = SimStorage::reliable();
        let mut rng = SimRng::new(42);

        let result = storage.write(0, b"hello".to_vec(), &mut rng);
        assert!(matches!(result, WriteResult::Success { .. }));

        // Fsync to make durable
        let result = storage.fsync(&mut rng);
        assert!(matches!(result, FsyncResult::Success { .. }));

        // Read back
        let result = storage.read(0, &mut rng);
        match result {
            ReadResult::Success { data, .. } => {
                assert_eq!(data, b"hello");
            }
            _ => panic!("expected Success"),
        }
    }

    #[test]
    fn read_your_writes_before_fsync() {
        let mut storage = SimStorage::reliable();
        let mut rng = SimRng::new(42);

        storage.write(0, b"pending".to_vec(), &mut rng);

        // Should be able to read pending write
        let result = storage.read(0, &mut rng);
        match result {
            ReadResult::Success { data, .. } => {
                assert_eq!(data, b"pending");
            }
            _ => panic!("expected Success"),
        }
    }

    #[test]
    fn crash_loses_pending_writes() {
        let mut storage = SimStorage::reliable();
        let mut rng = SimRng::new(42);

        storage.write(0, b"will be lost".to_vec(), &mut rng);
        assert!(storage.is_dirty());

        storage.crash();

        assert!(!storage.is_dirty());
        let result = storage.read(0, &mut rng);
        assert!(matches!(result, ReadResult::NotFound { .. }));
    }

    #[test]
    fn fsynced_data_survives_crash() {
        let mut storage = SimStorage::reliable();
        let mut rng = SimRng::new(42);

        storage.write(0, b"durable".to_vec(), &mut rng);
        storage.fsync(&mut rng);
        storage.crash();

        let result = storage.read(0, &mut rng);
        match result {
            ReadResult::Success { data, .. } => {
                assert_eq!(data, b"durable");
            }
            _ => panic!("expected Success"),
        }
    }

    #[test]
    fn write_failure() {
        let config = StorageConfig {
            write_failure_probability: 1.0,
            ..StorageConfig::default()
        };
        let mut storage = SimStorage::new(config);
        let mut rng = SimRng::new(42);

        let result = storage.write(0, b"will fail".to_vec(), &mut rng);
        assert!(matches!(result, WriteResult::Failed { .. }));
        assert_eq!(storage.stats().writes_failed, 1);
    }

    #[test]
    fn read_corruption() {
        let config = StorageConfig {
            read_corruption_probability: 1.0,
            ..StorageConfig::default()
        };
        let mut storage = SimStorage::new(config);
        let mut rng = SimRng::new(42);

        storage.write(0, b"hello".to_vec(), &mut rng);
        storage.fsync(&mut rng);

        let result = storage.read(0, &mut rng);
        match result {
            ReadResult::Corrupted { data, original, .. } => {
                assert_eq!(original, b"hello");
                assert_ne!(data, original); // Should be corrupted
            }
            _ => panic!("expected Corrupted"),
        }
    }

    #[test]
    fn fsync_failure() {
        let config = StorageConfig {
            fsync_failure_probability: 1.0,
            ..StorageConfig::default()
        };
        let mut storage = SimStorage::new(config);
        let mut rng = SimRng::new(42);

        storage.write(0, b"will be lost".to_vec(), &mut rng);
        let result = storage.fsync(&mut rng);
        assert!(matches!(result, FsyncResult::Failed { .. }));

        // Pending writes should be cleared
        assert!(!storage.is_dirty());
        let result = storage.read(0, &mut rng);
        assert!(matches!(result, ReadResult::NotFound { .. }));
    }

    #[test]
    fn partial_write() {
        let config = StorageConfig {
            partial_write_probability: 1.0,
            ..StorageConfig::default()
        };
        let mut storage = SimStorage::new(config);
        let mut rng = SimRng::new(42);

        let data = b"hello world".to_vec();
        let result = storage.write(0, data.clone(), &mut rng);

        match result {
            WriteResult::Partial {
                bytes_written,
                bytes_requested,
                ..
            } => {
                assert!(bytes_written < bytes_requested);
                assert_eq!(bytes_requested, data.len());
            }
            _ => panic!("expected Partial"),
        }
    }

    #[test]
    fn stats_tracking() {
        let mut storage = SimStorage::reliable();
        let mut rng = SimRng::new(42);

        storage.write(0, b"data".to_vec(), &mut rng);
        storage.write(1, b"more".to_vec(), &mut rng);
        storage.fsync(&mut rng);
        storage.read(0, &mut rng);
        storage.read(1, &mut rng);
        storage.read(2, &mut rng); // Not found

        assert_eq!(storage.stats().writes, 2);
        assert_eq!(storage.stats().writes_successful, 2);
        assert_eq!(storage.stats().fsyncs, 1);
        assert_eq!(storage.stats().fsyncs_successful, 1);
        assert_eq!(storage.stats().reads, 3);
        assert_eq!(storage.stats().reads_successful, 2);
        assert_eq!(storage.stats().reads_not_found, 1);
    }

    #[test]
    fn delete_block() {
        let mut storage = SimStorage::reliable();
        let mut rng = SimRng::new(42);

        storage.write(0, b"data".to_vec(), &mut rng);
        storage.fsync(&mut rng);

        assert!(storage.exists(0));
        assert!(storage.delete(0));
        assert!(!storage.exists(0));

        let result = storage.read(0, &mut rng);
        assert!(matches!(result, ReadResult::NotFound { .. }));
    }
}
