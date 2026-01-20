//! AES-256-GCM authenticated encryption for tenant data isolation.
//!
//! Provides envelope encryption where each tenant's data is encrypted with
//! a unique key. AES-256-GCM is a FIPS 197 approved AEAD cipher that provides
//! both confidentiality and integrity.
//!
//! # Example
//!
//! ```
//! use vdb_crypto::encryption::{EncryptionKey, Nonce, encrypt, decrypt};
//!
//! let key = EncryptionKey::generate();
//! let position: u64 = 42;  // Log position of the record
//! let nonce = Nonce::from_position(position);
//!
//! let plaintext = b"sensitive tenant data";
//! let ciphertext = encrypt(&key, &nonce, plaintext);
//!
//! let decrypted = decrypt(&key, &nonce, &ciphertext).unwrap();
//! assert_eq!(decrypted, plaintext.as_slice());
//! ```
//!
//! # Security
//!
//! - **Never reuse a nonce** with the same key. For `VerityDB`, nonces are
//!   derived from log position to guarantee uniqueness.
//! - Store keys encrypted at rest using [`WrappedKey`] for the key hierarchy.
//! - The authentication tag prevents tampering — decryption fails if the
//!   ciphertext or tag is modified.

use aes_gcm::{Aes256Gcm, KeyInit, aead::Aead};
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::CryptoError;

// ============================================================================
// Constants
// ============================================================================

/// Length of an AES-256-GCM encryption key in bytes (256 bits).
///
/// AES-256-GCM is chosen as a FIPS 197 approved algorithm, providing
/// compliance for regulated industries (healthcare, finance, government).
pub const KEY_LENGTH: usize = 32;

/// Length of an AES-256-GCM nonce in bytes (96 bits).
///
/// In `VerityDB`, nonces are derived from the log position to guarantee
/// uniqueness without requiring random generation.
pub const NONCE_LENGTH: usize = 12;

/// Length of the AES-GCM authentication tag in bytes (128 bits).
///
/// The authentication tag ensures integrity — decryption fails if the
/// ciphertext or tag has been tampered with.
pub const TAG_LENGTH: usize = 16;

pub const WRAPPED_KEY_LENGTH: usize = NONCE_LENGTH + KEY_LENGTH + TAG_LENGTH;

// ============================================================================
// EncryptionKey
// ============================================================================

/// An AES-256-GCM encryption key (256 bits).
///
/// This is secret key material that must be protected. Use [`EncryptionKey::generate`]
/// to create a new random key, or [`EncryptionKey::from_bytes`] to restore from
/// secure storage.
///
/// Key material is securely zeroed from memory when dropped via [`ZeroizeOnDrop`].
///
/// # Security
///
/// - Never log or expose the key bytes
/// - Store encrypted at rest (wrap with a KEK from the key hierarchy)
/// - Use one key per tenant/segment for isolation
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct EncryptionKey([u8; KEY_LENGTH]);

impl EncryptionKey {
    // ========================================================================
    // Functional Core (pure, testable)
    // ========================================================================

    /// Creates an encryption key from random bytes (pure, no IO).
    ///
    /// This is the functional core - it performs no IO and is fully testable.
    /// Use [`Self::generate`] for the public API that handles randomness.
    ///
    /// # Security
    ///
    /// Only use bytes from a CSPRNG. This is `pub(crate)` to prevent misuse.
    pub(crate) fn from_random_bytes(bytes: [u8; KEY_LENGTH]) -> Self {
        // Precondition: caller provided non-degenerate random bytes
        debug_assert!(
            bytes.iter().any(|&b| b != 0),
            "random bytes are all zeros"
        );

        Self(bytes)
    }

    /// Restores an encryption key from its 32-byte representation.
    ///
    /// # Security
    ///
    /// Only use bytes from a previously generated key or a secure KDF.
    pub fn from_bytes(bytes: &[u8; KEY_LENGTH]) -> Self {
        // Precondition: caller didn't pass degenerate key material
        debug_assert!(bytes.iter().any(|&b| b != 0), "key bytes are all zeros");

        Self(*bytes)
    }

    /// Returns the raw 32-byte key material.
    ///
    /// # Security
    ///
    /// Handle with care — this is secret key material.
    pub fn to_bytes(&self) -> [u8; KEY_LENGTH] {
        self.0
    }

    // ========================================================================
    // Imperative Shell (IO boundary)
    // ========================================================================

    /// Generates a new random encryption key using the OS CSPRNG.
    ///
    /// This is the imperative shell - it handles IO (randomness) and delegates
    /// to the pure [`Self::from_random_bytes`] for the actual construction.
    ///
    /// # Panics
    ///
    /// Panics if the OS CSPRNG fails (catastrophic system error).
    pub fn generate() -> Self {
        let random_bytes: [u8; KEY_LENGTH] = generate_random();
        Self::from_random_bytes(random_bytes)
    }
}

// ============================================================================
// Nonce
// ============================================================================

/// An AES-256-GCM nonce (96 bits) derived from log position.
///
/// Nonces in `VerityDB` are deterministically derived from the record's log
/// position, guaranteeing uniqueness without requiring random generation.
/// The 8-byte position is placed in the first 8 bytes; the remaining 4 bytes
/// are reserved (currently zero).
///
/// ```text
/// Nonce layout (12 bytes):
/// ┌────────────────────────────┬──────────────┐
/// │  position (8 bytes, LE)    │  reserved    │
/// │  [0..8]                    │  [8..12]     │
/// └────────────────────────────┴──────────────┘
/// ```
///
/// # Security
///
/// **Never reuse a nonce with the same key.** Nonce reuse completely breaks
/// the confidentiality of AES-GCM. Position-derived nonces guarantee uniqueness
/// as long as each position is encrypted at most once per key.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Nonce([u8; NONCE_LENGTH]);

impl Nonce {
    // ========================================================================
    // Functional Core (pure, testable)
    // ========================================================================

    /// Creates a nonce from random bytes (pure, no IO).
    ///
    /// This is the functional core - it performs no IO and is fully testable.
    /// Use [`Self::generate_random`] for the public API that handles randomness.
    ///
    /// # Security
    ///
    /// Only use bytes from a CSPRNG. This is `pub(crate)` to prevent misuse.
    pub(crate) fn from_random_bytes(bytes: [u8; NONCE_LENGTH]) -> Self {
        // Precondition: caller provided non-degenerate random bytes
        debug_assert!(
            bytes.iter().any(|&b| b != 0),
            "random bytes are all zeros"
        );

        Self(bytes)
    }

    /// Creates a nonce from a log position (pure, deterministic).
    ///
    /// The position's little-endian bytes occupy the first 8 bytes of the
    /// nonce. The remaining 4 bytes are reserved for future use (e.g., key
    /// rotation counter) and are currently set to zero.
    ///
    /// # Arguments
    ///
    /// * `position` - The log position of the record being encrypted
    pub fn from_position(position: u64) -> Self {
        let mut nonce = [0u8; NONCE_LENGTH];
        nonce[..8].copy_from_slice(&position.to_le_bytes());

        Self(nonce)
    }

    /// Creates a nonce from raw bytes.
    ///
    /// Used when deserializing a nonce from storage (e.g., from a [`WrappedKey`]).
    ///
    /// # Arguments
    ///
    /// * `bytes` - The 12-byte nonce
    pub fn from_bytes(bytes: [u8; NONCE_LENGTH]) -> Self {
        Self(bytes)
    }

    /// Returns the raw 12-byte nonce.
    ///
    /// Use this for serialization or when interfacing with external systems.
    pub fn to_bytes(&self) -> [u8; NONCE_LENGTH] {
        self.0
    }

    // ========================================================================
    // Imperative Shell (IO boundary)
    // ========================================================================

    /// Generates a random nonce using the OS CSPRNG.
    ///
    /// Used for key wrapping where there's no log position to derive from.
    /// The random nonce must be stored alongside the ciphertext.
    ///
    /// This is the imperative shell - it handles IO (randomness) and delegates
    /// to the pure [`Self::from_random_bytes`] for the actual construction.
    ///
    /// # Panics
    ///
    /// Panics if the OS CSPRNG fails (catastrophic system error).
    pub fn generate_random() -> Self {
        let random_bytes: [u8; NONCE_LENGTH] = generate_random();
        Self::from_random_bytes(random_bytes)
    }
}

// ============================================================================
// Ciphertext
// ============================================================================

/// Encrypted data with its authentication tag.
///
/// Contains the ciphertext followed by a 16-byte AES-GCM authentication tag.
/// The total length is `plaintext.len() + TAG_LENGTH`. Decryption will fail
/// if the ciphertext or tag has been modified.
///
/// ```text
/// Ciphertext layout:
/// ┌────────────────────────────┬──────────────────┐
/// │  encrypted data            │  auth tag        │
/// │  [0..plaintext.len()]      │  [last 16 bytes] │
/// └────────────────────────────┴──────────────────┘
/// ```
#[derive(Clone, PartialEq, Eq)]
pub struct Ciphertext(Vec<u8>);

impl Ciphertext {
    /// Creates a ciphertext from raw bytes.
    ///
    /// # Arguments
    ///
    /// * `bytes` - The encrypted data with authentication tag appended
    ///
    /// # Panics
    ///
    /// Debug builds panic if `bytes.len() < TAG_LENGTH` (no room for auth tag).
    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        debug_assert!(
            bytes.len() >= TAG_LENGTH,
            "ciphertext too short: must be at least {TAG_LENGTH} bytes for auth tag"
        );

        Self(bytes)
    }

    /// Returns the raw ciphertext bytes (including authentication tag).
    ///
    /// Use this for serialization or storage.
    pub fn to_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Returns the length of the ciphertext (including authentication tag).
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Returns true if the ciphertext is empty (which would be invalid).
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

// ============================================================================
// WrappedKey
// ============================================================================

/// An encryption key wrapped (encrypted) by another key.
///
/// Used in the key hierarchy to protect DEKs with KEKs, and KEKs with the
/// master key. The wrapped key can be safely stored on disk — it cannot be
/// unwrapped without the wrapping key.
///
/// ```text
/// WrappedKey layout (60 bytes):
/// ┌────────────────────────────┬────────────────────────────┬──────────────┐
/// │  nonce (12 bytes)          │  encrypted key (32 bytes)  │  tag (16)    │
/// └────────────────────────────┴────────────────────────────┴──────────────┘
/// ```
///
/// # Security
///
/// - The wrapped ciphertext is encrypted, not raw key material
/// - The nonce is not secret (it's stored alongside the ciphertext)
/// - `ZeroizeOnDrop` is not needed since this contains no plaintext secrets
/// - The wrapping key (KEK/MK) is what must be protected
///
/// # Example
///
/// ```
/// use vdb_crypto::encryption::{EncryptionKey, WrappedKey, KEY_LENGTH};
///
/// // KEK wraps a DEK
/// let kek = EncryptionKey::generate();
/// let dek_bytes: [u8; KEY_LENGTH] = [0x42; KEY_LENGTH]; // In practice, use generate()
///
/// let wrapped = WrappedKey::new(&kek, &dek_bytes);
///
/// // Store wrapped.to_bytes() on disk...
///
/// // Later, unwrap with the same KEK
/// let unwrapped = wrapped.unwrap_key(&kek).unwrap();
/// assert_eq!(dek_bytes, unwrapped);
/// ```
pub struct WrappedKey {
    nonce: Nonce,
    ciphertext: Ciphertext,
}

impl WrappedKey {
    /// Wraps (encrypts) a key using the provided wrapping key.
    ///
    /// Generates a random nonce and encrypts the key material using AES-256-GCM.
    /// The nonce is stored alongside the ciphertext for later unwrapping.
    ///
    /// # Arguments
    ///
    /// * `wrapping_key` - The key used to encrypt (KEK or master key)
    /// * `key_to_wrap` - The 32-byte key material to protect (DEK or KEK)
    ///
    /// # Returns
    ///
    /// A [`WrappedKey`] that can be serialized and stored safely.
    pub fn new(wrapping_key: &EncryptionKey, key_to_wrap: &[u8; KEY_LENGTH]) -> Self {
        // Precondition: key material isn't degenerate
        debug_assert!(
            key_to_wrap.iter().any(|&b| b != 0),
            "key_to_wrap is all zeros"
        );

        let nonce = Nonce::generate_random();
        let ciphertext = encrypt(wrapping_key, &nonce, key_to_wrap);

        // Postcondition: ciphertext is correct size (key + tag)
        debug_assert_eq!(
            ciphertext.len(),
            KEY_LENGTH + TAG_LENGTH,
            "wrapped ciphertext has unexpected length"
        );

        Self { nonce, ciphertext }
    }

    /// Unwraps (decrypts) the key using the provided wrapping key.
    ///
    /// Verifies the authentication tag and returns the original key material
    /// if the wrapping key is correct.
    ///
    /// # Arguments
    ///
    /// * `wrapping_key` - The key used during wrapping (must match)
    ///
    /// # Errors
    ///
    /// Returns [`CryptoError::DecryptionError`] if:
    /// - The wrapping key is incorrect
    /// - The wrapped data has been tampered with
    ///
    /// # Returns
    ///
    /// The original 32-byte key material.
    pub fn unwrap_key(
        &self,
        wrapping_key: &EncryptionKey,
    ) -> Result<[u8; KEY_LENGTH], CryptoError> {
        let decrypted = decrypt(wrapping_key, &self.nonce, &self.ciphertext)?;

        // Postcondition: decrypted key has correct length
        debug_assert_eq!(
            decrypted.len(),
            KEY_LENGTH,
            "unwrapped key has unexpected length"
        );

        decrypted
            .try_into()
            .map_err(|_| CryptoError::DecryptionError)
    }

    /// Serializes the wrapped key to bytes for storage.
    ///
    /// The format is: `nonce (12 bytes) || ciphertext (48 bytes)`.
    ///
    /// # Returns
    ///
    /// A 60-byte array suitable for storing on disk or in a database.
    pub fn to_bytes(&self) -> [u8; WRAPPED_KEY_LENGTH] {
        let mut bytes = [0u8; WRAPPED_KEY_LENGTH];
        bytes[..NONCE_LENGTH].copy_from_slice(&self.nonce.to_bytes());
        bytes[NONCE_LENGTH..].copy_from_slice(self.ciphertext.to_bytes());

        // Postcondition: we produced non-degenerate output
        debug_assert!(
            bytes.iter().any(|&b| b != 0),
            "serialized wrapped key is all zeros"
        );

        bytes
    }

    /// Deserializes a wrapped key from bytes.
    ///
    /// # Arguments
    ///
    /// * `bytes` - A 60-byte array from [`WrappedKey::to_bytes`]
    ///
    /// # Returns
    ///
    /// A [`WrappedKey`] that can be unwrapped with the original wrapping key.
    pub fn from_bytes(bytes: &[u8; WRAPPED_KEY_LENGTH]) -> Self {
        // Precondition: bytes aren't all zeros (likely corrupted or uninitialized)
        debug_assert!(
            bytes.iter().any(|&b| b != 0),
            "wrapped key bytes are all zeros"
        );

        let mut nonce_bytes = [0u8; NONCE_LENGTH];
        nonce_bytes.copy_from_slice(&bytes[..NONCE_LENGTH]);
        let ciphertext = Ciphertext::from_bytes(bytes[NONCE_LENGTH..].to_vec());

        Self {
            nonce: Nonce::from_bytes(nonce_bytes),
            ciphertext,
        }
    }
}

// ============================================================================
// Key Hierarchy
// ============================================================================

/// Provider for master key operations.
///
/// The master key is the root of the key hierarchy. It wraps Key Encryption
/// Keys (KEKs), which in turn wrap Data Encryption Keys (DEKs).
///
/// ```text
/// MasterKeyProvider
///     │
///     └── wraps ──► KeyEncryptionKey (per tenant)
///                       │
///                       └── wraps ──► DataEncryptionKey (per segment)
///                                         │
///                                         └── encrypts ──► Record data
/// ```
///
/// This trait abstracts the master key storage, enabling:
/// - [`InMemoryMasterKey`]: Development, testing, single-node deployments
/// - Future: HSM-backed implementation where the key never leaves the hardware
///
/// # Security
///
/// The master key is the most sensitive secret in the system. If compromised,
/// all tenant data can be decrypted. In production, use an HSM.
pub trait MasterKeyProvider {
    /// Wraps a Key Encryption Key for secure storage.
    ///
    /// The wrapped KEK can be stored on disk and later unwrapped with
    /// [`MasterKeyProvider::unwrap_kek`].
    fn wrap_kek(&self, kek_bytes: &[u8; KEY_LENGTH]) -> WrappedKey;

    /// Unwraps a Key Encryption Key from storage.
    ///
    /// # Errors
    ///
    /// Returns [`CryptoError::DecryptionError`] if the wrapped key is
    /// corrupted or was wrapped by a different master key.
    fn unwrap_kek(&self, wrapped: &WrappedKey) -> Result<[u8; KEY_LENGTH], CryptoError>;
}

/// In-memory master key for development and testing.
///
/// Stores the master key material directly in memory. Suitable for:
/// - Development and testing
/// - Single-node deployments with disk encryption
/// - Environments without HSM access
///
/// # Security
///
/// The key material is zeroed on drop via [`ZeroizeOnDrop`]. However, for
/// production deployments handling sensitive data, prefer an HSM-backed
/// implementation.
///
/// # Example
///
/// ```
/// use vdb_crypto::encryption::{InMemoryMasterKey, MasterKeyProvider, KeyEncryptionKey};
///
/// let master = InMemoryMasterKey::generate();
/// let (kek, wrapped_kek) = KeyEncryptionKey::generate_and_wrap(&master);
///
/// // Store wrapped_kek.to_bytes() on disk...
/// ```
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct InMemoryMasterKey(EncryptionKey);

impl InMemoryMasterKey {
    // ========================================================================
    // Functional Core (pure, testable)
    // ========================================================================

    /// Creates a master key from random bytes (pure, no IO).
    ///
    /// This is the functional core - it performs no IO and is fully testable.
    /// Use [`Self::generate`] for the public API that handles randomness.
    ///
    /// # Security
    ///
    /// Only use bytes from a CSPRNG. This is `pub(crate)` to prevent misuse.
    pub(crate) fn from_random_bytes(bytes: [u8; KEY_LENGTH]) -> Self {
        Self(EncryptionKey::from_random_bytes(bytes))
    }

    /// Restores a master key from its 32-byte representation.
    ///
    /// # Security
    ///
    /// Only use bytes from a previously generated key or secure backup.
    /// The bytes should come from encrypted-at-rest storage.
    pub fn from_bytes(bytes: &[u8; KEY_LENGTH]) -> Self {
        // Precondition: caller didn't pass degenerate key material
        debug_assert!(
            bytes.iter().any(|&b| b != 0),
            "master key bytes are all zeros"
        );

        Self(EncryptionKey::from_bytes(bytes))
    }

    /// Returns the raw 32-byte key material for backup.
    ///
    /// # Security
    ///
    /// **Handle with extreme care.** This is the root secret of the entire
    /// key hierarchy. Only use this for:
    /// - Secure backup to encrypted storage
    /// - Key escrow with proper controls
    ///
    /// Never log, transmit unencrypted, or store in plaintext.
    pub fn to_bytes(&self) -> [u8; KEY_LENGTH] {
        self.0.to_bytes()
    }

    // ========================================================================
    // Imperative Shell (IO boundary)
    // ========================================================================

    /// Generates a new random master key using the OS CSPRNG.
    ///
    /// This is the imperative shell - it handles IO (randomness) and delegates
    /// to the pure [`Self::from_random_bytes`] for the actual construction.
    ///
    /// # Panics
    ///
    /// Panics if the OS CSPRNG fails (catastrophic system error).
    pub fn generate() -> Self {
        let random_bytes: [u8; KEY_LENGTH] = generate_random();
        Self::from_random_bytes(random_bytes)
    }
}

impl MasterKeyProvider for InMemoryMasterKey {
    fn wrap_kek(&self, kek_bytes: &[u8; KEY_LENGTH]) -> WrappedKey {
        // Precondition: KEK bytes aren't degenerate
        debug_assert!(
            kek_bytes.iter().any(|&b| b != 0),
            "KEK bytes are all zeros"
        );

        WrappedKey::new(&self.0, kek_bytes)
    }

    fn unwrap_kek(&self, wrapped: &WrappedKey) -> Result<[u8; KEY_LENGTH], CryptoError> {
        let kek_bytes = wrapped.unwrap_key(&self.0)?;

        // Postcondition: unwrapped KEK isn't degenerate
        debug_assert!(
            kek_bytes.iter().any(|&b| b != 0),
            "unwrapped KEK is all zeros"
        );

        Ok(kek_bytes)
    }
}

/// Key Encryption Key (KEK) for wrapping Data Encryption Keys.
///
/// Each tenant has one KEK. The KEK is wrapped by the master key and stored
/// alongside tenant metadata. Deleting a tenant's wrapped KEK renders all
/// their data cryptographically inaccessible (GDPR "right to erasure").
///
/// # Key Hierarchy Position
///
/// ```text
/// MasterKeyProvider
///     │
///     └── wraps ──► KeyEncryptionKey (this type)
///                       │
///                       └── wraps ──► DataEncryptionKey
/// ```
///
/// # Example
///
/// ```
/// use vdb_crypto::encryption::{
///     InMemoryMasterKey, MasterKeyProvider, KeyEncryptionKey, DataEncryptionKey,
/// };
///
/// let master = InMemoryMasterKey::generate();
///
/// // Create KEK for a new tenant
/// let (kek, wrapped_kek) = KeyEncryptionKey::generate_and_wrap(&master);
///
/// // Store wrapped_kek.to_bytes() in tenant metadata...
///
/// // Later: restore KEK when tenant accesses data
/// let kek = KeyEncryptionKey::restore(&master, &wrapped_kek).unwrap();
/// ```
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct KeyEncryptionKey(EncryptionKey);

impl KeyEncryptionKey {
    // ========================================================================
    // Functional Core (pure, testable)
    // ========================================================================

    /// Creates a KEK from random bytes and wraps it (pure, no IO).
    ///
    /// This is the functional core - it performs no IO and is fully testable.
    /// Use [`Self::generate_and_wrap`] for the public API that handles randomness.
    ///
    /// # Security
    ///
    /// Only use bytes from a CSPRNG. This is `pub(crate)` to prevent misuse.
    pub(crate) fn from_random_bytes_and_wrap(
        random_bytes: [u8; KEY_LENGTH],
        master: &impl MasterKeyProvider,
    ) -> (Self, WrappedKey) {
        let key = EncryptionKey::from_random_bytes(random_bytes);
        let wrapped = master.wrap_kek(&key.to_bytes());

        (Self(key), wrapped)
    }

    /// Restores a KEK from its wrapped form (pure, no IO).
    ///
    /// Use this when loading a tenant's KEK from storage.
    ///
    /// # Arguments
    ///
    /// * `master` - The master key provider that originally wrapped this KEK
    /// * `wrapped` - The wrapped KEK from storage
    ///
    /// # Errors
    ///
    /// Returns [`CryptoError::DecryptionError`] if:
    /// - The wrapped key is corrupted
    /// - The wrong master key is used
    pub fn restore(
        master: &impl MasterKeyProvider,
        wrapped: &WrappedKey,
    ) -> Result<Self, CryptoError> {
        let key_bytes = master.unwrap_kek(wrapped)?;

        // Postcondition: restored key isn't degenerate
        debug_assert!(
            key_bytes.iter().any(|&b| b != 0),
            "restored KEK is all zeros"
        );

        Ok(Self(EncryptionKey::from_bytes(&key_bytes)))
    }

    /// Wraps a Data Encryption Key for secure storage.
    ///
    /// The wrapped DEK should be stored in the segment header.
    pub fn wrap_dek(&self, dek_bytes: &[u8; KEY_LENGTH]) -> WrappedKey {
        // Precondition: DEK bytes aren't degenerate
        debug_assert!(
            dek_bytes.iter().any(|&b| b != 0),
            "DEK bytes are all zeros"
        );

        WrappedKey::new(&self.0, dek_bytes)
    }

    /// Unwraps a Data Encryption Key from storage.
    ///
    /// # Errors
    ///
    /// Returns [`CryptoError::DecryptionError`] if:
    /// - The wrapped key is corrupted
    /// - The wrong KEK is used
    pub fn unwrap_dek(&self, wrapped: &WrappedKey) -> Result<[u8; KEY_LENGTH], CryptoError> {
        let dek_bytes = wrapped.unwrap_key(&self.0)?;

        // Postcondition: unwrapped DEK isn't degenerate
        debug_assert!(
            dek_bytes.iter().any(|&b| b != 0),
            "unwrapped DEK is all zeros"
        );

        Ok(dek_bytes)
    }

    // ========================================================================
    // Imperative Shell (IO boundary)
    // ========================================================================

    /// Generates a new KEK and wraps it with the master key.
    ///
    /// Returns both the usable KEK and its wrapped form for storage.
    /// The wrapped form should be persisted alongside tenant metadata.
    ///
    /// This is the imperative shell - it handles IO (randomness) and delegates
    /// to the pure [`Self::from_random_bytes_and_wrap`] for the actual construction.
    ///
    /// # Arguments
    ///
    /// * `master` - The master key provider to wrap the KEK
    ///
    /// # Returns
    ///
    /// A tuple of `(usable_kek, wrapped_kek_for_storage)`.
    ///
    /// # Panics
    ///
    /// Panics if the OS CSPRNG fails (catastrophic system error).
    pub fn generate_and_wrap(master: &impl MasterKeyProvider) -> (Self, WrappedKey) {
        let random_bytes: [u8; KEY_LENGTH] = generate_random();
        Self::from_random_bytes_and_wrap(random_bytes, master)
    }
}

/// Data Encryption Key (DEK) for encrypting actual record data.
///
/// Each segment (chunk of the log) has its own DEK. The DEK is wrapped by
/// the tenant's KEK and stored in the segment header.
///
/// # Key Hierarchy Position
///
/// ```text
/// MasterKeyProvider
///     │
///     └── wraps ──► KeyEncryptionKey
///                       │
///                       └── wraps ──► DataEncryptionKey (this type)
///                                         │
///                                         └── encrypts ──► Record data
/// ```
///
/// # Example
///
/// ```
/// use vdb_crypto::encryption::{
///     InMemoryMasterKey, KeyEncryptionKey, DataEncryptionKey,
///     Nonce, encrypt, decrypt,
/// };
///
/// let master = InMemoryMasterKey::generate();
/// let (kek, _) = KeyEncryptionKey::generate_and_wrap(&master);
///
/// // Create DEK for a new segment
/// let (dek, wrapped_dek) = DataEncryptionKey::generate_and_wrap(&kek);
///
/// // Encrypt data
/// let nonce = Nonce::from_position(0);
/// let ciphertext = encrypt(dek.encryption_key(), &nonce, b"secret data");
///
/// // Decrypt data
/// let plaintext = decrypt(dek.encryption_key(), &nonce, &ciphertext).unwrap();
/// assert_eq!(plaintext, b"secret data");
/// ```
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct DataEncryptionKey(EncryptionKey);

impl DataEncryptionKey {
    // ========================================================================
    // Functional Core (pure, testable)
    // ========================================================================

    /// Creates a DEK from random bytes and wraps it (pure, no IO).
    ///
    /// This is the functional core - it performs no IO and is fully testable.
    /// Use [`Self::generate_and_wrap`] for the public API that handles randomness.
    ///
    /// # Security
    ///
    /// Only use bytes from a CSPRNG. This is `pub(crate)` to prevent misuse.
    pub(crate) fn from_random_bytes_and_wrap(
        random_bytes: [u8; KEY_LENGTH],
        kek: &KeyEncryptionKey,
    ) -> (Self, WrappedKey) {
        let key = EncryptionKey::from_random_bytes(random_bytes);
        let wrapped = kek.wrap_dek(&key.to_bytes());

        (Self(key), wrapped)
    }

    /// Restores a DEK from its wrapped form (pure, no IO).
    ///
    /// Use this when loading a segment's DEK from its header.
    ///
    /// # Arguments
    ///
    /// * `kek` - The KEK that originally wrapped this DEK
    /// * `wrapped` - The wrapped DEK from the segment header
    ///
    /// # Errors
    ///
    /// Returns [`CryptoError::DecryptionError`] if:
    /// - The wrapped key is corrupted
    /// - The wrong KEK is used
    pub fn restore(kek: &KeyEncryptionKey, wrapped: &WrappedKey) -> Result<Self, CryptoError> {
        let key_bytes = kek.unwrap_dek(wrapped)?;

        // Postcondition: restored key isn't degenerate
        debug_assert!(
            key_bytes.iter().any(|&b| b != 0),
            "restored DEK is all zeros"
        );

        Ok(Self(EncryptionKey::from_bytes(&key_bytes)))
    }

    /// Returns a reference to the underlying encryption key.
    ///
    /// Use this with [`encrypt`] and [`decrypt`] to encrypt/decrypt record data.
    ///
    /// # Example
    ///
    /// ```
    /// # use vdb_crypto::encryption::{InMemoryMasterKey, KeyEncryptionKey, DataEncryptionKey, Nonce, encrypt};
    /// # let master = InMemoryMasterKey::generate();
    /// # let (kek, _) = KeyEncryptionKey::generate_and_wrap(&master);
    /// # let (dek, _) = DataEncryptionKey::generate_and_wrap(&kek);
    /// let nonce = Nonce::from_position(42);
    /// let ciphertext = encrypt(dek.encryption_key(), &nonce, b"data");
    /// ```
    pub fn encryption_key(&self) -> &EncryptionKey {
        &self.0
    }

    // ========================================================================
    // Imperative Shell (IO boundary)
    // ========================================================================

    /// Generates a new DEK and wraps it with the KEK.
    ///
    /// Returns both the usable DEK and its wrapped form for storage.
    /// The wrapped form should be stored in the segment header.
    ///
    /// This is the imperative shell - it handles IO (randomness) and delegates
    /// to the pure [`Self::from_random_bytes_and_wrap`] for the actual construction.
    ///
    /// # Arguments
    ///
    /// * `kek` - The Key Encryption Key to wrap this DEK
    ///
    /// # Returns
    ///
    /// A tuple of `(usable_dek, wrapped_dek_for_storage)`.
    ///
    /// # Panics
    ///
    /// Panics if the OS CSPRNG fails (catastrophic system error).
    pub fn generate_and_wrap(kek: &KeyEncryptionKey) -> (Self, WrappedKey) {
        let random_bytes: [u8; KEY_LENGTH] = generate_random();
        Self::from_random_bytes_and_wrap(random_bytes, kek)
    }
}

// ============================================================================
// Encrypt / Decrypt
// ============================================================================

/// Maximum plaintext size for encryption (64 MiB).
///
/// This is a sanity limit to catch accidental misuse. AES-GCM can encrypt
/// larger messages, but individual records should never approach this size.
#[allow(dead_code)]
const MAX_PLAINTEXT_LENGTH: usize = 64 * 1024 * 1024;

/// Encrypts plaintext using AES-256-GCM.
///
/// Returns a [`Ciphertext`] containing the encrypted data with a 16-byte
/// authentication tag appended. The ciphertext length is `plaintext.len() + 16`.
///
/// # Arguments
///
/// * `key` - The encryption key
/// * `nonce` - A unique nonce derived from log position (never reuse with the same key!)
/// * `plaintext` - The data to encrypt
///
/// # Returns
///
/// A [`Ciphertext`] containing the encrypted data and authentication tag.
///
/// # Panics
///
/// Debug builds panic if `plaintext` exceeds 64 MiB (sanity limit).
pub fn encrypt(key: &EncryptionKey, nonce: &Nonce, plaintext: &[u8]) -> Ciphertext {
    // Precondition: plaintext length is reasonable
    debug_assert!(
        plaintext.len() <= MAX_PLAINTEXT_LENGTH,
        "plaintext exceeds {MAX_PLAINTEXT_LENGTH} byte sanity limit"
    );

    let cipher = Aes256Gcm::new_from_slice(&key.0).expect("KEY_LENGTH is always valid");
    let nonce_array = nonce.0.into();

    let data = cipher
        .encrypt(&nonce_array, plaintext)
        .expect("AES-GCM encryption cannot fail with valid inputs");

    // Postcondition: ciphertext is plaintext + tag
    debug_assert_eq!(
        data.len(),
        plaintext.len() + TAG_LENGTH,
        "ciphertext length mismatch"
    );

    Ciphertext(data)
}

/// Decrypts ciphertext using AES-256-GCM.
///
/// Verifies the authentication tag and returns the original plaintext if valid.
///
/// # Arguments
///
/// * `key` - The encryption key (must match the key used for encryption)
/// * `nonce` - The nonce used during encryption (same log position)
/// * `ciphertext` - The encrypted data with authentication tag
///
/// # Errors
///
/// Returns [`CryptoError::DecryptionError`] if:
/// - The key is incorrect
/// - The nonce is incorrect
/// - The ciphertext has been tampered with
/// - The authentication tag is invalid
///
/// # Panics
///
/// Debug builds panic if `ciphertext` is shorter than [`TAG_LENGTH`] bytes.
pub fn decrypt(
    key: &EncryptionKey,
    nonce: &Nonce,
    ciphertext: &Ciphertext,
) -> Result<Vec<u8>, CryptoError> {
    // Precondition: ciphertext has at least the auth tag
    let ciphertext_len = ciphertext.0.len();
    debug_assert!(
        ciphertext_len >= TAG_LENGTH,
        "ciphertext too short: {ciphertext_len} bytes, need at least {TAG_LENGTH}"
    );

    let cipher = Aes256Gcm::new_from_slice(&key.0).expect("KEY_LENGTH is always valid");
    let nonce_array = nonce.0.into();

    let plaintext = cipher
        .decrypt(&nonce_array, ciphertext.0.as_slice())
        .map_err(|_| CryptoError::DecryptionError)?;

    // Postcondition: plaintext is ciphertext minus tag
    debug_assert_eq!(
        plaintext.len(),
        ciphertext.0.len() - TAG_LENGTH,
        "plaintext length mismatch"
    );

    Ok(plaintext)
}

// ============================================================================
// Internal Helpers
// ============================================================================

/// Fills a buffer with cryptographically secure random bytes.
///
/// # Panics
///
/// Panics if the OS CSPRNG fails. This indicates a catastrophic system error
/// (e.g., /dev/urandom unavailable) and cannot be meaningfully recovered from.
fn generate_random<const N: usize>() -> [u8; N] {
    let mut bytes = [0u8; N];
    getrandom::fill(&mut bytes).expect("CSPRNG failure");
    bytes
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encrypt_decrypt_roundtrip() {
        let key = EncryptionKey::generate();
        let nonce = Nonce::from_position(42);
        let plaintext = b"sensitive tenant data";

        let ciphertext = encrypt(&key, &nonce, plaintext);
        let decrypted = decrypt(&key, &nonce, &ciphertext).unwrap();

        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn encrypt_decrypt_empty_plaintext() {
        let key = EncryptionKey::generate();
        let nonce = Nonce::from_position(0);
        let plaintext = b"";

        let ciphertext = encrypt(&key, &nonce, plaintext);
        let decrypted = decrypt(&key, &nonce, &ciphertext).unwrap();

        assert_eq!(decrypted, plaintext);
        assert_eq!(ciphertext.len(), TAG_LENGTH); // Just the tag
    }

    #[test]
    fn ciphertext_length_is_plaintext_plus_tag() {
        let key = EncryptionKey::generate();
        let nonce = Nonce::from_position(100);
        let plaintext = b"hello world";

        let ciphertext = encrypt(&key, &nonce, plaintext);

        assert_eq!(ciphertext.len(), plaintext.len() + TAG_LENGTH);
    }

    #[test]
    fn wrong_key_fails_decryption() {
        let key1 = EncryptionKey::generate();
        let key2 = EncryptionKey::generate();
        let nonce = Nonce::from_position(42);
        let plaintext = b"secret message";

        let ciphertext = encrypt(&key1, &nonce, plaintext);
        let result = decrypt(&key2, &nonce, &ciphertext);

        assert!(result.is_err());
    }

    #[test]
    fn wrong_nonce_fails_decryption() {
        let key = EncryptionKey::generate();
        let nonce1 = Nonce::from_position(42);
        let nonce2 = Nonce::from_position(43);
        let plaintext = b"secret message";

        let ciphertext = encrypt(&key, &nonce1, plaintext);
        let result = decrypt(&key, &nonce2, &ciphertext);

        assert!(result.is_err());
    }

    #[test]
    fn tampered_ciphertext_fails_decryption() {
        let key = EncryptionKey::generate();
        let nonce = Nonce::from_position(42);
        let plaintext = b"secret message";

        let ciphertext = encrypt(&key, &nonce, plaintext);

        // Tamper with the ciphertext
        let mut tampered_bytes = ciphertext.to_bytes().to_vec();
        tampered_bytes[0] ^= 0x01; // Flip a bit
        let tampered = Ciphertext::from_bytes(tampered_bytes);

        let result = decrypt(&key, &nonce, &tampered);

        assert!(result.is_err());
    }

    #[test]
    fn tampered_tag_fails_decryption() {
        let key = EncryptionKey::generate();
        let nonce = Nonce::from_position(42);
        let plaintext = b"secret message";

        let ciphertext = encrypt(&key, &nonce, plaintext);

        // Tamper with the auth tag (last 16 bytes)
        let mut tampered_bytes = ciphertext.to_bytes().to_vec();
        let len = tampered_bytes.len();
        tampered_bytes[len - 1] ^= 0x01; // Flip a bit in the tag
        let tampered = Ciphertext::from_bytes(tampered_bytes);

        let result = decrypt(&key, &nonce, &tampered);

        assert!(result.is_err());
    }

    #[test]
    fn nonce_from_position_layout() {
        let nonce = Nonce::from_position(0x0102_0304_0506_0708);
        let bytes = nonce.to_bytes();

        // Little-endian: least significant byte first
        assert_eq!(bytes[0], 0x08);
        assert_eq!(bytes[1], 0x07);
        assert_eq!(bytes[2], 0x06);
        assert_eq!(bytes[3], 0x05);
        assert_eq!(bytes[4], 0x04);
        assert_eq!(bytes[5], 0x03);
        assert_eq!(bytes[6], 0x02);
        assert_eq!(bytes[7], 0x01);

        // Reserved bytes are zero
        assert_eq!(bytes[8], 0x00);
        assert_eq!(bytes[9], 0x00);
        assert_eq!(bytes[10], 0x00);
        assert_eq!(bytes[11], 0x00);
    }

    #[test]
    fn nonce_position_zero_is_valid() {
        let key = EncryptionKey::generate();
        let nonce = Nonce::from_position(0);
        let plaintext = b"first record";

        let ciphertext = encrypt(&key, &nonce, plaintext);
        let decrypted = decrypt(&key, &nonce, &ciphertext).unwrap();

        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn encryption_key_roundtrip() {
        let original = EncryptionKey::generate();
        let bytes = original.to_bytes();
        let restored = EncryptionKey::from_bytes(&bytes);

        // Same key should produce same ciphertext
        let nonce = Nonce::from_position(1);
        let plaintext = b"test";

        let ct1 = encrypt(&original, &nonce, plaintext);
        let ct2 = encrypt(&restored, &nonce, plaintext);

        assert_eq!(ct1.to_bytes(), ct2.to_bytes());
    }

    #[test]
    fn ciphertext_roundtrip() {
        let key = EncryptionKey::generate();
        let nonce = Nonce::from_position(999);
        let plaintext = b"data to serialize";

        let ciphertext = encrypt(&key, &nonce, plaintext);

        // Simulate serialization/deserialization
        let bytes = ciphertext.to_bytes().to_vec();
        let restored = Ciphertext::from_bytes(bytes);

        let decrypted = decrypt(&key, &nonce, &restored).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn different_positions_produce_different_ciphertexts() {
        let key = EncryptionKey::generate();
        let plaintext = b"same plaintext";

        let ct1 = encrypt(&key, &Nonce::from_position(1), plaintext);
        let ct2 = encrypt(&key, &Nonce::from_position(2), plaintext);

        // Same plaintext, different nonces = different ciphertexts
        assert_ne!(ct1.to_bytes(), ct2.to_bytes());
    }

    #[test]
    fn encryption_is_deterministic() {
        let key = EncryptionKey::generate();
        let nonce = Nonce::from_position(42);
        let plaintext = b"deterministic test";

        let ct1 = encrypt(&key, &nonce, plaintext);
        let ct2 = encrypt(&key, &nonce, plaintext);

        // Same key + nonce + plaintext = same ciphertext
        assert_eq!(ct1.to_bytes(), ct2.to_bytes());
    }

    // ========================================================================
    // WrappedKey Tests
    // ========================================================================

    #[test]
    fn wrap_unwrap_roundtrip() {
        let wrapping_key = EncryptionKey::generate();
        let original_key: [u8; KEY_LENGTH] = generate_random();

        let wrapped = WrappedKey::new(&wrapping_key, &original_key);
        let unwrapped = wrapped.unwrap_key(&wrapping_key).unwrap();

        assert_eq!(original_key, unwrapped);
    }

    #[test]
    fn wrapped_key_serialization_roundtrip() {
        let wrapping_key = EncryptionKey::generate();
        let original_key: [u8; KEY_LENGTH] = generate_random();

        let wrapped = WrappedKey::new(&wrapping_key, &original_key);
        let bytes = wrapped.to_bytes();

        // Simulate storing to disk and loading back
        let restored = WrappedKey::from_bytes(&bytes);
        let unwrapped = restored.unwrap_key(&wrapping_key).unwrap();

        assert_eq!(original_key, unwrapped);
    }

    #[test]
    fn wrapped_key_has_correct_length() {
        let wrapping_key = EncryptionKey::generate();
        let key_to_wrap: [u8; KEY_LENGTH] = generate_random();

        let wrapped = WrappedKey::new(&wrapping_key, &key_to_wrap);
        let bytes = wrapped.to_bytes();

        assert_eq!(bytes.len(), WRAPPED_KEY_LENGTH);
        assert_eq!(bytes.len(), NONCE_LENGTH + KEY_LENGTH + TAG_LENGTH);
    }

    #[test]
    fn wrong_wrapping_key_fails_unwrap() {
        let key1 = EncryptionKey::generate();
        let key2 = EncryptionKey::generate();
        let original: [u8; KEY_LENGTH] = generate_random();

        let wrapped = WrappedKey::new(&key1, &original);
        let result = wrapped.unwrap_key(&key2);

        assert!(result.is_err());
    }

    #[test]
    fn tampered_wrapped_key_fails_unwrap() {
        let wrapping_key = EncryptionKey::generate();
        let original: [u8; KEY_LENGTH] = generate_random();

        let wrapped = WrappedKey::new(&wrapping_key, &original);
        let mut bytes = wrapped.to_bytes();

        // Tamper with the ciphertext portion
        bytes[NONCE_LENGTH] ^= 0x01;

        let tampered = WrappedKey::from_bytes(&bytes);
        let result = tampered.unwrap_key(&wrapping_key);

        assert!(result.is_err());
    }

    #[test]
    fn different_keys_produce_different_wrapped_output() {
        let wrapping_key = EncryptionKey::generate();
        let key1: [u8; KEY_LENGTH] = generate_random();
        let key2: [u8; KEY_LENGTH] = generate_random();

        let wrapped1 = WrappedKey::new(&wrapping_key, &key1);
        let wrapped2 = WrappedKey::new(&wrapping_key, &key2);

        // Different plaintext keys = different ciphertexts
        assert_ne!(wrapped1.to_bytes(), wrapped2.to_bytes());
    }

    #[test]
    fn same_key_wrapped_twice_differs_due_to_random_nonce() {
        let wrapping_key = EncryptionKey::generate();
        let key_to_wrap: [u8; KEY_LENGTH] = generate_random();

        let wrapped1 = WrappedKey::new(&wrapping_key, &key_to_wrap);
        let wrapped2 = WrappedKey::new(&wrapping_key, &key_to_wrap);

        // Same key wrapped twice uses different random nonces
        assert_ne!(wrapped1.to_bytes(), wrapped2.to_bytes());

        // But both unwrap to the same key
        let unwrapped1 = wrapped1.unwrap_key(&wrapping_key).unwrap();
        let unwrapped2 = wrapped2.unwrap_key(&wrapping_key).unwrap();
        assert_eq!(unwrapped1, unwrapped2);
        assert_eq!(unwrapped1, key_to_wrap);
    }

    // ========================================================================
    // Key Hierarchy Tests
    // ========================================================================

    #[test]
    fn master_key_generate_and_restore() {
        let master = InMemoryMasterKey::generate();
        let bytes = master.to_bytes();

        let restored = InMemoryMasterKey::from_bytes(&bytes);

        // Both should wrap the same KEK identically (modulo random nonce)
        let kek_bytes: [u8; KEY_LENGTH] = generate_random();
        let wrapped1 = master.wrap_kek(&kek_bytes);
        let wrapped2 = restored.wrap_kek(&kek_bytes);

        // Different nonces, but both unwrap to same key
        let unwrapped1 = master.unwrap_kek(&wrapped1).unwrap();
        let unwrapped2 = restored.unwrap_kek(&wrapped2).unwrap();

        assert_eq!(unwrapped1, kek_bytes);
        assert_eq!(unwrapped2, kek_bytes);
    }

    #[test]
    fn kek_generate_and_restore() {
        let master = InMemoryMasterKey::generate();

        // Generate a new KEK
        let (kek, wrapped_kek) = KeyEncryptionKey::generate_and_wrap(&master);

        // Restore it from the wrapped form
        let restored_kek = KeyEncryptionKey::restore(&master, &wrapped_kek).unwrap();

        // Both should wrap the same DEK bytes
        let dek_bytes: [u8; KEY_LENGTH] = generate_random();
        let wrapped1 = kek.wrap_dek(&dek_bytes);
        let wrapped2 = restored_kek.wrap_dek(&dek_bytes);

        // Verify both can unwrap each other's wrapped keys
        let unwrapped1 = kek.unwrap_dek(&wrapped2).unwrap();
        let unwrapped2 = restored_kek.unwrap_dek(&wrapped1).unwrap();

        assert_eq!(unwrapped1, dek_bytes);
        assert_eq!(unwrapped2, dek_bytes);
    }

    #[test]
    fn dek_generate_and_restore() {
        let master = InMemoryMasterKey::generate();
        let (kek, _) = KeyEncryptionKey::generate_and_wrap(&master);

        // Generate a new DEK
        let (dek, wrapped_dek) = DataEncryptionKey::generate_and_wrap(&kek);

        // Restore it from the wrapped form
        let restored_dek = DataEncryptionKey::restore(&kek, &wrapped_dek).unwrap();

        // Both should encrypt/decrypt identically
        let nonce = Nonce::from_position(42);
        let plaintext = b"secret tenant data";

        let ciphertext = encrypt(dek.encryption_key(), &nonce, plaintext);
        let decrypted = decrypt(restored_dek.encryption_key(), &nonce, &ciphertext).unwrap();

        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn full_key_hierarchy_roundtrip() {
        // Master key (root of trust)
        let master = InMemoryMasterKey::generate();

        // KEK for tenant "acme"
        let (kek_acme, wrapped_kek_acme) = KeyEncryptionKey::generate_and_wrap(&master);

        // DEK for segment 0 of tenant "acme"
        let (dek_seg0, wrapped_dek_seg0) = DataEncryptionKey::generate_and_wrap(&kek_acme);

        // Encrypt some data
        let nonce = Nonce::from_position(0);
        let plaintext = b"acme's sensitive record";
        let ciphertext = encrypt(dek_seg0.encryption_key(), &nonce, plaintext);

        // --- Simulate restart: reload everything from wrapped forms ---

        // Restore KEK from wrapped form
        let restored_kek = KeyEncryptionKey::restore(&master, &wrapped_kek_acme).unwrap();

        // Restore DEK from wrapped form
        let restored_dek = DataEncryptionKey::restore(&restored_kek, &wrapped_dek_seg0).unwrap();

        // Decrypt the data
        let decrypted = decrypt(restored_dek.encryption_key(), &nonce, &ciphertext).unwrap();

        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn wrong_master_key_fails_kek_restore() {
        let master1 = InMemoryMasterKey::generate();
        let master2 = InMemoryMasterKey::generate();

        let (_, wrapped_kek) = KeyEncryptionKey::generate_and_wrap(&master1);

        // Try to restore with wrong master key
        let result = KeyEncryptionKey::restore(&master2, &wrapped_kek);

        assert!(result.is_err());
    }

    #[test]
    fn wrong_kek_fails_dek_restore() {
        let master = InMemoryMasterKey::generate();
        let (kek1, _) = KeyEncryptionKey::generate_and_wrap(&master);
        let (kek2, _) = KeyEncryptionKey::generate_and_wrap(&master);

        let (_, wrapped_dek) = DataEncryptionKey::generate_and_wrap(&kek1);

        // Try to restore with wrong KEK
        let result = DataEncryptionKey::restore(&kek2, &wrapped_dek);

        assert!(result.is_err());
    }

    #[test]
    fn tenant_isolation_via_kek() {
        let master = InMemoryMasterKey::generate();

        // Two tenants with different KEKs
        let (kek_tenant_a, _) = KeyEncryptionKey::generate_and_wrap(&master);
        let (kek_tenant_b, _) = KeyEncryptionKey::generate_and_wrap(&master);

        // Tenant A encrypts data
        let (dek_a, wrapped_dek_a) = DataEncryptionKey::generate_and_wrap(&kek_tenant_a);
        let nonce = Nonce::from_position(0);
        let _ciphertext_a = encrypt(dek_a.encryption_key(), &nonce, b"tenant A secret");

        // Tenant B cannot restore tenant A's DEK
        let result = DataEncryptionKey::restore(&kek_tenant_b, &wrapped_dek_a);
        assert!(result.is_err());

        // Even if somehow they got the wrapped DEK bytes, they can't decrypt
        // (This is guaranteed by the above test, but demonstrates the isolation)
    }

    #[test]
    fn wrapped_kek_serialization_roundtrip() {
        let master = InMemoryMasterKey::generate();
        let (_, wrapped_kek) = KeyEncryptionKey::generate_and_wrap(&master);

        // Serialize to bytes (for storage)
        let bytes = wrapped_kek.to_bytes();

        // Deserialize back
        let restored_wrapped = WrappedKey::from_bytes(&bytes);

        // Should still be unwrappable
        let kek = KeyEncryptionKey::restore(&master, &restored_wrapped).unwrap();

        // And should work for wrapping DEKs
        let dek_bytes: [u8; KEY_LENGTH] = generate_random();
        let wrapped_dek = kek.wrap_dek(&dek_bytes);
        let unwrapped = kek.unwrap_dek(&wrapped_dek).unwrap();

        assert_eq!(unwrapped, dek_bytes);
    }

    #[test]
    fn dek_encryption_key_reference() {
        let master = InMemoryMasterKey::generate();
        let (kek, _) = KeyEncryptionKey::generate_and_wrap(&master);
        let (dek, _) = DataEncryptionKey::generate_and_wrap(&kek);

        // Get reference to inner key
        let key_ref = dek.encryption_key();

        // Use it for encryption
        let nonce = Nonce::from_position(1);
        let ciphertext = encrypt(key_ref, &nonce, b"test");

        // And decryption
        let plaintext = decrypt(key_ref, &nonce, &ciphertext).unwrap();

        assert_eq!(plaintext, b"test");
    }
}
