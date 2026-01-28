//! # vdb-types: Core types for `VerityDB`
//!
//! This crate contains shared types used across the `VerityDB` system:
//! - Entity IDs ([`TenantId`], [`StreamId`], [`Offset`], [`GroupId`])
//! - Cryptographic types ([`Hash`])
//! - Temporal types ([`Timestamp`])
//! - Log record types ([`RecordKind`], [`RecordHeader`], [`Checkpoint`], [`CheckpointPolicy`])
//! - Projection tracking ([`AppliedIndex`])
//! - Idempotency ([`IdempotencyId`])
//! - Recovery tracking ([`Generation`], [`RecoveryRecord`], [`RecoveryReason`])
//! - Data classification ([`DataClass`])
//! - Placement rules ([`Placement`], [`Region`])
//! - Stream metadata ([`StreamMetadata`])
//! - Audit actions ([`AuditAction`])
//! - Event persistence ([`EventPersister`], [`PersistError`])

use std::{
    fmt::{Debug, Display},
    ops::{Add, AddAssign, Range, Sub},
    time::{SystemTime, UNIX_EPOCH},
};

use bytes::Bytes;
use serde::{Deserialize, Serialize};

// ============================================================================
// Entity IDs - All Copy (cheap 8-byte values)
// ============================================================================

/// Unique identifier for a tenant (organization/customer).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct TenantId(u64);

impl TenantId {
    pub fn new(id: u64) -> Self {
        Self(id)
    }
}

impl From<u64> for TenantId {
    fn from(value: u64) -> Self {
        Self(value)
    }
}

impl From<TenantId> for u64 {
    fn from(id: TenantId) -> Self {
        id.0
    }
}

/// Unique identifier for a stream within the system.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default,
)]
pub struct StreamId(u64);

impl StreamId {
    pub fn new(id: u64) -> Self {
        Self(id)
    }
}

impl Add for StreamId {
    type Output = StreamId;

    fn add(self, rhs: Self) -> Self::Output {
        let v = self.0 + rhs.0;
        StreamId::new(v)
    }
}

impl Display for StreamId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<u64> for StreamId {
    fn from(value: u64) -> Self {
        Self(value)
    }
}

impl From<StreamId> for u64 {
    fn from(id: StreamId) -> Self {
        id.0
    }
}

/// Position of an event within a stream.
///
/// Offsets are zero-indexed and sequential. The first event in a stream
/// has offset 0, the second has offset 1, and so on.
///
/// Uses `u64` internally — offsets are never negative by definition.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default,
)]
pub struct Offset(u64);

impl Offset {
    pub const ZERO: Offset = Offset(0);

    pub fn new(offset: u64) -> Self {
        Self(offset)
    }

    /// Returns the offset as a `u64`.
    pub fn as_u64(&self) -> u64 {
        self.0
    }

    /// Returns the offset as a `usize` for indexing.
    ///
    /// # Panics
    ///
    /// Panics on 32-bit platforms if the offset exceeds `usize::MAX`.
    pub fn as_usize(&self) -> usize {
        self.0 as usize
    }
}

impl Display for Offset {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Add for Offset {
    type Output = Self;
    fn add(self, rhs: Self) -> Self::Output {
        Self(self.0 + rhs.0)
    }
}

impl AddAssign for Offset {
    fn add_assign(&mut self, rhs: Self) {
        self.0 += rhs.0;
    }
}

impl Sub for Offset {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self::Output {
        Self(self.0 - rhs.0)
    }
}

impl From<u64> for Offset {
    fn from(value: u64) -> Self {
        Self(value)
    }
}

impl From<Offset> for u64 {
    fn from(offset: Offset) -> Self {
        offset.0
    }
}

/// Unique identifier for a replication group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct GroupId(u64);

impl Display for GroupId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl GroupId {
    pub fn new(id: u64) -> Self {
        Self(id)
    }
}

impl From<u64> for GroupId {
    fn from(value: u64) -> Self {
        Self(value)
    }
}

impl From<GroupId> for u64 {
    fn from(id: GroupId) -> Self {
        id.0
    }
}

// ============================================================================
// Cryptographic Hash - Copy (fixed 32-byte value)
// ============================================================================

/// Length of cryptographic hashes in bytes (SHA-256 / BLAKE3).
pub const HASH_LENGTH: usize = 32;

/// A 32-byte cryptographic hash.
///
/// This is a foundation type used across `VerityDB` for:
/// - Hash chain links (`prev_hash` in records)
/// - Verification anchors (in checkpoints and projections)
/// - Content addressing
///
/// The specific algorithm (SHA-256 for compliance, BLAKE3 for internal)
/// is determined by the context where the hash is computed. This type
/// only stores the resulting 32-byte digest.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Hash([u8; HASH_LENGTH]);

impl Hash {
    /// The genesis hash (all zeros) used as the `prev_hash` for the first record.
    pub const GENESIS: Hash = Hash([0u8; HASH_LENGTH]);

    /// Creates a hash from raw bytes.
    pub fn from_bytes(bytes: [u8; HASH_LENGTH]) -> Self {
        Self(bytes)
    }

    /// Returns the hash as a byte slice.
    pub fn as_bytes(&self) -> &[u8; HASH_LENGTH] {
        &self.0
    }

    /// Returns true if this is the genesis hash (all zeros).
    pub fn is_genesis(&self) -> bool {
        self.0 == [0u8; HASH_LENGTH]
    }
}

impl Debug for Hash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Show first 8 bytes in hex for debugging without exposing full hash
        write!(
            f,
            "Hash({:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}...)",
            self.0[0], self.0[1], self.0[2], self.0[3], self.0[4], self.0[5], self.0[6], self.0[7]
        )
    }
}

impl Display for Hash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Full hex representation for display
        for byte in &self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl Default for Hash {
    fn default() -> Self {
        Self::GENESIS
    }
}

impl From<[u8; HASH_LENGTH]> for Hash {
    fn from(bytes: [u8; HASH_LENGTH]) -> Self {
        Self(bytes)
    }
}

impl From<Hash> for [u8; HASH_LENGTH] {
    fn from(hash: Hash) -> Self {
        hash.0
    }
}

impl AsRef<[u8]> for Hash {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

// ============================================================================
// Timestamp - Copy (8-byte value with monotonic guarantee)
// ============================================================================

/// Wall-clock timestamp with monotonic guarantee within the system.
///
/// Compliance requires real-world time for audit trails; monotonicity
/// prevents ordering issues when system clocks are adjusted.
///
/// Stored as nanoseconds since Unix epoch (1970-01-01 00:00:00 UTC).
/// This gives us ~584 years of range, well beyond any practical use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Timestamp(u64);

impl Timestamp {
    /// The Unix epoch (1970-01-01 00:00:00 UTC).
    pub const EPOCH: Timestamp = Timestamp(0);

    /// Creates a timestamp from nanoseconds since Unix epoch.
    pub fn from_nanos(nanos: u64) -> Self {
        Self(nanos)
    }

    /// Returns the timestamp as nanoseconds since Unix epoch.
    pub fn as_nanos(&self) -> u64 {
        self.0
    }

    /// Returns the timestamp as seconds since Unix epoch (truncates nanoseconds).
    pub fn as_secs(&self) -> u64 {
        self.0 / 1_000_000_000
    }

    /// Creates a timestamp for the current time.
    ///
    /// # Panics
    ///
    /// Panics if the system clock is before Unix epoch (should never happen).
    pub fn now() -> Self {
        let duration = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock is before Unix epoch");
        Self(duration.as_nanos() as u64)
    }

    /// Creates a timestamp ensuring monotonicity: `max(now, last + 1ns)`.
    ///
    /// This guarantees that each timestamp is strictly greater than the previous,
    /// even if the system clock moves backwards or two events occur in the same
    /// nanosecond.
    ///
    /// # Arguments
    ///
    /// * `last` - The previous timestamp, if any. Pass `None` for the first timestamp.
    pub fn now_monotonic(last: Option<Timestamp>) -> Self {
        let now = Self::now();
        match last {
            Some(prev) => {
                // Ensure strictly increasing: at least prev + 1 nanosecond
                if now.0 <= prev.0 {
                    Timestamp(prev.0.saturating_add(1))
                } else {
                    now
                }
            }
            None => now,
        }
    }
}

impl Display for Timestamp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Display as seconds.nanoseconds for readability
        let secs = self.0 / 1_000_000_000;
        let nanos = self.0 % 1_000_000_000;
        write!(f, "{secs}.{nanos:09}")
    }
}

impl Default for Timestamp {
    fn default() -> Self {
        Self::EPOCH
    }
}

impl From<u64> for Timestamp {
    fn from(nanos: u64) -> Self {
        Self(nanos)
    }
}

impl From<Timestamp> for u64 {
    fn from(ts: Timestamp) -> Self {
        ts.0
    }
}

// ============================================================================
// Record Types - Copy (small enum and struct)
// ============================================================================

/// The kind of record stored in the log.
///
/// This enum distinguishes between different record types to enable
/// efficient processing and verification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum RecordKind {
    /// Normal application data record.
    #[default]
    Data,
    /// Periodic verification checkpoint (contains cumulative hash).
    Checkpoint,
    /// Logical deletion marker (data is not physically deleted).
    Tombstone,
}

impl RecordKind {
    /// Returns the single-byte discriminant for serialization.
    pub fn as_byte(&self) -> u8 {
        match self {
            RecordKind::Data => 0,
            RecordKind::Checkpoint => 1,
            RecordKind::Tombstone => 2,
        }
    }

    /// Creates a `RecordKind` from its byte discriminant.
    ///
    /// # Errors
    ///
    /// Returns `None` if the byte is not a valid discriminant.
    pub fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            0 => Some(RecordKind::Data),
            1 => Some(RecordKind::Checkpoint),
            2 => Some(RecordKind::Tombstone),
            _ => None,
        }
    }
}

/// Metadata header for every log record.
///
/// This structure contains all metadata needed to verify and process
/// a log record without reading its payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RecordHeader {
    /// Position in the log (0-indexed).
    pub offset: Offset,
    /// SHA-256 link to the previous record's hash (genesis for first record).
    pub prev_hash: Hash,
    /// When the record was committed (monotonic wall-clock).
    pub timestamp: Timestamp,
    /// Size of the payload in bytes.
    pub payload_len: u32,
    /// Type of record (Data, Checkpoint, Tombstone).
    pub record_kind: RecordKind,
}

impl RecordHeader {
    /// Creates a new record header.
    ///
    /// # Arguments
    ///
    /// * `offset` - Position in the log
    /// * `prev_hash` - Hash of the previous record (or GENESIS for first)
    /// * `timestamp` - When this record was committed
    /// * `payload_len` - Size of the payload in bytes
    /// * `record_kind` - Type of record
    pub fn new(
        offset: Offset,
        prev_hash: Hash,
        timestamp: Timestamp,
        payload_len: u32,
        record_kind: RecordKind,
    ) -> Self {
        Self {
            offset,
            prev_hash,
            timestamp,
            payload_len,
            record_kind,
        }
    }

    /// Returns true if this is the first record in the log.
    pub fn is_genesis(&self) -> bool {
        self.offset == Offset::ZERO && self.prev_hash.is_genesis()
    }
}

// ============================================================================
// Projection Tracking - Copy (small struct for projections)
// ============================================================================

/// Tracks which log entry a projection row was derived from.
///
/// Projections embed this in each row to enable:
/// - Point-in-time queries (find rows at a specific offset)
/// - Verification without walking the hash chain (hash provides direct check)
/// - Audit trails (know exactly which event created/updated a row)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AppliedIndex {
    /// The log offset this row was derived from.
    pub offset: Offset,
    /// The hash at this offset for direct verification.
    pub hash: Hash,
}

impl AppliedIndex {
    /// Creates a new applied index.
    pub fn new(offset: Offset, hash: Hash) -> Self {
        Self { offset, hash }
    }

    /// Creates the initial applied index (before any records).
    pub fn genesis() -> Self {
        Self {
            offset: Offset::ZERO,
            hash: Hash::GENESIS,
        }
    }
}

impl Default for AppliedIndex {
    fn default() -> Self {
        Self::genesis()
    }
}

// ============================================================================
// Checkpoints - Copy (verification anchors in the log)
// ============================================================================

/// A periodic verification checkpoint stored in the log.
///
/// Checkpoints are records IN the log (not separate files), which means:
/// - They are part of the hash chain (tamper-evident)
/// - Checkpoint history is immutable
/// - Single source of truth
///
/// Checkpoints enable efficient verified reads by providing trusted
/// anchor points, reducing verification from O(n) to O(k) where k is
/// the distance to the nearest checkpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Checkpoint {
    /// Log position of this checkpoint record.
    pub offset: Offset,
    /// Cumulative hash of the chain at this point.
    pub chain_hash: Hash,
    /// Total number of records from genesis to this checkpoint.
    pub record_count: u64,
    /// When this checkpoint was created.
    pub created_at: Timestamp,
}

impl Checkpoint {
    /// Creates a new checkpoint.
    ///
    /// # Preconditions
    ///
    /// - `record_count` should equal `offset.as_u64() + 1` (0-indexed offset)
    pub fn new(offset: Offset, chain_hash: Hash, record_count: u64, created_at: Timestamp) -> Self {
        debug_assert_eq!(
            record_count,
            offset.as_u64() + 1,
            "record_count should equal offset + 1"
        );
        Self {
            offset,
            chain_hash,
            record_count,
            created_at,
        }
    }
}

/// Policy for when to create checkpoints.
///
/// Checkpoints bound the worst-case verification cost. The default policy
/// creates a checkpoint every 1000 records and on graceful shutdown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CheckpointPolicy {
    /// Create a checkpoint every N records. Set to 0 to disable.
    pub every_n_records: u64,
    /// Create a checkpoint on graceful shutdown.
    pub on_shutdown: bool,
    /// If true, disable automatic checkpoints (only explicit calls).
    pub explicit_only: bool,
}

impl CheckpointPolicy {
    /// Creates a policy that checkpoints every N records.
    pub fn every(n: u64) -> Self {
        Self {
            every_n_records: n,
            on_shutdown: true,
            explicit_only: false,
        }
    }

    /// Creates a policy that only creates explicit checkpoints.
    pub fn explicit_only() -> Self {
        Self {
            every_n_records: 0,
            on_shutdown: false,
            explicit_only: true,
        }
    }

    /// Returns true if a checkpoint should be created at this offset.
    pub fn should_checkpoint(&self, offset: Offset) -> bool {
        if self.explicit_only {
            return false;
        }
        if self.every_n_records == 0 {
            return false;
        }
        // Checkpoint at offsets that are multiples of every_n_records
        // (offset 999 for every_n_records=1000, etc.)
        (offset.as_u64() + 1) % self.every_n_records == 0
    }
}

impl Default for CheckpointPolicy {
    /// Default policy: checkpoint every 1000 records, on shutdown.
    fn default() -> Self {
        Self {
            every_n_records: 1000,
            on_shutdown: true,
            explicit_only: false,
        }
    }
}

// ============================================================================
// Idempotency - Copy (16-byte identifier for duplicate prevention)
// ============================================================================

/// Length of idempotency IDs in bytes.
pub const IDEMPOTENCY_ID_LENGTH: usize = 16;

/// Unique identifier for duplicate transaction prevention.
///
/// Clients generate an `IdempotencyId` before their first attempt at a
/// transaction. If the transaction needs to be retried (e.g., network
/// timeout), the client reuses the same ID. The server tracks committed
/// IDs to return the same result for duplicate requests.
///
/// Inspired by `FoundationDB`'s idempotency key design.
///
/// # FCIS Pattern
///
/// This type follows the Functional Core / Imperative Shell pattern:
/// - `from_bytes()`: Pure restoration from storage
/// - `from_random_bytes()`: Pure construction from bytes (`pub(crate)`)
/// - `generate()`: Impure shell that invokes CSPRNG
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct IdempotencyId([u8; IDEMPOTENCY_ID_LENGTH]);

impl IdempotencyId {
    // ========================================================================
    // Functional Core (pure, testable)
    // ========================================================================

    /// Pure construction from random bytes.
    ///
    /// Restricted to `pub(crate)` to prevent misuse with weak random sources.
    /// External callers should use `generate()` or `from_bytes()`.
    pub(crate) fn from_random_bytes(bytes: [u8; IDEMPOTENCY_ID_LENGTH]) -> Self {
        debug_assert!(
            bytes.iter().any(|&b| b != 0),
            "idempotency ID bytes are all zeros"
        );
        Self(bytes)
    }

    /// Restoration from stored bytes (pure).
    ///
    /// Use this when loading an `IdempotencyId` from storage or wire protocol.
    pub fn from_bytes(bytes: [u8; IDEMPOTENCY_ID_LENGTH]) -> Self {
        Self(bytes)
    }

    /// Returns the ID as a byte slice.
    pub fn as_bytes(&self) -> &[u8; IDEMPOTENCY_ID_LENGTH] {
        &self.0
    }

    // ========================================================================
    // Imperative Shell (IO boundary)
    // ========================================================================

    /// Generates a new random idempotency ID using the OS CSPRNG.
    ///
    /// # Panics
    ///
    /// Panics if the OS CSPRNG fails, which indicates a catastrophic
    /// system error (e.g., no entropy source available).
    pub fn generate() -> Self {
        let mut bytes = [0u8; IDEMPOTENCY_ID_LENGTH];
        getrandom::fill(&mut bytes).expect("CSPRNG failure is catastrophic");
        Self::from_random_bytes(bytes)
    }
}

impl Debug for IdempotencyId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Show full hex for debugging (IDs are meant to be logged)
        write!(f, "IdempotencyId(")?;
        for byte in &self.0 {
            write!(f, "{byte:02x}")?;
        }
        write!(f, ")")
    }
}

impl Display for IdempotencyId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Full hex representation
        for byte in &self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl From<[u8; IDEMPOTENCY_ID_LENGTH]> for IdempotencyId {
    fn from(bytes: [u8; IDEMPOTENCY_ID_LENGTH]) -> Self {
        Self::from_bytes(bytes)
    }
}

impl From<IdempotencyId> for [u8; IDEMPOTENCY_ID_LENGTH] {
    fn from(id: IdempotencyId) -> Self {
        id.0
    }
}

// ============================================================================
// Recovery Tracking - Copy (generation-based recovery for compliance)
// ============================================================================

/// Monotonically increasing recovery generation.
///
/// Each recovery event creates a new generation. This provides natural
/// audit checkpoints and explicit tracking of system recovery events.
///
/// Inspired by `FoundationDB`'s generation-based recovery tracking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Generation(u64);

impl Generation {
    /// The initial generation (before any recovery).
    pub const INITIAL: Generation = Generation(0);

    /// Creates a generation from a raw value.
    pub fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the generation as a u64.
    pub fn as_u64(&self) -> u64 {
        self.0
    }

    /// Returns the next generation (incremented by 1).
    pub fn next(&self) -> Self {
        Generation(self.0.saturating_add(1))
    }
}

impl Display for Generation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "gen:{}", self.0)
    }
}

impl Default for Generation {
    fn default() -> Self {
        Self::INITIAL
    }
}

impl From<u64> for Generation {
    fn from(value: u64) -> Self {
        Self(value)
    }
}

impl From<Generation> for u64 {
    fn from(generation: Generation) -> Self {
        generation.0
    }
}

/// Reason why a recovery was triggered.
///
/// This is recorded in the recovery log for compliance auditing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RecoveryReason {
    /// Normal node restart (graceful or crash recovery).
    NodeRestart,
    /// Lost quorum and had to recover from remaining replicas.
    QuorumLoss,
    /// Detected data corruption requiring recovery.
    CorruptionDetected,
    /// Operator manually triggered recovery.
    ManualIntervention,
}

impl Display for RecoveryReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RecoveryReason::NodeRestart => write!(f, "node_restart"),
            RecoveryReason::QuorumLoss => write!(f, "quorum_loss"),
            RecoveryReason::CorruptionDetected => write!(f, "corruption_detected"),
            RecoveryReason::ManualIntervention => write!(f, "manual_intervention"),
        }
    }
}

/// Records a recovery event with explicit tracking of any data loss.
///
/// Critical for compliance: auditors can see exactly what happened during
/// recovery, including any mutations that were discarded.
///
/// Inspired by `FoundationDB`'s 9-phase recovery with explicit data loss tracking.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryRecord {
    /// New generation after recovery.
    pub generation: Generation,
    /// Previous generation before recovery.
    pub previous_generation: Generation,
    /// Last known committed offset before recovery.
    pub known_committed: Offset,
    /// Offset we recovered to.
    pub recovery_point: Offset,
    /// Range of discarded prepares (if any) - EXPLICIT LOSS TRACKING.
    ///
    /// If `Some`, this range of offsets contained prepared but uncommitted
    /// mutations that were discarded during recovery. This is the critical
    /// compliance field: it explicitly documents any data loss.
    pub discarded_range: Option<Range<Offset>>,
    /// When recovery occurred.
    pub timestamp: Timestamp,
    /// Why recovery was triggered.
    pub reason: RecoveryReason,
}

impl RecoveryRecord {
    /// Creates a new recovery record.
    ///
    /// # Arguments
    ///
    /// * `generation` - The new generation after recovery
    /// * `previous_generation` - The generation before recovery
    /// * `known_committed` - Last known committed offset
    /// * `recovery_point` - The offset we recovered to
    /// * `discarded_range` - Range of discarded uncommitted prepares, if any
    /// * `timestamp` - When recovery occurred
    /// * `reason` - Why recovery was triggered
    ///
    /// # Preconditions
    ///
    /// - `generation` must be greater than `previous_generation`
    /// - `recovery_point` must be <= `known_committed`
    pub fn new(
        generation: Generation,
        previous_generation: Generation,
        known_committed: Offset,
        recovery_point: Offset,
        discarded_range: Option<Range<Offset>>,
        timestamp: Timestamp,
        reason: RecoveryReason,
    ) -> Self {
        debug_assert!(
            generation > previous_generation,
            "new generation must be greater than previous"
        );
        debug_assert!(
            recovery_point <= known_committed,
            "recovery point cannot exceed known committed"
        );

        Self {
            generation,
            previous_generation,
            known_committed,
            recovery_point,
            discarded_range,
            timestamp,
            reason,
        }
    }

    /// Returns true if any data was lost during this recovery.
    pub fn had_data_loss(&self) -> bool {
        self.discarded_range.is_some()
    }

    /// Returns the number of discarded records, if any.
    pub fn discarded_count(&self) -> u64 {
        self.discarded_range
            .as_ref()
            .map_or(0, |r| r.end.as_u64().saturating_sub(r.start.as_u64()))
    }
}

// ============================================================================
// Stream Name - Clone (contains String, but rarely cloned)
// ============================================================================

/// Human-readable name for a stream.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct StreamName(String);

impl StreamName {
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Display for StreamName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<String> for StreamName {
    fn from(name: String) -> Self {
        Self(name)
    }
}

impl From<&str> for StreamName {
    fn from(name: &str) -> Self {
        Self(name.to_string())
    }
}

impl From<StreamName> for String {
    fn from(value: StreamName) -> Self {
        value.0
    }
}

// ============================================================================
// Data Classification - Copy (simple enum, no heap data)
// ============================================================================

/// Classification of data for compliance purposes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum DataClass {
    /// Protected Health Information - subject to HIPAA restrictions.
    PHI,
    /// Non-PHI data that doesn't contain health information.
    NonPHI,
    /// Data that has been de-identified per HIPAA Safe Harbor.
    Deidentified,
}

// ============================================================================
// Placement - Clone (Region::Custom contains String)
// ============================================================================

/// Placement policy for a stream.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Placement {
    /// Data must remain within the specified region.
    Region(Region),
    /// Data can be replicated globally across all regions.
    Global,
}

/// Geographic region for data placement.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Region {
    /// US East (N. Virginia) - us-east-1
    USEast1,
    /// Asia Pacific (Sydney) - ap-southeast-2
    APSoutheast2,
    /// Custom region identifier
    Custom(String),
}

impl Region {
    pub fn custom(name: impl Into<String>) -> Self {
        Self::Custom(name.into())
    }
}

impl Display for Region {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Region::USEast1 => write!(f, "us-east-1"),
            Region::APSoutheast2 => write!(f, "ap-southeast-2"),
            Region::Custom(custom) => write!(f, "{custom}"),
        }
    }
}

// ============================================================================
// Stream Metadata - Clone (created once per stream, cloned rarely)
// ============================================================================

/// Metadata describing a stream's configuration and current state.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct StreamMetadata {
    pub stream_id: StreamId,
    pub stream_name: StreamName,
    pub data_class: DataClass,
    pub placement: Placement,
    pub current_offset: Offset,
}

impl StreamMetadata {
    /// Creates new stream metadata with offset initialized to 0.
    pub fn new(
        stream_id: StreamId,
        stream_name: StreamName,
        data_class: DataClass,
        placement: Placement,
    ) -> Self {
        Self {
            stream_id,
            stream_name,
            data_class,
            placement,
            current_offset: Offset::default(),
        }
    }
}

// ============================================================================
// Batch Payload - NOT Clone (contains Vec<Bytes>, move only)
// ============================================================================

/// A batch of events to append to a stream.
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatchPayload {
    pub stream_id: StreamId,
    /// The events to append (zero-copy Bytes).
    pub events: Vec<Bytes>,
    /// Expected current offset for optimistic concurrency.
    pub expected_offset: Offset,
}

impl BatchPayload {
    pub fn new(stream_id: StreamId, events: Vec<Bytes>, expected_offset: Offset) -> Self {
        Self {
            stream_id,
            events,
            expected_offset,
        }
    }
}

// ============================================================================
// Audit Actions - Clone (for flexibility in logging)
// ============================================================================

/// Actions recorded in the audit log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuditAction {
    /// A new stream was created.
    StreamCreated {
        stream_id: StreamId,
        stream_name: StreamName,
        data_class: DataClass,
        placement: Placement,
    },
    /// Events were appended to a stream.
    EventsAppended {
        stream_id: StreamId,
        count: u32,
        from_offset: Offset,
    },
}

// ============================================================================
// Event Persistence - Trait for durable event log writes
// ============================================================================

/// Abstraction for persisting events to the durable event log.
///
/// This trait is the bridge between the projection layer and the
/// `VerityDB` replication system. Implementations must block until
/// persistence is confirmed.
///
/// # Healthcare Compliance
///
/// This is the critical path for HIPAA compliance. The implementation must:
/// - **Block until VSR consensus** completes (quorum durability)
/// - **Return `Err`** if consensus fails (triggers rollback)
/// - **Never return `Ok`** unless events are durably stored
///
/// # Implementation Notes
///
/// The implementor (typically `Runtime`) must handle the sync→async bridge:
///
/// ```ignore
/// impl EventPersister for RuntimeHandle {
///     fn persist_blocking(&self, stream_id: StreamId, events: Vec<Bytes>) -> Result<Offset, PersistError> {
///         // Bridge sync callback to async runtime
///         tokio::task::block_in_place(|| {
///             tokio::runtime::Handle::current().block_on(async {
///                 self.inner.append(stream_id, events).await
///             })
///         })
///         .map_err(|e| {
///             tracing::error!(error = %e, "VSR persistence failed");
///             PersistError::ConsensusFailed
///         })
///     }
/// }
/// ```
///
/// # Why `Vec<Bytes>` instead of typed events?
///
/// Events are serialized before reaching this trait. This keeps `vdb-types`
/// decoupled from domain-specific event schemas.
pub trait EventPersister: Send + Sync + Debug {
    /// Persist a batch of serialized events to the durable event log.
    ///
    /// This method **blocks** until VSR consensus confirms the events are
    /// durably stored on a quorum of nodes.
    ///
    /// # Arguments
    ///
    /// * `stream_id` - The stream to append events to
    /// * `events` - Serialized events
    ///
    /// # Returns
    ///
    /// * `Ok(offset)` - Events persisted, returns the new stream offset
    /// * `Err(PersistError)` - Persistence failed, caller should rollback
    ///
    /// # Errors
    ///
    /// * [`PersistError::ConsensusFailed`] - VSR quorum unavailable after retries
    /// * [`PersistError::StorageError`] - Disk I/O or serialization failure
    /// * [`PersistError::ShuttingDown`] - System is terminating
    fn persist_blocking(
        &self,
        stream_id: StreamId,
        events: Vec<Bytes>,
    ) -> Result<Offset, PersistError>;
}

/// Error returned when event persistence fails.
///
/// The hook uses this to decide whether to rollback the transaction.
/// Specific underlying errors are logged by the implementation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PersistError {
    /// VSR consensus failed after retries (quorum unavailable)
    ConsensusFailed,
    /// Storage I/O error
    StorageError,
    /// System is shutting down
    ShuttingDown,
}

impl std::fmt::Display for PersistError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ConsensusFailed => write!(f, "consensus failed after retries"),
            Self::StorageError => write!(f, "storage I/O error"),
            Self::ShuttingDown => write!(f, "system is shutting down"),
        }
    }
}

impl std::error::Error for PersistError {}

#[cfg(test)]
mod tests;
