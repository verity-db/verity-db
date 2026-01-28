# VerityDB Implementation Plan

## Overview

**VerityDB** is a compliance-first system of record designed for any industry where data integrity and verifiable correctness are critical. Built on a single architectural principle: **all data is an immutable, ordered log; all state is a derived view**.

In an era of increasing regulatory scrutiny, VerityDB provides a provable source of truth and a secure way to share that truth with trusted third parties.

Inspired by TigerBeetle's approach to financial transactions, VerityDB prioritizes correctness and auditability over flexibility and convenience.

### Why VerityDB?

VerityDB is built for industries where proving the integrity of your data is non-negotiable—healthcare, legal, government, finance, or any regulated field. VerityDB ensures that your data is not just stored—it's verifiably correct.

### Core Principles

- **Immutable by Design**: Every piece of data is stored in an append-only log, ensuring an immutable history of changes.
- **Verifiable History**: Use cryptographic proofs to verify that a given state matches a specific sequence of events.
- **Secure Data Sharing**: First-party support for securely sharing data with third-party services while protecting sensitive information.
- **Flexible Consistency Guarantees**: Choose the level of consistency that fits your regulatory needs: eventual, causal, or linearizable.
- **Compliance-First Architecture**: Compliance is not an add-on; it's the foundation of how VerityDB is designed.

**Core Invariant**:
```
One ordered log → Deterministic apply → Snapshot state
```

---

## Key Architectural Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| **Kernel threading** | Single-threaded | Deterministic execution, no synchronization overhead, enables DST |
| **I/O layer** | mio (not tokio) | Explicit control flow, custom event loop, enables simulation testing |
| **Testing strategy** | VOPR-style DST first | Build simulation harness before VSR; every line of consensus tested under faults |
| **Storage backend** | Single optimized implementation | Simpler to verify, smaller attack surface, TigerBeetle approach |
| **Developer experience** | Hybrid (tables + events) | Tables by default, events underneath, custom projections opt-in |
| **Query SQL** | Minimal subset | SELECT, WHERE (=,<,>,IN), ORDER BY, LIMIT - queries are lookups |
| **Projection SQL** | JOINs/aggregates allowed | Computed at write time, not query time |
| **Cryptography** | SHA-256/BLAKE3 + Ed25519 + AES-256-GCM | SHA-256 for compliance paths, BLAKE3 for internal hot paths |
| **Nonce derivation** | Position-based (not random) | Cryptographically sound, prevents nonce reuse at high throughput |
| **Memory security** | zeroize for key material | Secure clearing prevents key extraction from memory |
| **Checkpoint signing** | Ed25519 + Merkle roots | Tamper-evident sealing every 10k-100k events |
| **Wire protocol** | Custom binary protocol | Like TigerBeetle/Iggy, maximum control |
| **Secure data sharing** | Anonymization + field encryption + audit | Enable safe third-party/LLM access to sensitive data |
| **Hardware crypto** | target-cpu=native | Automatic AES-NI/ARMv8 crypto for 10-20x encryption speedup |
| **BLAKE3 parallelism** | rayon for >128 KiB | 3-5x improvement for internal hashing (Merkle trees, snapshots) |
| **fsync strategy** | Configurable group commit | 10-100x write throughput via batched durability |
| **Read optimization** | Sparse index + checkpoints | O(1) random reads, verified reads from checkpoints |
| **Parallelism model** | Tenant-level | Linear scaling; kernel stays single-threaded for DST |
| **Benchmark framework** | Criterion + hdrhistogram | CI regression detection, p50/p90/p99/p999 tracking |
| **Memory allocation** | Static at startup | Predictable latency, no OOM surprises (TigerBeetle-inspired) |
| **Idempotency** | Transaction-level IDs | Prevent duplicate writes on retry (FoundationDB-inspired) |
| **Recovery tracking** | Generation-based | Explicit data loss logging for compliance (FoundationDB-inspired) |
| **Metadata durability** | Superblock (4 copies) | Atomic metadata updates, survives partial writes (TigerBeetle-inspired) |

---

## Architecture

```
┌─────────────────────────────────────────────────────────────────────────┐
│                              VerityDB                                    │
│                                                                          │
│  ┌──────────────────────────────────────────────────────────────────┐   │
│  │                         Client Layer                              │   │
│  │   vdb (SDK)    vdb-client (RPC)    vdb-admin (CLI)               │   │
│  └───────────────────────────┬──────────────────────────────────────┘   │
│                              │                                           │
│                              ▼                                           │
│  ┌──────────────────────────────────────────────────────────────────┐   │
│  │                       Protocol Layer                              │   │
│  │        vdb-wire (binary protocol)    vdb-server (daemon)         │   │
│  └───────────────────────────┬──────────────────────────────────────┘   │
│                              │                                           │
│                              ▼                                           │
│  ┌──────────────────────────────────────────────────────────────────┐   │
│  │                    Data Sharing Layer                             │   │
│  │   vdb-sharing (export/anonymize)    vdb-mcp (LLM integration)    │   │
│  └───────────────────────────┬──────────────────────────────────────┘   │
│                              │                                           │
│                              ▼                                           │
│  ┌──────────────────────────────────────────────────────────────────┐   │
│  │                      Coordination Layer                           │   │
│  │   vdb-runtime (orchestrator)    vdb-directory (placement)        │   │
│  └───────────────────────────┬──────────────────────────────────────┘   │
│                              │                                           │
│                              ▼                                           │
│  ┌──────────────────────────────────────────────────────────────────┐   │
│  │                         Core Layer                                │   │
│  │                                                                   │   │
│  │   vdb-kernel        vdb-vsr         vdb-query      vdb-store     │   │
│  │   (state machine)   (consensus)     (SQL parser)   (B+tree)      │   │
│  └───────────────────────────┬──────────────────────────────────────┘   │
│                              │                                           │
│                              ▼                                           │
│  ┌──────────────────────────────────────────────────────────────────┐   │
│  │                      Foundation Layer                             │   │
│  │   vdb-types (IDs)    vdb-crypto (hashing)    vdb-storage (log)   │   │
│  └──────────────────────────────────────────────────────────────────┘   │
│                                                                          │
└─────────────────────────────────────────────────────────────────────────┘
```

---

## Crate Structure

### Foundation Layer (Active)

| Crate | Status | Purpose |
|-------|--------|---------|
| `vdb` | ✅ Active | Main facade crate, re-exports foundation types |
| `vdb-types` | ✅ Active | Core type definitions (IDs, offsets, positions) |
| `vdb-crypto` | ✅ Active | Cryptographic primitives (SHA-256 compliance, BLAKE3 internal) |
| `vdb-storage` | ✅ Active | Append-only log with CRC32 checksums (sync I/O) |
| `vdb-kernel` | ✅ Active | Pure functional state machine (Command → State + Effects) |
| `vdb-directory` | ✅ Active | Placement routing, tenant-to-shard mapping |
| `vdb-sim` | ✅ Active | VOPR simulation harness for deterministic testing |
| `vdb-vsr` | ✅ Active | Viewstamped Replication consensus |

### Planned Crates (Future Phases)

| Crate | Phase | Purpose |
|-------|-------|---------|
| `vdb-store` | Phase 4 | B+tree projection store with MVCC |
| `vdb-query` | Phase 5 | SQL subset parser and executor |
| `vdb-wire` | Phase 7 | Binary wire protocol definitions |
| `vdb-server` | Phase 7 | RPC server daemon |
| `vdb-client` | Phase 7 | Low-level RPC client |
| `vdb-admin` | Phase 7 | CLI administration tool |
| `vdb-sharing` | Phase 8 | Secure data export, anonymization, scoped access tokens |
| `vdb-mcp` | Phase 9 | MCP server for LLM/third-party integrations |

---

## Implementation Phases

### Phase 1: Foundation (Crypto & Storage) ✓ COMPLETE

**Goal**: Complete crypto primitives, enhance storage layer

**Completed**:
- [x] Delete SQLite-based `vdb-projections` crate
- [x] Remove SQLite/sqlx dependencies from workspace
- [x] Clean up `vdb-types` (remove sqlx derive)
- [x] Update PLAN.md with architecture decisions
- [x] Create comprehensive documentation (`/docs`)
- [x] Set up idiomatic Rust project structure (rustfmt.toml, .editorconfig, clippy lints)
- [x] Remove tokio dependency, convert to synchronous I/O (mio transition prep)
- [x] Clean up stub crates (vdb-vsr, vdb-runtime, vdb-wire, vdb-server, vdb-client, vdb-admin)
- [x] Implement hash chain in `vdb-crypto` (`chain_hash(prev, data) -> ChainHash`) - migrating to SHA-256
- [x] Implement AES-256-GCM envelope encryption with position-derived nonces
- [x] Implement three-tier key hierarchy (MasterKey → KEK per tenant → DEK per segment)
- [x] Implement key wrapping for secure key storage
- [x] Implement `zeroize` for secure memory clearing of key material
- [x] Implement Ed25519 signature support (FIPS 186-5 compliant)
- [x] Create `MasterKeyProvider` trait for future HSM integration
- [x] Implement dual-hash cryptography strategy (SHA-256 compliance + BLAKE3 internal)
- [x] Document cryptographic boundaries in COMPLIANCE.md and ARCHITECTURE.md

**Next**:
- [x] Extend `vdb-storage` with hash chains
  - [x] Add `prev_hash` field to Record
  - [x] Implement verified reads from genesis
  - [x] Add `OffsetIndex` data structure with persistence
  - [x] Integrate `OffsetIndex` into `Storage` for O(1) lookups
  - [x] Add checkpoint support
    - [x] Add `RecordKind` to Record (Data, Checkpoint, Tombstone)
    - [x] Add `CheckpointPayload` serialization
    - [x] Add `CheckpointIndex` for sparse checkpoint lookup
    - [x] Add checkpoint-optimized verified reads
- [x] Extend `vdb-types` with foundation types
  - [x] Add `Hash` type (32-byte cryptographic hash wrapper)
  - [x] Add `Timestamp` with monotonic wall-clock guarantee
  - [x] Add `RecordHeader` (offset, prev_hash, timestamp, payload_len, record_kind)
  - [x] Add `RecordKind` enum (Data, Checkpoint, Tombstone)
  - [x] Add `AppliedIndex` for projection tracking (offset + hash for verification)
  - [x] Add `Checkpoint` type (offset, chain_hash, record_count, created_at)
  - [x] Add `CheckpointPolicy` (every_n_records, on_shutdown, explicit_only)
  - [x] Add `IdempotencyId` for duplicate transaction prevention (16-byte unique identifier)
  - [x] Add `Generation` and `RecoveryRecord` for recovery tracking

### Phase 1 Detailed Implementation Plan

#### Step 1: Foundation Types (vdb-types)

**Timestamp with Monotonic Guarantee**:
```rust
/// Wall-clock timestamp with monotonic guarantee within the system.
/// Compliance requires real-world time; monotonicity prevents ordering issues.
pub struct Timestamp(u64);  // Nanoseconds since Unix epoch

impl Timestamp {
    /// Create timestamp ensuring monotonicity: max(now, last + 1ns)
    pub fn now_monotonic(last: Option<Timestamp>) -> Timestamp;
}
```

**RecordHeader** (metadata for every log entry):
```rust
pub struct RecordHeader {
    pub offset: Offset,           // Position in log (0-indexed)
    pub prev_hash: Hash,          // SHA-256 link to previous record
    pub timestamp: Timestamp,     // When committed (monotonic wall-clock)
    pub payload_len: u32,         // Payload size in bytes
    pub record_kind: RecordKind,  // Data vs Checkpoint vs Tombstone
}

pub enum RecordKind {
    Data,       // Normal application record
    Checkpoint, // Periodic verification anchor
    Tombstone,  // Logical deletion marker
}
```

**AppliedIndex** (what projections embed):
```rust
/// Tracks which log entry a projection row was derived from.
/// Includes hash to enable verification without walking the chain.
pub struct AppliedIndex {
    pub offset: Offset,
    pub hash: Hash,  // Hash at this offset for direct verification
}
```

**Checkpoint** (periodic verification anchors):
```rust
pub struct Checkpoint {
    pub offset: Offset,           // Log position of this checkpoint
    pub chain_hash: Hash,         // Cumulative hash at this point
    pub record_count: u64,        // Total records from genesis
    pub created_at: Timestamp,    // When checkpoint was created
}

pub struct CheckpointPolicy {
    pub every_n_records: u64,     // Create checkpoint every N records (e.g., 1000)
    pub on_shutdown: bool,        // Create checkpoint on graceful shutdown
    pub explicit_only: bool,      // Disable automatic checkpoints
}
```

**IdempotencyId** (prevents duplicate transactions on retry):
```rust
/// Unique identifier per transaction for duplicate detection.
/// Client generates before first attempt; retries use same ID.
/// Inspired by FoundationDB's idempotency key design.
pub struct IdempotencyId([u8; 16]);

impl IdempotencyId {
    /// Generate a new random idempotency ID
    pub fn generate() -> Self;

    /// Create from raw bytes (for client retry)
    pub fn from_bytes(bytes: [u8; 16]) -> Self;
}
```

**Generation and RecoveryRecord** (recovery tracking for compliance):
```rust
/// Monotonically increasing recovery generation.
/// Each recovery creates a new generation with explicit transition record.
/// Inspired by FoundationDB's 9-phase recovery with explicit data loss tracking.
pub struct Generation(u64);

/// Records a recovery event with explicit tracking of any data loss.
/// Critical for compliance: auditors can see exactly what happened during recovery.
pub struct RecoveryRecord {
    /// New generation after recovery
    pub generation: Generation,
    /// Previous generation before recovery
    pub previous_generation: Generation,
    /// Last known committed offset
    pub known_committed: Offset,
    /// Recovery point offset
    pub recovery_point: Offset,
    /// Range of discarded prepares (if any) - EXPLICIT LOSS TRACKING
    pub discarded_range: Option<Range<Offset>>,
    /// When recovery occurred
    pub timestamp: Timestamp,
    /// Why recovery was triggered
    pub reason: RecoveryReason,
}

pub enum RecoveryReason {
    NodeRestart,
    QuorumLoss,
    CorruptionDetected,
    ManualIntervention,
}
```

#### Step 2: Offset Index (vdb-storage)

**Design**: Persisted index file with CRC protection.

```
data.vlog      <- append-only log (exists)
data.vlog.idx  <- offset index (new)
```

**Structure**:
```rust
/// Maps offset → byte position for O(1) lookups.
/// Persisted alongside log; rebuildable from log if corrupted.
pub struct OffsetIndex {
    positions: Vec<u64>,  // index = offset, value = byte position
    checksum: Crc32,      // Integrity check
}

impl OffsetIndex {
    /// Called after each log append
    pub fn append(&mut self, byte_position: u64);

    /// O(1) lookup
    pub fn lookup(&self, offset: Offset) -> Option<u64>;

    /// Recovery path: rebuild from log scan
    pub fn rebuild_from_log(log: &Log) -> Self;

    /// Persistence
    pub fn persist(&self, path: &Path) -> Result<()>;
    pub fn load(path: &Path) -> Result<Self>;
}
```

**Startup behavior**:
1. Try loading index file
2. Validate CRC checksum
3. If invalid/missing, rebuild from log (warn once)

#### Step 3: Checkpoints (vdb-storage)

**Design**: Checkpoints are records IN the log (not separate).

This means:
- Checkpoints are part of the hash chain (tamper-evident)
- Checkpoint history is immutable
- Single source of truth

**Checkpoint as log record**:
```rust
// RecordKind::Checkpoint payload
pub struct CheckpointPayload {
    pub chain_hash: Hash,
    pub record_count: u64,
}
```

**Checkpoint index** (in-memory, derived):
```rust
/// Sparse index of checkpoint offsets for fast lookup.
/// Rebuilt on startup by scanning RecordKind::Checkpoint entries.
pub struct CheckpointIndex {
    checkpoints: Vec<Offset>,  // Sorted checkpoint positions
}

impl CheckpointIndex {
    /// Find nearest checkpoint at or before offset
    pub fn find_nearest(&self, offset: Offset) -> Option<Offset>;
}
```

**Verified reads with checkpoints**:
```
Before: Read offset 5000 → verify 5000 → 4999 → ... → 0 (genesis)
        O(n) hash checks

After:  Read offset 5000 → find checkpoint at 4500
        → verify 5000 → 4999 → ... → 4500 (stop at checkpoint)
        O(500) hash checks
```

**Default checkpoint policy**:
- `every_n_records: 1000` (bounds worst-case verification)
- `on_shutdown: true` (fast startup verification)
- `explicit_only: false` (automatic by default)

### Phase 1.5: Data Sharing Foundation (NEW)

**Goal**: Design and implement core anonymization primitives for secure data sharing

**Crypto Primitives**:
- [x] Field-level encryption support in `vdb-crypto`
- [x] Deterministic encryption for tokenization (HMAC-based)
- [x] Key hierarchy for field-level keys (master → tenant → field)

**Anonymization Core**:
- [x] Redaction: Field removal/masking utilities
- [x] Generalization: Value bucketing (age ranges, date truncation, geographic generalization)
- [x] Pseudonymization: Consistent tokenization with reversibility option

**Design Documents**:
- [ ] Token-based access control model specification
- [ ] Consent/purpose tracking schema
- [ ] Export audit trail format

---

## Performance Architecture

VerityDB achieves **"Compliance without the cost of performance"** through four pillars. The guiding principle: Fast, Correct, Safe — choose 3.

### Pillar 1: Crypto Performance

**Hardware Acceleration**:
- Enable AES-NI and ARMv8 crypto extensions via `.cargo/config.toml`:
  ```toml
  [target.'cfg(any(target_arch = "x86_64", target_arch = "aarch64"))']
  rustflags = ["-Ctarget-cpu=native"]
  ```
- Provides 10-20x encryption throughput improvement
- No code changes needed — crates auto-detect hardware

**BLAKE3 Parallel Hashing**:
- Add `internal_hash_parallel()` using `blake3::Hasher::update_rayon()`
- Threshold: 128 KiB (below this, single-threaded is faster)
- Use for: Merkle tree construction, snapshot hashing, content dedup
- Target: 5 GB/s single-threaded, 15+ GB/s parallel (4+ cores)

**Batched Encryption**:
- Amortize AES key schedule across multiple small records
- Add `batch_encrypt()` API for records < 4 KiB

### Pillar 2: I/O Performance

**Group Commit**:
- Implement `SyncPolicy` enum with configurable durability:
  - `EveryRecord`: fsync per record (~1K TPS, safest)
  - `EveryBatch`: fsync per batch (~50K TPS, balanced)
  - `GroupCommit { max_delay }`: PostgreSQL-style (~100K TPS, fastest)
- Make durability an explicit, documented feature

**Sparse Offset Index**:
- Index every Nth record (e.g., every 1024)
- O(1) lookup to nearest index entry, then short scan
- Target: < 100μs random read latency

**Checkpoint Support**:
- Store `(position, hash, index_snapshot, signature)` periodically
- Enable verified reads without replaying from genesis
- Ed25519 signature for tamper-evident checkpoints

**Future: io_uring Abstraction**:
- Define `IoBackend` trait for I/O abstraction
- Implementations: `SyncIoBackend` (DST), `IoUringBackend` (Linux 5.6+)
- Reserve architecture now, implement when Linux support matures

### Pillar 3: Pipeline Architecture

**Tenant-Level Parallelism**:
- Different tenants processed on different cores
- Kernel remains single-threaded per tenant (deterministic)
- Linear scaling up to core count

**Stage Pipelining**:
```
Stage 1 (Parse)     →  Tenant A, B, C in parallel
Stage 2 (Kernel)    →  Apply sequentially per tenant (hash chain)
Stage 3 (Effects)   →  Storage, Crypto, Projections overlap
```
- Kernel Apply is sequential (hash chain dependency)
- Effect execution and projections can overlap with next batch

**Bounded Queues**:
- All inter-stage queues are bounded (unbounded = infinite latency)
- Backpressure prevents memory exhaustion under load

### Pillar 4: Profiling Infrastructure

**Benchmark Suite**:
- Create `vdb-bench` crate with Criterion benchmarks
- CI workflow to detect regressions (10-20% threshold)
- Track: crypto ops, storage ops, kernel apply, end-to-end TPS

**Latency Histograms**:
- Use `hdrhistogram` to track p50/p90/p99/p999
- Record latencies for: append, read, encrypt, hash
- Export via tracing spans and metrics endpoint

**Profiling Tools**:
- `just flamegraph` for visual hotspot analysis
- `just profile` for samply (Firefox Profiler UI)
- `just bench` for Criterion benchmarks

### Benchmark Targets

| Operation | Target | Notes |
|-----------|--------|-------|
| SHA-256 chain hash | 500 MB/s | Compliance path, FIPS 180-4 |
| BLAKE3 internal hash | 5 GB/s | Single-threaded |
| BLAKE3 parallel (1 MiB) | 15 GB/s | 4+ cores |
| AES-256-GCM encrypt | 2 GB/s | With AES-NI |
| Record serialize | 1M ops/s | Hot path |
| Append (fsync each) | 1K TPS | `SyncPolicy::EveryRecord` |
| Append (group commit) | 100K TPS | `SyncPolicy::GroupCommit(5ms)` |
| Random read | < 100μs | With offset index |
| Kernel apply | 500K ops/s | Pure state machine |

---

### Phase 2: Deterministic Simulation Testing

**Goal**: Build VOPR simulation harness before VSR implementation

- [x] Create `vdb-sim` crate (simulation harness)
  - [x] Simulated time (discrete event) - `SimClock` with nanosecond precision
  - [x] Event scheduling - `EventQueue` with priority ordering
  - [x] Deterministic RNG - `SimRng` with seed-based reproducibility
  - [x] Simulated network (message queues, partitions, delays) - `SimNetwork`
  - [x] Simulated storage (failure injection) - `SimStorage`
- [x] Implement invariant checkers
  - [x] Log consistency checker - `LogConsistencyChecker`
  - [x] Hash chain verifier - `HashChainChecker`
  - [x] Linearizability checker - `LinearizabilityChecker`
  - [x] Byte-for-byte replica consistency checker - `ReplicaConsistencyChecker`
- [x] Build VOPR binary
  - [x] Seed-based reproducibility
  - [x] Fault injection configuration
  - [ ] Shrinking for minimal reproductions (future enhancement)
- [x] Advanced fault injection (inspired by FoundationDB/TigerBeetle)
  - [x] Swizzle-clogging: randomly clog/unclog network to nodes - `SwizzleClogger`
  - [x] Gray failure injection: partially-failed nodes (slow, intermittent) - `GrayFailureInjector`
  - [x] Enhanced storage faults: distinguish "not seen" vs "seen but corrupt" - `StorageFaultInjector`

### Phase 3: Consensus (VSR) ← COMPLETE

**Goal**: Implement Viewstamped Replication with full simulation testing

- [x] Implement VSR protocol in `vdb-vsr`
  - [x] Normal operation (Prepare/PrepareOK/Commit)
  - [x] View changes (StartViewChange/DoViewChange/StartView)
  - [x] Repair mechanisms (log repair, state transfer)
  - [x] Nack protocol for truncating uncommitted ops
- [x] Protocol-Aware Recovery (PAR) - TigerBeetle-inspired
  - [x] Distinguish between "not seen" vs "seen but corrupt" prepares
  - [x] NACK quorum protocol: require 4+ of 6 replicas to confirm safe truncation
  - [x] Prevents truncating potentially-committed prepares on checksum failures
- [x] Transparent repair mechanism - TigerBeetle-inspired
  - [x] Every block identified by (address, checksum) pair
  - [x] Corrupted reads trigger automatic fetch from peer replica
  - [x] Physical repair (fetch block bytes) not logical repair (re-derive)
- [x] Generation-based recovery tracking - FoundationDB-inspired
  - [x] Each recovery creates new generation with explicit transition record
  - [x] Track `known_committed_version` vs `recovery_point`
  - [x] Log any discarded mutations explicitly for audit compliance
- [x] Superblock pattern for consensus metadata
  - [x] 4 physical copies for atomic metadata updates
  - [x] Hash-chain to previous version
  - [x] Survives up to 3 simultaneous copy corruptions
- [x] Idempotency tracking in kernel
  - [x] Track committed IdempotencyIds with (Offset, Timestamp)
  - [x] Provide "did this commit?" query for compliance
  - [x] Configurable cleanup policy (e.g., 24 hours minimum retention)
- [x] Test every line under simulation
  - [x] Node crashes and restarts
  - [x] Network partitions (symmetric and asymmetric)
  - [x] Message reordering, loss, and duplication
  - [x] Storage faults (bit flips, partial writes, disk full)
- [x] SingleNodeReplicator as degenerate case
- [x] Cryptographic checkpoint signatures
  - [x] Ed25519 signed Merkle roots every 10k-100k events
  - [x] Checkpoint structure: log_hash + projection_hash + timestamp + signature
  - [ ] Third-party attestation support (RFC 3161 TSA, blockchain anchoring)

### Phase 4: Custom Projection Store ← CURRENT

**Goal**: Build `vdb-store` with B+tree and MVCC

- [ ] Create `vdb-store` crate
- [ ] Implement page-based storage (4KB pages)
- [ ] Implement B+tree for primary key lookups
- [ ] Implement secondary indexes
- [ ] Implement MVCC for point-in-time queries

**API Target**:
```rust
pub trait ProjectionStore: Send + Sync {
    fn apply(&self, position: LogPosition, batch: WriteBatch) -> Result<()>;
    fn applied_position(&self) -> Result<LogPosition>;
    fn get(&self, key: &Key) -> Result<Option<Bytes>>;
    fn get_at(&self, key: &Key, position: LogPosition) -> Result<Option<Bytes>>;
    fn scan(&self, range: Range<Key>, limit: usize) -> Result<Vec<(Key, Bytes)>>;
}
```

### Phase 5: Query Layer

**Goal**: SQL subset parser and executor

- [ ] Create `vdb-query` crate
- [ ] Use `sqlparser` for parsing
- [ ] Support: SELECT, WHERE (=, <, >, IN), ORDER BY, LIMIT
- [ ] Query planner (index selection)
- [ ] Query executor (against projection store)

### Phase 6: SDK & Integration

**Goal**: User-facing API with tenant isolation

- [ ] Implement `Verity` struct in `vdb` crate
- [ ] Implement `TenantHandle`
- [ ] Wire runtime to vdb-store
- [ ] Implement apply loop (log → projection)

**API Target**:
```rust
pub struct Verity { /* ... */ }

impl Verity {
    pub fn tenant(&self, id: TenantId) -> TenantHandle;
}

impl TenantHandle {
    pub async fn execute(&self, sql: &str, params: &[Value]) -> Result<()>;
    pub async fn query(&self, sql: &str, params: &[Value]) -> Result<Rows>;
    pub async fn query_at(&self, sql: &str, position: LogPosition) -> Result<Rows>;
}
```

### Phase 7: Protocol & Server

**Goal**: Wire protocol and network server

- [ ] Define binary protocol in `vdb-wire`
- [ ] Implement server in `vdb-server`
- [ ] Implement client in `vdb-client`
- [ ] Implement CLI in `vdb-admin`

### Phase 8: Data Sharing Layer (NEW)

**Goal**: Secure data export and third-party sharing infrastructure

**Create `vdb-sharing` crate**:
- [ ] Scoped export generation (time-bound, field-limited)
- [ ] Token management (create, validate, revoke access tokens)
- [ ] Transformation pipeline (anonymize, pseudonymize, redact based on rules)
- [ ] Consent ledger (track what was shared, when, with whom, for what purpose)

**Export Capabilities**:
- [ ] Anonymize or pseudonymize data before export
- [ ] Encrypt sensitive fields so only authorized recipients can decrypt
- [ ] Complete audit of all data exports with cryptographic proof

**Access Control**:
- [ ] Time-bound export tokens (automatic expiration)
- [ ] Scope-limited access (specific tables, fields, date ranges)
- [ ] One-time use tokens for sensitive operations
- [ ] Query rewriting for automatic field redaction

### Phase 9: MCP Integration (NEW)

**Goal**: Enable secure LLM and third-party API access via MCP

**Create `vdb-mcp` crate**:
- [ ] MCP server implementation
- [ ] Tool definitions for query, export, verify
- [ ] Automatic scope enforcement based on access tokens
- [ ] Rate limiting and access controls

**Safety Features**:
- [ ] Query validation (prevent data exfiltration patterns)
- [ ] Differential privacy for statistical queries (future)
- [ ] Automatic PII detection and redaction
- [ ] Comprehensive access logging

### Phase 10: Bug Bounty Program

**Goal**: Launch public security research program with staged scope

**Stage 1: Foundation Bounty** (Post Phase 2)
- [ ] Scope: `vdb-crypto`, `vdb-storage` crates only
- [ ] Focus: Hash chain integrity, cryptographic primitives, storage correctness
- [ ] Bounty range: $500 - $5,000

**Stage 2: Consensus Bounty** (Post Phase 3 VOPR validation)
- [ ] Scope: Add `vdb-vsr`, `vdb-sim` crates
- [ ] Focus: Consensus safety, linearizability, data loss scenarios
- [ ] Bounty range: $1,000 - $20,000 (TigerBeetle-style consensus challenge)

**Stage 3: Full Bounty** (Post Phase 8)
- [ ] Scope: All crates, wire protocol, encryption, data sharing
- [ ] Focus: End-to-end security, MVCC isolation, authentication bypass
- [ ] Bounty range: $500 - $50,000

**Program Infrastructure**:
- [ ] Security policy (SECURITY.md)
- [ ] Responsible disclosure process
- [ ] HackerOne or similar platform integration
- [ ] Invariant documentation for researchers

See [docs/BUG_BOUNTY.md](docs/BUG_BOUNTY.md) for detailed program specification.

---

## Design Principles

### Functional Core / Imperative Shell (FCIS)
- **Core (pure)**: Commands → State + Effects. No IO, no clocks, deterministic.
- **Shell (impure)**: RPC, auth, VSR transport, storage IO.

### Make Illegal States Unrepresentable
- Use Rust's type system to prevent invalid states at compile time
- Enums over booleans, newtypes over primitives

### Parse, Don't Validate
- Validate at system boundaries, then use typed representations
- Once parsed, data is known-valid by construction

### Assertion Density (2+ per function)
- Preconditions, postconditions, invariants
- Assertions in pairs (at write site and read site)

### Explicit Control Flow
- No recursion (use loops with explicit bounds)
- Push ifs up, fors down
- Minimal abstractions

See [docs/VERITASERUM.md](docs/VERITASERUM.md) for complete coding standards.

---

## Design Inspirations

VerityDB draws architectural inspiration from two pioneering distributed systems. This section documents the patterns we've adopted and why.

### From FoundationDB

| Pattern | Description | VerityDB Application |
|---------|-------------|---------------------|
| **Idempotency IDs** | Transaction-level unique identifiers with commitment proof | `IdempotencyId` type prevents duplicate writes on retry; kernel tracks committed IDs |
| **Generation-Based Recovery** | 9-phase recovery with explicit data loss tracking | `Generation` and `RecoveryRecord` types; explicit logging of discarded prepares |
| **Trillion CPU-Hour Simulation** | Massive investment in deterministic testing | VOPR harness; swizzle-clogging; comprehensive fault injection |

**Key Insight**: FoundationDB's experience shows that explicit tracking of what might be lost during recovery is essential for compliance-critical systems. Their generation concept provides natural audit checkpoints.

### From TigerBeetle

| Pattern | Description | VerityDB Application |
|---------|-------------|---------------------|
| **Protocol-Aware Recovery (PAR)** | NACK protocol distinguishes "not seen" vs "seen but corrupt" | Safe truncation only with 4+ replica confirmation |
| **Transparent Repair** | Checksum-based automatic repair from healthy replicas | Physical (not logical) repair maintains consistency proofs |
| **Superblock (4 copies)** | Atomic metadata updates via copy rotation | Consensus metadata survives up to 3 simultaneous copy corruptions |
| **Static Memory Allocation** | All memory allocated at startup, no malloc after init | Predictable latency, natural backpressure, no OOM surprises |
| **Byte-for-Byte Replica Checkers** | Verify all caught-up replicas are identical | VOPR invariant checker for replica consistency |
| **Gray Failure Injection** | Simulate partially-failed nodes | Slow responses, partial writes, intermittent network modes |
| **Control/Data Plane Separation** | O(1) control decisions separate from O(N) data processing | Kernel batch selection vs batch application |

**Key Insight**: TigerBeetle's approach of making the system "boringly reliable" through exhaustive testing and conservative design aligns perfectly with VerityDB's compliance-first mission.

### Synthesis for Compliance

The combination of these patterns creates a system where:

1. **Duplicate transactions are impossible**: Idempotency IDs with commitment proofs
2. **Data loss is explicit**: Generation tracking logs any discarded mutations
3. **Corruption is self-healing**: Transparent repair from healthy replicas
4. **Recovery is safe**: PAR prevents truncating potentially-committed data
5. **Performance is predictable**: Static allocation, bounded queues, explicit capacity

---

## Documentation

| Document | Purpose |
|----------|---------|
| [ARCHITECTURE.md](docs/ARCHITECTURE.md) | System design and data flow |
| [VERITASERUM.md](docs/VERITASERUM.md) | Coding philosophy and standards |
| [TESTING.md](docs/TESTING.md) | Testing strategy and VOPR |
| [COMPLIANCE.md](docs/COMPLIANCE.md) | Audit trails and encryption |
| [PERFORMANCE.md](docs/PERFORMANCE.md) | Performance guidelines |
| [OPERATIONS.md](docs/OPERATIONS.md) | Deployment and operations |
| [DATA_SHARING.md](docs/DATA_SHARING.md) | Secure third-party data sharing |
| [BUG_BOUNTY.md](docs/BUG_BOUNTY.md) | Security research program specification |

---

## Verification

### Build & Test
```bash
cargo build --workspace
cargo test --workspace
cargo clippy --workspace -- -D warnings
```

### Demo Target (Post Phase 6)
```rust
use vdb::Verity;

let db = Verity::open("./data").await?;
let tenant = db.tenant(TenantId::new(1));

// Write via SQL
tenant.execute(
    "INSERT INTO patients (id, name) VALUES (?, ?)",
    &[1.into(), "John Doe".into()]
).await?;

// Query
let results = tenant.query(
    "SELECT * FROM patients WHERE id = ?",
    &[1.into()]
).await?;

// Point-in-time query
let position = LogPosition::new(12345);
let historical = tenant.query_at(
    "SELECT * FROM patients WHERE id = ?",
    position
).await?;
```

---

## Dependencies

```toml
[workspace.dependencies]
# Core
anyhow = "1"
thiserror = "2"
bytes = { version = "1", features = ["serde"] }
tracing = "0.1"

# Async (minimal, explicit control)
mio = "1"

# Cryptography (FIPS 140-3 compliant - no feature flags, one code path)
sha2 = "0.10"  # SHA-256 for hash chains (FIPS approved)
ed25519-dalek = "2"  # Ed25519 signatures (FIPS 186-5 approved)
aes-gcm = "0.10"  # AES-256-GCM encryption (FIPS approved)
zeroize = { version = "1", features = ["derive"] }  # Secure memory clearing
subtle = "2"  # Constant-time operations
getrandom = "0.2"  # OS CSPRNG

# Serialization
serde = { version = "1", features = ["derive"] }
serde_json = "1"

# SQL parsing
sqlparser = "0.60"

# Testing
proptest = "1"
tempfile = "3"
```
