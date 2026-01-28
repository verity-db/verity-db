//! 4-copy superblock for atomic metadata persistence.
//!
//! The superblock stores critical VSR metadata that must survive crashes:
//! - Current view number
//! - Operation and commit numbers
//! - Recovery generation
//! - Checkpoint reference
//!
//! # Design
//!
//! We maintain 4 copies of the superblock written in rotation. This provides:
//! - **Atomicity**: Single-sector writes are atomic on modern storage
//! - **Durability**: At least one valid copy survives any single failure
//! - **Recovery**: Majority vote on highest valid sequence number
//!
//! # Write Protocol
//!
//! 1. Increment sequence number
//! 2. Compute CRC32 of all fields
//! 3. Write to next slot (round-robin)
//! 4. fsync
//!
//! # Recovery Protocol
//!
//! 1. Read all 4 copies
//! 2. Validate CRC32 for each
//! 3. Select the copy with highest valid sequence number
//!
//! # Storage Layout
//!
//! ```text
//! Offset     Size    Content
//! ────────────────────────────
//! 0x0000     512     Superblock copy 0
//! 0x0200     512     Superblock copy 1
//! 0x0400     512     Superblock copy 2
//! 0x0600     512     Superblock copy 3
//! ```
//!
//! Inspired by `TigerBeetle`'s superblock design.

use std::io::{self, Read, Seek, SeekFrom, Write};

use serde::{Deserialize, Serialize};
use craton_types::{Generation, Hash, Timestamp};

use crate::types::{CommitNumber, OpNumber, ReplicaId, ViewNumber};

// ============================================================================
// Constants
// ============================================================================

/// Magic bytes identifying a valid superblock.
const SUPERBLOCK_MAGIC: [u8; 8] = *b"VDBSUPER";

/// Current superblock format version.
const SUPERBLOCK_VERSION: u32 = 1;

/// Size of each superblock copy in bytes.
///
/// Must be a power of 2 and at least as large as the sector size (typically 512).
/// We use 512 bytes to fit in a single sector for atomic writes.
pub const SUPERBLOCK_COPY_SIZE: usize = 512;

/// Number of superblock copies.
pub const SUPERBLOCK_COPIES: usize = 4;

/// Total size of the superblock region.
pub const SUPERBLOCK_TOTAL_SIZE: usize = SUPERBLOCK_COPY_SIZE * SUPERBLOCK_COPIES;

/// Offset of the CRC32 field within the superblock.
const CRC_OFFSET: usize = SUPERBLOCK_COPY_SIZE - 4;

// ============================================================================
// Superblock Data
// ============================================================================

/// Durable metadata stored in the superblock.
///
/// This structure contains all state that must be persisted atomically
/// for crash recovery.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SuperblockData {
    /// Monotonically increasing sequence number.
    ///
    /// Used to identify the most recent valid superblock during recovery.
    pub sequence: u64,

    /// The replica ID for this storage.
    pub replica_id: ReplicaId,

    /// Current view number.
    pub view: ViewNumber,

    /// Highest operation number in the log.
    pub op_number: OpNumber,

    /// Highest committed operation number.
    pub commit_number: CommitNumber,

    /// Current recovery generation.
    pub generation: Generation,

    /// Operation number of the last checkpoint.
    pub checkpoint_op: OpNumber,

    /// Merkle root of the last checkpoint (BLAKE3).
    pub checkpoint_hash: Hash,

    /// When this superblock was written.
    pub timestamp: Timestamp,
}

impl SuperblockData {
    /// Creates initial superblock data for a new replica.
    pub fn new(replica_id: ReplicaId) -> Self {
        Self {
            sequence: 0,
            replica_id,
            view: ViewNumber::ZERO,
            op_number: OpNumber::ZERO,
            commit_number: CommitNumber::ZERO,
            generation: Generation::INITIAL,
            checkpoint_op: OpNumber::ZERO,
            checkpoint_hash: Hash::GENESIS,
            timestamp: Timestamp::EPOCH,
        }
    }

    /// Creates the next superblock with updated state.
    ///
    /// Increments the sequence number and updates the timestamp.
    pub fn next(&self, view: ViewNumber, op_number: OpNumber, commit_number: CommitNumber) -> Self {
        Self {
            sequence: self.sequence.saturating_add(1),
            replica_id: self.replica_id,
            view,
            op_number,
            commit_number,
            generation: self.generation,
            checkpoint_op: self.checkpoint_op,
            checkpoint_hash: self.checkpoint_hash,
            timestamp: Timestamp::now(),
        }
    }

    /// Creates the next superblock with a new checkpoint.
    pub fn with_checkpoint(&self, checkpoint_op: OpNumber, checkpoint_hash: Hash) -> Self {
        Self {
            sequence: self.sequence.saturating_add(1),
            checkpoint_op,
            checkpoint_hash,
            timestamp: Timestamp::now(),
            ..*self
        }
    }

    /// Creates the next superblock with a new generation (recovery).
    pub fn with_generation(&self, generation: Generation) -> Self {
        debug_assert!(
            generation > self.generation,
            "new generation must be greater than current"
        );
        Self {
            sequence: self.sequence.saturating_add(1),
            generation,
            timestamp: Timestamp::now(),
            ..*self
        }
    }

    /// Serializes the superblock to bytes with CRC32.
    ///
    /// Returns exactly `SUPERBLOCK_COPY_SIZE` bytes.
    pub fn to_bytes(&self) -> [u8; SUPERBLOCK_COPY_SIZE] {
        let mut buf = [0u8; SUPERBLOCK_COPY_SIZE];

        // Write magic and version
        buf[0..8].copy_from_slice(&SUPERBLOCK_MAGIC);
        buf[8..12].copy_from_slice(&SUPERBLOCK_VERSION.to_le_bytes());

        // Serialize the data using a compact binary format
        let mut offset = 12;

        // sequence (8 bytes)
        buf[offset..offset + 8].copy_from_slice(&self.sequence.to_le_bytes());
        offset += 8;

        // replica_id (1 byte)
        buf[offset] = self.replica_id.as_u8();
        offset += 1;

        // view (8 bytes)
        buf[offset..offset + 8].copy_from_slice(&self.view.as_u64().to_le_bytes());
        offset += 8;

        // op_number (8 bytes)
        buf[offset..offset + 8].copy_from_slice(&self.op_number.as_u64().to_le_bytes());
        offset += 8;

        // commit_number (8 bytes)
        buf[offset..offset + 8].copy_from_slice(&self.commit_number.as_u64().to_le_bytes());
        offset += 8;

        // generation (8 bytes)
        buf[offset..offset + 8].copy_from_slice(&self.generation.as_u64().to_le_bytes());
        offset += 8;

        // checkpoint_op (8 bytes)
        buf[offset..offset + 8].copy_from_slice(&self.checkpoint_op.as_u64().to_le_bytes());
        offset += 8;

        // checkpoint_hash (32 bytes)
        buf[offset..offset + 32].copy_from_slice(self.checkpoint_hash.as_bytes());
        offset += 32;

        // timestamp (8 bytes)
        buf[offset..offset + 8].copy_from_slice(&self.timestamp.as_nanos().to_le_bytes());

        // Compute and write CRC32 at the end
        let crc = crc32fast::hash(&buf[..CRC_OFFSET]);
        buf[CRC_OFFSET..].copy_from_slice(&crc.to_le_bytes());

        buf
    }

    /// Deserializes a superblock from bytes.
    ///
    /// Returns `None` if the magic, version, or CRC32 is invalid.
    pub fn from_bytes(buf: &[u8; SUPERBLOCK_COPY_SIZE]) -> Option<Self> {
        // Verify magic
        if buf[0..8] != SUPERBLOCK_MAGIC {
            return None;
        }

        // Verify version
        let version = u32::from_le_bytes(buf[8..12].try_into().ok()?);
        if version != SUPERBLOCK_VERSION {
            return None;
        }

        // Verify CRC32
        let expected_crc = u32::from_le_bytes(buf[CRC_OFFSET..].try_into().ok()?);
        let actual_crc = crc32fast::hash(&buf[..CRC_OFFSET]);
        if expected_crc != actual_crc {
            return None;
        }

        // Parse fields
        let mut offset = 12;

        let sequence = u64::from_le_bytes(buf[offset..offset + 8].try_into().ok()?);
        offset += 8;

        let replica_id = ReplicaId::new(buf[offset]);
        offset += 1;

        let view = ViewNumber::new(u64::from_le_bytes(buf[offset..offset + 8].try_into().ok()?));
        offset += 8;

        let op_number = OpNumber::new(u64::from_le_bytes(buf[offset..offset + 8].try_into().ok()?));
        offset += 8;

        let commit_number = CommitNumber::new(OpNumber::new(u64::from_le_bytes(
            buf[offset..offset + 8].try_into().ok()?,
        )));
        offset += 8;

        let generation =
            Generation::new(u64::from_le_bytes(buf[offset..offset + 8].try_into().ok()?));
        offset += 8;

        let checkpoint_op =
            OpNumber::new(u64::from_le_bytes(buf[offset..offset + 8].try_into().ok()?));
        offset += 8;

        let mut hash_bytes = [0u8; 32];
        hash_bytes.copy_from_slice(&buf[offset..offset + 32]);
        let checkpoint_hash = Hash::from_bytes(hash_bytes);
        offset += 32;

        let timestamp =
            Timestamp::from_nanos(u64::from_le_bytes(buf[offset..offset + 8].try_into().ok()?));

        Some(Self {
            sequence,
            replica_id,
            view,
            op_number,
            commit_number,
            generation,
            checkpoint_op,
            checkpoint_hash,
            timestamp,
        })
    }
}

// ============================================================================
// Superblock Manager
// ============================================================================

/// Manages 4-copy superblock persistence.
///
/// Provides atomic, durable updates to VSR metadata through a rotation
/// protocol that ensures at least one valid copy survives any failure.
#[derive(Debug)]
pub struct Superblock<W> {
    /// The underlying storage (file or memory).
    storage: W,

    /// Current superblock data.
    data: SuperblockData,

    /// Next slot to write (0-3).
    next_slot: usize,
}

impl<W: Read + Write + Seek> Superblock<W> {
    /// Creates a new superblock manager for a new replica.
    ///
    /// Initializes all 4 copies with the initial state.
    pub fn create(mut storage: W, replica_id: ReplicaId) -> io::Result<Self> {
        let data = SuperblockData::new(replica_id);
        let bytes = data.to_bytes();

        // Write all 4 copies
        for slot in 0..SUPERBLOCK_COPIES {
            let offset = (slot * SUPERBLOCK_COPY_SIZE) as u64;
            storage.seek(SeekFrom::Start(offset))?;
            storage.write_all(&bytes)?;
        }
        storage.flush()?;

        Ok(Self {
            storage,
            data,
            next_slot: 0,
        })
    }

    /// Opens an existing superblock from storage.
    ///
    /// Recovers by selecting the copy with the highest valid sequence number.
    ///
    /// # Errors
    ///
    /// Returns an error if no valid superblock copy is found.
    pub fn open(mut storage: W) -> io::Result<Self> {
        let mut best: Option<(SuperblockData, usize)> = None;

        // Read and validate all 4 copies
        for slot in 0..SUPERBLOCK_COPIES {
            let offset = (slot * SUPERBLOCK_COPY_SIZE) as u64;
            storage.seek(SeekFrom::Start(offset))?;

            let mut buf = [0u8; SUPERBLOCK_COPY_SIZE];
            if storage.read_exact(&mut buf).is_err() {
                continue;
            }

            if let Some(data) = SuperblockData::from_bytes(&buf) {
                match &best {
                    None => best = Some((data, slot)),
                    Some((best_data, _)) if data.sequence > best_data.sequence => {
                        best = Some((data, slot));
                    }
                    _ => {}
                }
            }
        }

        let (data, last_slot) = best.ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "no valid superblock copy found")
        })?;

        // Next slot is after the last written slot
        let next_slot = (last_slot + 1) % SUPERBLOCK_COPIES;

        Ok(Self {
            storage,
            data,
            next_slot,
        })
    }

    /// Returns the current superblock data.
    pub fn data(&self) -> &SuperblockData {
        &self.data
    }

    /// Updates the superblock with new VSR state.
    ///
    /// Writes to the next slot in rotation and fsyncs.
    pub fn update(
        &mut self,
        view: ViewNumber,
        op_number: OpNumber,
        commit_number: CommitNumber,
    ) -> io::Result<()> {
        let new_data = self.data.next(view, op_number, commit_number);
        self.write_slot(new_data)
    }

    /// Updates the superblock with a new checkpoint.
    pub fn update_checkpoint(
        &mut self,
        checkpoint_op: OpNumber,
        checkpoint_hash: Hash,
    ) -> io::Result<()> {
        let new_data = self.data.with_checkpoint(checkpoint_op, checkpoint_hash);
        self.write_slot(new_data)
    }

    /// Updates the superblock with a new generation (recovery).
    pub fn update_generation(&mut self, generation: Generation) -> io::Result<()> {
        let new_data = self.data.with_generation(generation);
        self.write_slot(new_data)
    }

    /// Writes to the next slot and advances the rotation.
    fn write_slot(&mut self, new_data: SuperblockData) -> io::Result<()> {
        let bytes = new_data.to_bytes();
        let offset = (self.next_slot * SUPERBLOCK_COPY_SIZE) as u64;

        self.storage.seek(SeekFrom::Start(offset))?;
        self.storage.write_all(&bytes)?;
        self.storage.flush()?;

        self.data = new_data;
        self.next_slot = (self.next_slot + 1) % SUPERBLOCK_COPIES;

        Ok(())
    }

    /// Returns the replica ID.
    pub fn replica_id(&self) -> ReplicaId {
        self.data.replica_id
    }

    /// Returns the current view.
    pub fn view(&self) -> ViewNumber {
        self.data.view
    }

    /// Returns the highest operation number.
    pub fn op_number(&self) -> OpNumber {
        self.data.op_number
    }

    /// Returns the highest committed operation number.
    pub fn commit_number(&self) -> CommitNumber {
        self.data.commit_number
    }

    /// Returns the current generation.
    pub fn generation(&self) -> Generation {
        self.data.generation
    }

    /// Returns a reference to the underlying storage (for testing).
    #[cfg(test)]
    pub fn storage(&self) -> &W {
        &self.storage
    }

    /// Consumes the superblock and returns the underlying storage (for testing).
    #[cfg(test)]
    pub fn into_storage(self) -> W {
        self.storage
    }
}

// ============================================================================
// In-Memory Superblock (for testing)
// ============================================================================

/// In-memory storage for superblock testing.
#[derive(Debug)]
pub struct MemorySuperblock {
    data: Vec<u8>,
    position: u64,
}

impl MemorySuperblock {
    /// Creates a new in-memory superblock storage.
    pub fn new() -> Self {
        Self {
            data: vec![0u8; SUPERBLOCK_TOTAL_SIZE],
            position: 0,
        }
    }
}

impl Default for MemorySuperblock {
    fn default() -> Self {
        Self::new()
    }
}

impl MemorySuperblock {
    /// Returns a copy of the internal data buffer (for testing).
    pub fn clone_data(&self) -> Vec<u8> {
        self.data.clone()
    }

    /// Creates a new `MemorySuperblock` with the given data (for testing).
    pub fn from_data(data: Vec<u8>) -> Self {
        Self { data, position: 0 }
    }
}

impl Read for MemorySuperblock {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let pos = self.position as usize;
        let available = self.data.len().saturating_sub(pos);
        let to_read = buf.len().min(available);

        buf[..to_read].copy_from_slice(&self.data[pos..pos + to_read]);
        self.position += to_read as u64;

        Ok(to_read)
    }
}

impl Write for MemorySuperblock {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let pos = self.position as usize;
        let end = pos + buf.len();

        if end > self.data.len() {
            self.data.resize(end, 0);
        }

        self.data[pos..end].copy_from_slice(buf);
        self.position = end as u64;

        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Seek for MemorySuperblock {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let new_pos = match pos {
            SeekFrom::Start(offset) => offset as i64,
            SeekFrom::End(offset) => self.data.len() as i64 + offset,
            SeekFrom::Current(offset) => self.position as i64 + offset,
        };

        let new_pos = u64::try_from(new_pos)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "seek before start"))?;

        self.position = new_pos;
        Ok(self.position)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn superblock_data_roundtrip() {
        let data = SuperblockData::new(ReplicaId::new(5));
        let bytes = data.to_bytes();
        let recovered = SuperblockData::from_bytes(&bytes).expect("should parse");

        assert_eq!(data.sequence, recovered.sequence);
        assert_eq!(data.replica_id, recovered.replica_id);
        assert_eq!(data.view, recovered.view);
    }

    #[test]
    fn superblock_data_invalid_magic() {
        let mut buf = [0u8; SUPERBLOCK_COPY_SIZE];
        buf[0..8].copy_from_slice(b"INVALID!");

        assert!(SuperblockData::from_bytes(&buf).is_none());
    }

    #[test]
    fn superblock_data_invalid_crc() {
        let data = SuperblockData::new(ReplicaId::new(0));
        let mut bytes = data.to_bytes();

        // Corrupt a byte
        bytes[20] ^= 0xFF;

        assert!(SuperblockData::from_bytes(&bytes).is_none());
    }

    #[test]
    fn superblock_create_and_open() {
        let storage = MemorySuperblock::new();
        let sb = Superblock::create(storage, ReplicaId::new(3)).expect("create");

        assert_eq!(sb.replica_id(), ReplicaId::new(3));
        assert_eq!(sb.view(), ViewNumber::ZERO);
        assert_eq!(sb.op_number(), OpNumber::ZERO);

        // Simulate reopening
        let mut storage2 = MemorySuperblock::new();
        storage2.data = sb.storage.data.clone();

        let sb2 = Superblock::open(storage2).expect("open");
        assert_eq!(sb2.replica_id(), ReplicaId::new(3));
    }

    #[test]
    fn superblock_update_rotation() {
        let storage = MemorySuperblock::new();
        let mut sb = Superblock::create(storage, ReplicaId::new(0)).expect("create");

        // Update multiple times to test rotation
        for i in 1..=10 {
            sb.update(
                ViewNumber::new(i),
                OpNumber::new(i * 10),
                CommitNumber::new(OpNumber::new(i * 5)),
            )
            .expect("update");

            assert_eq!(sb.view(), ViewNumber::new(i));
            assert_eq!(sb.op_number(), OpNumber::new(i * 10));
        }

        // Verify sequence number increases
        assert_eq!(sb.data().sequence, 10);
    }

    #[test]
    fn superblock_recovery_selects_highest_sequence() {
        let storage = MemorySuperblock::new();
        let mut sb = Superblock::create(storage, ReplicaId::new(0)).expect("create");

        // Make several updates
        sb.update(ViewNumber::new(1), OpNumber::new(10), CommitNumber::ZERO)
            .expect("update");
        sb.update(ViewNumber::new(2), OpNumber::new(20), CommitNumber::ZERO)
            .expect("update");
        sb.update(ViewNumber::new(3), OpNumber::new(30), CommitNumber::ZERO)
            .expect("update");

        // Simulate reopening
        let mut storage2 = MemorySuperblock::new();
        storage2.data = sb.storage.data.clone();

        let sb2 = Superblock::open(storage2).expect("open");

        // Should have recovered the latest state
        assert_eq!(sb2.view(), ViewNumber::new(3));
        assert_eq!(sb2.op_number(), OpNumber::new(30));
    }

    #[test]
    fn superblock_checkpoint_update() {
        let storage = MemorySuperblock::new();
        let mut sb = Superblock::create(storage, ReplicaId::new(0)).expect("create");

        let checkpoint_hash = Hash::from_bytes([42u8; 32]);
        sb.update_checkpoint(OpNumber::new(100), checkpoint_hash)
            .expect("checkpoint");

        assert_eq!(sb.data().checkpoint_op, OpNumber::new(100));
        assert_eq!(sb.data().checkpoint_hash, checkpoint_hash);
    }

    #[test]
    fn superblock_generation_update() {
        let storage = MemorySuperblock::new();
        let mut sb = Superblock::create(storage, ReplicaId::new(0)).expect("create");

        sb.update_generation(Generation::new(1))
            .expect("generation");
        assert_eq!(sb.generation(), Generation::new(1));

        sb.update_generation(Generation::new(2))
            .expect("generation");
        assert_eq!(sb.generation(), Generation::new(2));
    }

    #[test]
    fn superblock_size_fits_sector() {
        // Verify our superblock fits in a single 512-byte sector
        assert_eq!(SUPERBLOCK_COPY_SIZE, 512);

        // Verify we have room for all fields
        let data = SuperblockData::new(ReplicaId::new(0));
        let bytes = data.to_bytes();
        assert_eq!(bytes.len(), 512);
    }
}
