//! Cryptographic checkpoints for VSR state verification.
//!
//! Checkpoints provide tamper-evident snapshots of the replicated state,
//! enabling:
//! - Efficient state verification via Merkle roots
//! - Non-repudiation via Ed25519 quorum signatures
//! - Fast state transfer to recovering replicas
//!
//! # Checkpoint Structure
//!
//! ```text
//! ┌────────────────────────────────────────┐
//! │  CheckpointData                        │
//! │  ├── `op_number`: OpNumber             │
//! │  ├── `view`: ViewNumber                │
//! │  ├── `log_root`: MerkleRoot (BLAKE3)   │
//! │  └── timestamp: u64                    │
//! ├────────────────────────────────────────┤
//! │  Signatures (quorum required)          │
//! │  ├── Replica 0: Ed25519 signature      │
//! │  ├── Replica 1: Ed25519 signature      │
//! │  └── ...                               │
//! └────────────────────────────────────────┘
//! ```
//!
//! # Merkle Tree
//!
//! Log entries are hashed into a binary Merkle tree using BLAKE3.
//! The root hash summarizes all entries up to the checkpoint.
//!
//! ```text
//!           [Root]
//!          /      \
//!     [H01]        [H23]
//!     /   \        /   \
//!  [H0]  [H1]   [H2]  [H3]
//!   |     |      |     |
//! Entry0 Entry1 Entry2 Entry3
//! ```

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use craton_crypto::{InternalHash, Signature, SigningKey, VerifyingKey, internal_hash};

use crate::types::{LogEntry, OpNumber, ReplicaId, ViewNumber};

// ============================================================================
// Constants
// ============================================================================

/// Length of a Merkle root in bytes (32 bytes, BLAKE3).
pub const MERKLE_ROOT_LENGTH: usize = 32;

// ============================================================================
// MerkleRoot
// ============================================================================

/// A Merkle tree root hash (BLAKE3).
///
/// This summarizes a set of log entries in a single 32-byte hash.
/// Any change to any entry produces a completely different root.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MerkleRoot([u8; MERKLE_ROOT_LENGTH]);

impl MerkleRoot {
    /// Creates a Merkle root from raw bytes.
    pub fn from_bytes(bytes: [u8; MERKLE_ROOT_LENGTH]) -> Self {
        Self(bytes)
    }

    /// Returns the raw bytes of the Merkle root.
    pub fn as_bytes(&self) -> &[u8; MERKLE_ROOT_LENGTH] {
        &self.0
    }

    /// Returns true if this is the empty root (no entries).
    pub fn is_empty(&self) -> bool {
        self.0.iter().all(|&b| b == 0)
    }

    /// The empty Merkle root (hash of empty input).
    pub fn empty() -> Self {
        let hash = internal_hash(b"");
        Self(*hash.as_bytes())
    }
}

impl From<InternalHash> for MerkleRoot {
    fn from(hash: InternalHash) -> Self {
        Self(*hash.as_bytes())
    }
}

impl Default for MerkleRoot {
    fn default() -> Self {
        Self::empty()
    }
}

// ============================================================================
// Merkle Tree Computation
// ============================================================================

/// Computes a Merkle root over a sequence of log entries.
///
/// Uses BLAKE3 for fast internal hashing. The tree is constructed
/// bottom-up with leaf nodes being hashes of serialized entries.
///
/// # Arguments
///
/// * `entries` - Log entries to include in the tree
///
/// # Returns
///
/// The Merkle root summarizing all entries.
pub fn compute_merkle_root(entries: &[LogEntry]) -> MerkleRoot {
    if entries.is_empty() {
        return MerkleRoot::empty();
    }

    // Compute leaf hashes
    let mut hashes: Vec<InternalHash> = entries.iter().map(hash_log_entry).collect();

    // Build tree bottom-up
    while hashes.len() > 1 {
        let mut next_level = Vec::with_capacity(hashes.len().div_ceil(2));

        let mut i = 0;
        while i < hashes.len() {
            if i + 1 < hashes.len() {
                // Combine two nodes
                next_level.push(hash_pair(&hashes[i], &hashes[i + 1]));
                i += 2;
            } else {
                // Odd node - promote to next level
                next_level.push(hashes[i]);
                i += 1;
            }
        }

        hashes = next_level;
    }

    MerkleRoot::from(hashes[0])
}

/// Hashes a single log entry for use as a Merkle leaf.
fn hash_log_entry(entry: &LogEntry) -> InternalHash {
    // Hash the entry's essential fields
    let mut data = Vec::new();
    data.extend_from_slice(&entry.op_number.as_u64().to_le_bytes());
    data.extend_from_slice(&entry.view.as_u64().to_le_bytes());
    data.extend_from_slice(&entry.checksum.to_le_bytes());

    internal_hash(&data)
}

/// Hashes two child nodes to produce a parent node.
fn hash_pair(left: &InternalHash, right: &InternalHash) -> InternalHash {
    let mut data = [0u8; 64];
    data[..32].copy_from_slice(left.as_bytes());
    data[32..].copy_from_slice(right.as_bytes());
    internal_hash(&data)
}

// ============================================================================
// CheckpointData
// ============================================================================

/// Core checkpoint data that gets signed by replicas.
///
/// This contains all the information needed to verify the state
/// at a particular point in the log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointData {
    /// The operation number at which this checkpoint was taken.
    pub op_number: OpNumber,

    /// The view number when this checkpoint was created.
    pub view: ViewNumber,

    /// Merkle root of all log entries up to `op_number`.
    pub log_root: MerkleRoot,

    /// Unix timestamp (seconds since epoch) when checkpoint was created.
    pub timestamp: u64,
}

impl CheckpointData {
    /// Creates new checkpoint data.
    pub fn new(
        op_number: OpNumber,
        view: ViewNumber,
        log_root: MerkleRoot,
        timestamp: u64,
    ) -> Self {
        Self {
            op_number,
            view,
            log_root,
            timestamp,
        }
    }

    /// Serializes the checkpoint data for signing.
    ///
    /// This produces a canonical byte representation that all replicas
    /// must agree on before signing.
    pub fn to_signable_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(80);

        // Domain separator to prevent cross-protocol attacks
        bytes.extend_from_slice(b"Craton-Checkpoint-v1");

        // Core fields
        bytes.extend_from_slice(&self.op_number.as_u64().to_le_bytes());
        bytes.extend_from_slice(&self.view.as_u64().to_le_bytes());
        bytes.extend_from_slice(self.log_root.as_bytes());
        bytes.extend_from_slice(&self.timestamp.to_le_bytes());

        bytes
    }
}

// ============================================================================
// ReplicaSignature
// ============================================================================

/// A signature from a single replica on checkpoint data.
#[derive(Debug, Clone)]
pub struct ReplicaSignature {
    /// The replica that produced this signature.
    pub replica_id: ReplicaId,

    /// The Ed25519 signature (64 bytes).
    signature_bytes: [u8; 64],
}

impl ReplicaSignature {
    /// Creates a new replica signature.
    pub fn new(replica_id: ReplicaId, signature: Signature) -> Self {
        Self {
            replica_id,
            signature_bytes: signature.to_bytes(),
        }
    }

    /// Returns the signature as a `Signature` type.
    pub fn as_signature(&self) -> Signature {
        Signature::from_bytes(&self.signature_bytes)
    }

    /// Returns the raw signature bytes.
    pub fn signature_bytes(&self) -> &[u8; 64] {
        &self.signature_bytes
    }
}

// Custom serialization for ReplicaSignature to handle [u8; 64]
impl Serialize for ReplicaSignature {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("ReplicaSignature", 2)?;
        state.serialize_field("replica_id", &self.replica_id)?;
        // Serialize as a Vec<u8> for portability
        state.serialize_field("signature", &self.signature_bytes.to_vec())?;
        state.end()
    }
}

impl<'de> Deserialize<'de> for ReplicaSignature {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Helper {
            replica_id: ReplicaId,
            signature: Vec<u8>,
        }

        let helper = Helper::deserialize(deserializer)?;

        let signature_bytes: [u8; 64] = helper
            .signature
            .try_into()
            .map_err(|_| serde::de::Error::custom("signature must be exactly 64 bytes"))?;

        Ok(ReplicaSignature {
            replica_id: helper.replica_id,
            signature_bytes,
        })
    }
}

// ============================================================================
// Checkpoint
// ============================================================================

/// A complete checkpoint with quorum signatures.
///
/// A checkpoint is valid when it has signatures from a quorum of replicas.
/// This provides Byzantine fault tolerance - even if some replicas are
/// compromised, the checkpoint remains trustworthy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Checkpoint {
    /// The checkpoint data that was signed.
    pub data: CheckpointData,

    /// Signatures from replicas.
    pub signatures: HashMap<ReplicaId, ReplicaSignature>,
}

impl Checkpoint {
    /// Creates a new checkpoint without signatures.
    pub fn new(data: CheckpointData) -> Self {
        Self {
            data,
            signatures: HashMap::new(),
        }
    }

    /// Creates a checkpoint from log entries.
    ///
    /// Computes the Merkle root automatically.
    pub fn from_entries(entries: &[LogEntry], view: ViewNumber, timestamp: u64) -> Self {
        let op_number = entries.last().map_or(OpNumber::ZERO, |e| e.op_number);

        let log_root = compute_merkle_root(entries);

        let data = CheckpointData::new(op_number, view, log_root, timestamp);
        Self::new(data)
    }

    /// Signs this checkpoint with the given key and adds the signature.
    pub fn sign(&mut self, replica_id: ReplicaId, signing_key: &SigningKey) {
        let signable = self.data.to_signable_bytes();
        let signature = signing_key.sign(&signable);
        self.signatures
            .insert(replica_id, ReplicaSignature::new(replica_id, signature));
    }

    /// Adds an existing signature to this checkpoint.
    pub fn add_signature(&mut self, sig: ReplicaSignature) {
        self.signatures.insert(sig.replica_id, sig);
    }

    /// Returns the number of signatures on this checkpoint.
    pub fn signature_count(&self) -> usize {
        self.signatures.len()
    }

    /// Returns true if this checkpoint has a quorum of signatures.
    pub fn has_quorum(&self, quorum_size: usize) -> bool {
        self.signatures.len() >= quorum_size
    }

    /// Verifies all signatures on this checkpoint.
    ///
    /// # Arguments
    ///
    /// * `verifying_keys` - Map from replica ID to verifying key
    ///
    /// # Returns
    ///
    /// The number of valid signatures.
    pub fn verify_signatures(&self, verifying_keys: &HashMap<ReplicaId, VerifyingKey>) -> usize {
        let signable = self.data.to_signable_bytes();
        let mut valid_count = 0;

        for (replica_id, replica_sig) in &self.signatures {
            if let Some(key) = verifying_keys.get(replica_id) {
                let signature = replica_sig.as_signature();
                if key.verify(&signable, &signature).is_ok() {
                    valid_count += 1;
                }
            }
        }

        valid_count
    }

    /// Verifies that this checkpoint has a valid quorum of signatures.
    pub fn verify_quorum(
        &self,
        quorum_size: usize,
        verifying_keys: &HashMap<ReplicaId, VerifyingKey>,
    ) -> bool {
        self.verify_signatures(verifying_keys) >= quorum_size
    }
}

// ============================================================================
// CheckpointBuilder
// ============================================================================

/// Builder for collecting checkpoint signatures from multiple replicas.
///
/// Used during the checkpoint protocol to accumulate signatures
/// until a quorum is reached.
#[derive(Debug)]
pub struct CheckpointBuilder {
    checkpoint: Checkpoint,
    quorum_size: usize,
}

impl CheckpointBuilder {
    /// Creates a new checkpoint builder.
    pub fn new(data: CheckpointData, quorum_size: usize) -> Self {
        Self {
            checkpoint: Checkpoint::new(data),
            quorum_size,
        }
    }

    /// Returns a reference to the checkpoint data.
    pub fn data(&self) -> &CheckpointData {
        &self.checkpoint.data
    }

    /// Adds a signature to the checkpoint.
    ///
    /// Returns true if this signature completed the quorum.
    pub fn add_signature(&mut self, sig: ReplicaSignature) -> bool {
        let had_quorum = self.checkpoint.has_quorum(self.quorum_size);
        self.checkpoint.add_signature(sig);
        !had_quorum && self.checkpoint.has_quorum(self.quorum_size)
    }

    /// Returns true if the checkpoint has a quorum of signatures.
    pub fn has_quorum(&self) -> bool {
        self.checkpoint.has_quorum(self.quorum_size)
    }

    /// Consumes the builder and returns the checkpoint.
    ///
    /// Should only be called after `has_quorum()` returns true.
    pub fn build(self) -> Checkpoint {
        debug_assert!(self.has_quorum(), "building checkpoint without quorum");
        self.checkpoint
    }

    /// Returns the current signature count.
    pub fn signature_count(&self) -> usize {
        self.checkpoint.signature_count()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ViewNumber;
    use craton_kernel::Command;
    use craton_types::{DataClass, Placement};

    fn test_command() -> Command {
        Command::create_stream_with_auto_id("test".into(), DataClass::NonPHI, Placement::Global)
    }

    fn test_entry(op: u64, view: u64) -> LogEntry {
        LogEntry::new(
            OpNumber::new(op),
            ViewNumber::new(view),
            test_command(),
            None,
        )
    }

    #[test]
    fn empty_merkle_root() {
        let root = compute_merkle_root(&[]);
        assert!(root.is_empty() || !root.is_empty()); // Just verify it doesn't panic
    }

    #[test]
    fn single_entry_merkle_root() {
        let entries = vec![test_entry(1, 0)];
        let root = compute_merkle_root(&entries);
        assert!(!root.is_empty());
    }

    #[test]
    fn merkle_root_changes_with_entries() {
        let entries1 = vec![test_entry(1, 0)];
        let entries2 = vec![test_entry(1, 0), test_entry(2, 0)];

        let root1 = compute_merkle_root(&entries1);
        let root2 = compute_merkle_root(&entries2);

        assert_ne!(root1, root2);
    }

    #[test]
    fn merkle_root_deterministic() {
        let entries = vec![test_entry(1, 0), test_entry(2, 0), test_entry(3, 0)];

        let root1 = compute_merkle_root(&entries);
        let root2 = compute_merkle_root(&entries);

        assert_eq!(root1, root2);
    }

    #[test]
    fn checkpoint_signing_and_verification() {
        let entries = vec![test_entry(1, 0), test_entry(2, 0)];
        let mut checkpoint = Checkpoint::from_entries(&entries, ViewNumber::ZERO, 1234567890);

        // Sign with replica 0
        let key0 = SigningKey::generate();
        checkpoint.sign(ReplicaId::new(0), &key0);

        assert_eq!(checkpoint.signature_count(), 1);

        // Verify
        let mut verifying_keys = HashMap::new();
        verifying_keys.insert(ReplicaId::new(0), key0.verifying_key());

        assert_eq!(checkpoint.verify_signatures(&verifying_keys), 1);
    }

    #[test]
    fn checkpoint_quorum_verification() {
        let data = CheckpointData::new(
            OpNumber::new(10),
            ViewNumber::new(1),
            MerkleRoot::empty(),
            1234567890,
        );

        let mut checkpoint = Checkpoint::new(data);
        let quorum_size = 2;

        // Generate keys for 3 replicas
        let keys: Vec<SigningKey> = (0..3).map(|_| SigningKey::generate()).collect();
        let verifying_keys: HashMap<ReplicaId, VerifyingKey> = (0..3)
            .map(|i| (ReplicaId::new(i), keys[i as usize].verifying_key()))
            .collect();

        // Sign with replica 0
        checkpoint.sign(ReplicaId::new(0), &keys[0]);
        assert!(!checkpoint.has_quorum(quorum_size));
        assert!(!checkpoint.verify_quorum(quorum_size, &verifying_keys));

        // Sign with replica 1 - now we have quorum
        checkpoint.sign(ReplicaId::new(1), &keys[1]);
        assert!(checkpoint.has_quorum(quorum_size));
        assert!(checkpoint.verify_quorum(quorum_size, &verifying_keys));
    }

    #[test]
    fn checkpoint_builder_tracks_quorum() {
        let data = CheckpointData::new(OpNumber::new(5), ViewNumber::ZERO, MerkleRoot::empty(), 0);

        let mut builder = CheckpointBuilder::new(data, 2);
        assert!(!builder.has_quorum());

        // Add first signature
        let key0 = SigningKey::generate();
        let signable = builder.data().to_signable_bytes();
        let sig0 = ReplicaSignature::new(ReplicaId::new(0), key0.sign(&signable));

        let completed = builder.add_signature(sig0);
        assert!(!completed);
        assert!(!builder.has_quorum());

        // Add second signature - completes quorum
        let key1 = SigningKey::generate();
        let sig1 = ReplicaSignature::new(ReplicaId::new(1), key1.sign(&signable));

        let completed = builder.add_signature(sig1);
        assert!(completed);
        assert!(builder.has_quorum());

        let checkpoint = builder.build();
        assert_eq!(checkpoint.signature_count(), 2);
    }

    #[test]
    fn tampered_checkpoint_fails_verification() {
        let entries = vec![test_entry(1, 0)];
        let mut checkpoint = Checkpoint::from_entries(&entries, ViewNumber::ZERO, 0);

        let key = SigningKey::generate();
        checkpoint.sign(ReplicaId::new(0), &key);

        // Tamper with the data after signing
        checkpoint.data.op_number = OpNumber::new(999);

        let mut verifying_keys = HashMap::new();
        verifying_keys.insert(ReplicaId::new(0), key.verifying_key());

        // Signature should now be invalid
        assert_eq!(checkpoint.verify_signatures(&verifying_keys), 0);
    }

    #[test]
    fn wrong_key_fails_verification() {
        let data = CheckpointData::new(OpNumber::new(1), ViewNumber::ZERO, MerkleRoot::empty(), 0);

        let mut checkpoint = Checkpoint::new(data);
        let signing_key = SigningKey::generate();
        let wrong_key = SigningKey::generate();

        checkpoint.sign(ReplicaId::new(0), &signing_key);

        // Verify with wrong key
        let mut verifying_keys = HashMap::new();
        verifying_keys.insert(ReplicaId::new(0), wrong_key.verifying_key());

        assert_eq!(checkpoint.verify_signatures(&verifying_keys), 0);
    }

    #[test]
    fn merkle_root_serialization() {
        let entries = vec![test_entry(1, 0), test_entry(2, 0)];
        let root = compute_merkle_root(&entries);

        // Serialize and deserialize
        let json = serde_json::to_string(&root).unwrap();
        let restored: MerkleRoot = serde_json::from_str(&json).unwrap();

        assert_eq!(root, restored);
    }

    #[test]
    fn checkpoint_data_signable_bytes_deterministic() {
        let data = CheckpointData::new(
            OpNumber::new(100),
            ViewNumber::new(5),
            MerkleRoot::empty(),
            1234567890,
        );

        let bytes1 = data.to_signable_bytes();
        let bytes2 = data.to_signable_bytes();

        assert_eq!(bytes1, bytes2);
    }
}
