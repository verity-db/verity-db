# VerityDB Architecture

This document describes the architecture of VerityDB, a compliance-first system of record designed for any industry where data integrity and verifiable correctness are critical. It explains the core invariants, component responsibilities, and data flow that make VerityDB uniquely suited for healthcare, finance, legal, government, and other domains requiring provable correctness.

---

## Table of Contents

1. [Introduction](#introduction)
2. [Core Invariant](#core-invariant)
3. [System Overview](#system-overview)
4. [Crate Structure](#crate-structure)
5. [Data Model](#data-model)
6. [The Log](#the-log)
7. [Consensus (VSR)](#consensus-vsr)
8. [Storage Engine](#storage-engine)
9. [Projection Engine](#projection-engine)
10. [Query Layer](#query-layer)
11. [Developer Experience](#developer-experience)
12. [Multitenancy](#multitenancy)
13. [Wire Protocol](#wire-protocol)
14. [Deployment Models](#deployment-models)

---

## Introduction

### What is VerityDB?

VerityDB is a compliance-native database built on a single architectural principle: **all data is an immutable, ordered log; all state is a derived view**.

This architecture, inspired by TigerBeetle's approach to financial transactions, makes VerityDB uniquely suited for environments where you must:

- **Prove what happened**: Every state can be reconstructed from first principles
- **Audit changes**: Every modification is logged, timestamped, and attributable
- **Withstand scrutiny**: Cryptographic hashes chain events for tamper evidence
- **Meet regulations**: HIPAA, SOC2, GDPR, 21 CFR Part 11, and similar frameworks require auditability by default

### What VerityDB Is Not

VerityDB is intentionally limited in scope:

- **Not a general-purpose SQL database**: We support a minimal SQL subset for lookups
- **Not an analytics engine**: Use a data warehouse for OLAP workloads
- **Not a message queue**: We're a system of record, not a message broker
- **Not Postgres-compatible**: Compatibility is not a goal

These limitations exist to maintain simplicity, auditability, and correctness.

---

## Core Invariant

Everything in VerityDB derives from a single invariant:

```
State = Apply(InitialState, Log)
```

Or more precisely:

```
One ordered log → Deterministic apply → Snapshot state
```

**Implications**:

1. **The log is the source of truth**. The log is not a write-ahead log for a database—it IS the database. State is just a cache.

2. **State is derived, not authoritative**. Projections (materialized views) can be rebuilt at any time by replaying the log from the beginning.

3. **Replay must be deterministic**. Given the same log and the same initial state, apply must produce identical state. No randomness, no clocks, no external dependencies in the apply function.

4. **Consensus before acknowledgment**. A write is not acknowledged until it is durably committed to the log and replicated to a quorum.

---

## System Overview

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

### Data Flow

1. **Client** sends a command (e.g., INSERT) via SDK or RPC
2. **Server** receives request, validates authentication and authorization
3. **Runtime** coordinates with consensus layer
4. **VSR** replicates command to quorum of nodes
5. **Log** durably stores the committed command
6. **Kernel** applies command to derive new state (pure function)
7. **Projections** materialize state for efficient queries
8. **Client** receives acknowledgment with position token

For reads:

1. **Client** sends query with consistency requirement
2. **Server** routes to appropriate projection
3. **Query** layer executes against projection store
4. **Results** returned to client

---

## Crate Structure

VerityDB is organized as a Cargo workspace with distinct crates:

### Foundation Layer

| Crate | Purpose | Dependencies |
|-------|---------|--------------|
| `vdb-types` | Core type definitions (IDs, offsets, positions) | minimal |
| `vdb-crypto` | Cryptographic primitives (SHA-256/BLAKE3 hashing, signatures, encryption) | sha2, blake3, ed25519-dalek, aes-gcm |
| `vdb-storage` | Append-only log implementation | vdb-types, vdb-crypto |

### Core Layer

| Crate | Purpose | Dependencies |
|-------|---------|--------------|
| `vdb-kernel` | Pure functional state machine (Command → State + Effects) | vdb-types |
| `vdb-vsr` | Viewstamped Replication consensus | vdb-types, vdb-storage |
| `vdb-store` | B+tree projection store with MVCC | vdb-types |
| `vdb-query` | SQL subset parser and executor | vdb-types, vdb-store, sqlparser |

### Coordination Layer

| Crate | Purpose | Dependencies |
|-------|---------|--------------|
| `vdb-runtime` | Orchestrates propose → commit → apply → execute | vdb-kernel, vdb-vsr, vdb-store |
| `vdb-directory` | Placement routing, tenant-to-shard mapping | vdb-types |

### Protocol Layer

| Crate | Purpose | Dependencies |
|-------|---------|--------------|
| `vdb-wire` | Binary wire protocol definitions | capnp |
| `vdb-server` | RPC server daemon | vdb-runtime, vdb-wire, mio |

### Client Layer

| Crate | Purpose | Dependencies |
|-------|---------|--------------|
| `vdb` | High-level SDK for applications | vdb-client |
| `vdb-client` | Low-level RPC client | vdb-wire |
| `vdb-admin` | CLI administration tool | vdb-client |

### Dependency Direction

Dependencies flow downward only. Foundation crates have no dependencies on higher layers:

```
Client Layer
    ↓
Protocol Layer
    ↓
Coordination Layer
    ↓
Core Layer
    ↓
Foundation Layer
```

This ensures the core logic (kernel, storage, crypto) can be tested in isolation.

---

## Data Model

VerityDB presents three levels of abstraction to developers, allowing them to work at the appropriate level of detail.

### Level 1: Tables (Default DX)

Most developers interact with VerityDB through a familiar table abstraction:

```sql
-- DDL defines the projection
CREATE TABLE records (
    id          BIGINT PRIMARY KEY,
    name        TEXT NOT NULL,
    created_at  TIMESTAMP,
    region      TEXT
);

-- DML appends to the log and updates projection
INSERT INTO records (id, name, created_at, region)
VALUES (1, 'Record Alpha', '2024-01-15 10:30:00', 'us-east');

-- Queries read from projection
SELECT * FROM records WHERE id = 1;
```

Under the hood:
- `INSERT` becomes an event appended to the tenant's log
- The projection engine applies the event to update the `records` projection
- `SELECT` reads from the materialized projection (B+tree)

### Level 2: Events (Audit/History)

When you need the audit trail, query the event stream directly:

```sql
-- See all events in a stream
SELECT * FROM __events
WHERE stream = 'records'
ORDER BY position DESC
LIMIT 10;

-- Point-in-time reconstruction
SELECT * FROM records AS OF POSITION 12345
WHERE id = 1;
```

Events are the source of truth. Tables are just a convenient view.

### Level 3: Custom Projections (Advanced)

For complex read models, define custom projections:

```sql
-- Projection with JOIN (computed at write time, not query time)
CREATE PROJECTION entity_summary AS
SELECT
    e.id,
    e.name,
    COUNT(a.id) as activity_count,
    MAX(a.timestamp) as last_activity
FROM entities e
LEFT JOIN activities a ON a.entity_id = e.id
GROUP BY e.id, e.name;

-- Query the pre-computed view
SELECT * FROM entity_summary WHERE id = 1;
```

Projections are maintained incrementally as events arrive, not computed at query time.

### Event Structure

Every event in the log has this structure:

```rust
struct Event {
    // Position in the log
    position: LogPosition,

    // Cryptographic chain
    prev_hash: Hash,
    hash: Hash,

    // Metadata
    tenant_id: TenantId,
    stream: StreamId,
    timestamp: Timestamp,
    caused_by: Option<EventId>,  // Correlation

    // Payload
    event_type: EventType,
    data: Bytes,

    // Integrity
    checksum: Crc32,
}
```

---

## The Log

The append-only log is the system of record. All state derives from it.

### Segment Structure

The log is divided into segments for manageability:

```
data/
├── log/
│   ├── 00000000.segment     # Segment 0: positions 0-999999
│   ├── 00000001.segment     # Segment 1: positions 1000000-1999999
│   ├── 00000002.segment     # Segment 2: current active segment
│   └── index.meta           # Segment index metadata
```

Each segment is a sequential file of records:

```
┌─────────────────────────────────────────────────────────────────┐
│ Segment File                                                     │
│                                                                  │
│  ┌──────────┬──────────┬──────────┬──────────┬─────────────┐   │
│  │ Record 0 │ Record 1 │ Record 2 │   ...    │  Record N   │   │
│  └──────────┴──────────┴──────────┴──────────┴─────────────┘   │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

### Record Format

Each record is self-describing and integrity-checked:

```
┌──────────────────────────────────────────────────────────────────┐
│ Record (on disk)                                                  │
│                                                                   │
│  ┌─────────┬──────────┬───────────┬──────────┬────────────────┐ │
│  │ Length  │ Checksum │ Prev Hash │ Metadata │     Data       │ │
│  │ (4 B)   │ (4 B)    │ (32 B)    │ (var)    │    (var)       │ │
│  └─────────┴──────────┴───────────┴──────────┴────────────────┘ │
│                                                                   │
└──────────────────────────────────────────────────────────────────┘
```

- **Length**: Total record size in bytes (u32)
- **Checksum**: CRC32 of the entire record (excluding length field)
- **Prev Hash**: SHA-256 hash of the previous record (32 bytes)
- **Metadata**: Position, tenant, stream, timestamp, event type
- **Data**: Application payload (serialized bytes)

### Hash Chaining

Every record includes the hash of the previous record, creating a tamper-evident chain:

```
Record N-1          Record N            Record N+1
┌─────────┐         ┌─────────┐         ┌─────────┐
│ ...     │         │ ...     │         │ ...     │
│ hash ───┼────────►│ prev    │         │ prev    │
│         │         │ hash ───┼────────►│ hash    │
└─────────┘         └─────────┘         └─────────┘
```

If any record is modified, all subsequent hashes become invalid.

### Hash Types and Algorithms

VerityDB uses two hash algorithms for different purposes:

| Type | Algorithm | Purpose | FIPS |
|------|-----------|---------|------|
| `ChainHash` | SHA-256 | Tamper-evident log chains, checkpoints, exports | Yes (180-4) |
| `InternalHash` | BLAKE3 | Content addressing, Merkle trees, deduplication | No |

**Selection by Purpose**:

```rust
match purpose {
    HashPurpose::Compliance => SHA-256,  // Audit trails, exports, proofs
    HashPurpose::Internal => BLAKE3,     // Dedup, Merkle trees, fingerprints
}
```

- The `chain_hash()` function always uses SHA-256 for compliance
- The `internal_hash()` function always uses BLAKE3 for performance
- All externally-verifiable data uses SHA-256 (FIPS-approved)

### Dual-Hash Performance Strategy

The dual-hash approach is a key performance optimization. BLAKE3 is 10x+ faster than SHA-256 and supports parallel hashing for large inputs:

| Hash | Algorithm | Use Case | Target Throughput |
|------|-----------|----------|-------------------|
| `ChainHash` | SHA-256 | Compliance paths, audit trails, exports | 500 MB/s |
| `InternalHash` | BLAKE3 | Content addressing, Merkle trees, dedup | 5+ GB/s (single), 15+ GB/s (parallel) |

**When to use each**:
- SHA-256 (`ChainHash`): Any data that leaves the system or appears in audit logs
- BLAKE3 (`InternalHash`): Internal operations where speed matters and external verification is not required

The `HashPurpose` enum enforces this boundary at compile time, making it impossible to accidentally use the wrong hash for a given purpose.

### Append Operation

Appending to the log is the only write operation:

```rust
impl Log {
    pub fn append(&mut self, event: Event) -> Result<LogPosition, StorageError> {
        // 1. Compute hash chain
        let prev_hash = self.last_hash();
        let hash = sha256(&[&prev_hash, &event.to_bytes()]);

        // 2. Create record
        let record = Record {
            prev_hash,
            hash,
            data: event,
            checksum: crc32(&record_bytes),
        };

        // 3. Write to segment
        let position = self.active_segment.append(&record)?;

        // 4. Fsync for durability
        self.active_segment.sync()?;

        // 5. Update in-memory state
        self.last_hash = hash;
        self.last_position = position;

        Ok(position)
    }
}
```

### Read Operations

Records can be read by position (O(1) via offset index) or scanned sequentially:

```rust
impl Log {
    /// Read a single record by position
    pub fn read(&self, position: LogPosition) -> Result<Record, StorageError>;

    /// Scan from a position forward
    pub fn scan(&self, from: LogPosition) -> impl Iterator<Item = Record>;

    /// Get current end position
    pub fn end_position(&self) -> LogPosition;
}
```

### Offset Index

The offset index enables O(1) random access to any record by position:

```
┌─────────────────────────────────────────────────────────────────┐
│ Offset Index                                                     │
│                                                                  │
│  Position    Byte Offset                                        │
│  ┌───────┬────────────┐                                         │
│  │   0   │     0      │                                         │
│  │   1   │   847      │                                         │
│  │   2   │  1523      │                                         │
│  │   3   │  2891      │                                         │
│  │  ...  │   ...      │                                         │
│  └───────┴────────────┘                                         │
│                                                                  │
│  Storage: data.vlog.idx (CRC-protected)                         │
│  Recovery: Rebuildable from log scan                            │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

**Design decisions**:
- **Persisted**: Avoids O(n) startup scan for large logs
- **CRC-protected**: Detects corruption, triggers rebuild
- **Derived data**: Can always be rebuilt from the log

**Startup behavior**:
1. Load index file and validate CRC
2. If valid, use directly (fast path)
3. If invalid or missing, rebuild from log scan (warn once)

### Checkpoints

Checkpoints are periodic verification anchors that bound the cost of verified reads:

```
┌─────────────────────────────────────────────────────────────────┐
│ Checkpoint Architecture                                          │
│                                                                  │
│  Log records:                                                    │
│  ┌────┬────┬────┬─────────────┬────┬────┬────┬─────────────┬──  │
│  │ D  │ D  │ D  │ CHECKPOINT  │ D  │ D  │ D  │ CHECKPOINT  │    │
│  │ 0  │ 1  │ 2  │     3       │ 4  │ 5  │ 6  │     7       │    │
│  └────┴────┴────┴─────────────┴────┴────┴────┴─────────────┴──  │
│                                                                  │
│  D = Data record                                                 │
│  CHECKPOINT = Verification anchor (in the hash chain)           │
│                                                                  │
│  Verified read at offset 6:                                      │
│  - Find nearest checkpoint: offset 3                            │
│  - Verify: 6 → 5 → 4 → 3 (stop at checkpoint)                   │
│  - O(3) instead of O(6) hash checks                             │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

**Design decisions**:
- **Checkpoints are log records**: Part of the hash chain, tamper-evident
- **Stored in-log**: Single source of truth, immutable history
- **Policy-driven**: Configurable frequency (default: every 1000 records)

**Checkpoint payload**:
```rust
struct Checkpoint {
    offset: Offset,           // Position in log
    chain_hash: Hash,         // Cumulative SHA-256 hash
    record_count: u64,        // Total records from genesis
    created_at: Timestamp,    // Wall-clock time
}
```

**Default policy** (tuned for compliance workloads):
- Create checkpoint every 1000 records
- Create checkpoint on graceful shutdown
- Bounds worst-case verification to 1000 hash checks

---

## Consensus (VSR)

VerityDB uses Viewstamped Replication (VSR) for consensus, the same protocol used by TigerBeetle.

### Why VSR?

- **Simplicity**: Fewer moving parts than Raft, easier to verify
- **Determinism**: All replicas process the same operations in the same order
- **Recovery**: Strong repair mechanisms for divergent replicas
- **Proven**: Battle-tested in production systems

### Cluster Topology

A VerityDB cluster consists of `2f + 1` replicas to tolerate `f` failures:

```
f=1 (3 replicas):  Can tolerate 1 failure
f=2 (5 replicas):  Can tolerate 2 failures

┌──────────┐     ┌──────────┐     ┌──────────┐
│ Replica  │     │ Replica  │     │ Replica  │
│   (P)    │     │   (B)    │     │   (B)    │
│  Leader  │     │ Backup   │     │ Backup   │
└──────────┘     └──────────┘     └──────────┘
      │               │                │
      └───────────────┼────────────────┘
                      │
                Consensus Group
```

### Normal Operation

1. **Client Request**: Client sends command to leader
2. **Prepare**: Leader assigns position, broadcasts Prepare to backups
3. **Prepare OK**: Backups acknowledge with PrepareOK
4. **Commit**: Leader receives quorum (f+1), broadcasts Commit
5. **Apply**: All replicas apply the committed command
6. **Reply**: Leader sends result to client

```
Client      Leader        Backup 1      Backup 2
  │           │              │              │
  │ Request   │              │              │
  ├──────────►│              │              │
  │           │   Prepare    │   Prepare    │
  │           ├─────────────►├─────────────►│
  │           │   PrepareOK  │   PrepareOK  │
  │           │◄─────────────┤◄─────────────┤
  │           │              │              │
  │           │   Commit     │   Commit     │
  │           ├─────────────►├─────────────►│
  │           │              │              │
  │   Reply   │              │              │
  │◄──────────┤              │              │
```

### View Changes

When the leader fails, the cluster elects a new leader:

1. **Timeout**: Backups detect leader failure via heartbeat timeout
2. **Start View Change**: Backup broadcasts StartViewChange
3. **Do View Change**: Replicas send their state to new leader
4. **Start View**: New leader begins accepting requests

View changes preserve all committed operations.

### Repair Mechanisms

VSR includes mechanisms to repair replicas that have diverged:

- **Log Repair**: Copy missing entries from healthy replicas
- **State Transfer**: For replicas far behind, transfer a snapshot + recent log
- **Nack Protocol**: Request retransmission of missed messages

### Single-Node Mode

For development and testing, VerityDB supports single-node operation:

```rust
// In single-node mode, VSR degenerates to:
// 1. Append to local log
// 2. Apply immediately
// 3. Return result

impl SingleNodeReplicator {
    fn propose(&mut self, command: Command) -> Result<Position> {
        let position = self.log.append(command)?;
        Ok(position)
    }
}
```

---

## Storage Engine

VerityDB uses a custom storage engine optimized for its specific access patterns.

### Design Principles

1. **Append-only log**: Primary storage, never modified
2. **B+tree projections**: Derived state for efficient lookups
3. **MVCC**: Concurrent readers without blocking writers
4. **Page-based**: 4KB aligned for efficient I/O

### Projection Store Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│ Projection Store                                                 │
│                                                                  │
│  ┌───────────────────────────────────────────────────────────┐  │
│  │ Page Cache                                                 │  │
│  │  ┌────────┐ ┌────────┐ ┌────────┐ ┌────────┐             │  │
│  │  │ Page 0 │ │ Page 7 │ │Page 12 │ │Page 99 │    ...      │  │
│  │  └────────┘ └────────┘ └────────┘ └────────┘             │  │
│  └───────────────────────────────────────────────────────────┘  │
│                              │                                   │
│                              ▼                                   │
│  ┌───────────────────────────────────────────────────────────┐  │
│  │ B+Tree Index                                               │  │
│  │                                                            │  │
│  │           ┌───────┐                                        │  │
│  │           │ Root  │                                        │  │
│  │           └───┬───┘                                        │  │
│  │        ┌──────┼──────┐                                     │  │
│  │        ▼      ▼      ▼                                     │  │
│  │     ┌────┐ ┌────┐ ┌────┐                                   │  │
│  │     │Int │ │Int │ │Int │   Internal Nodes                  │  │
│  │     └──┬─┘ └──┬─┘ └──┬─┘                                   │  │
│  │        │      │      │                                     │  │
│  │     ┌──┴──┐ ... ... ┌┴───┐                                 │  │
│  │     ▼     ▼         ▼    ▼                                 │  │
│  │  ┌────┐┌────┐    ┌────┐┌────┐                              │  │
│  │  │Leaf││Leaf│    │Leaf││Leaf│  Leaf Nodes (data)           │  │
│  │  └────┘└────┘    └────┘└────┘                              │  │
│  └───────────────────────────────────────────────────────────┘  │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

### MVCC (Multi-Version Concurrency Control)

Each row has multiple versions for point-in-time queries:

```rust
struct RowVersion {
    /// Log position when this version was created
    created_at: LogPosition,
    /// Log position when this version was deleted (or MAX if current)
    deleted_at: LogPosition,
    /// The actual row data
    data: Bytes,
}
```

Queries specify a position and see only versions visible at that position:

```rust
impl ProjectionStore {
    fn get_at(&self, key: &Key, position: LogPosition) -> Option<Row> {
        let versions = self.index.get(key)?;
        versions.iter()
            .find(|v| v.created_at <= position && position < v.deleted_at)
            .map(|v| v.data.clone())
    }
}
```

### Write Batch

Projection updates are applied atomically in batches:

```rust
pub struct WriteBatch {
    /// Log position this batch represents
    position: LogPosition,
    /// Operations to apply
    operations: Vec<WriteOp>,
}

pub enum WriteOp {
    Put { table: TableId, key: Key, value: Bytes },
    Delete { table: TableId, key: Key },
}
```

### Durability

Projection state is periodically checkpointed:

1. **Write to log**: All state changes go through the log first
2. **Apply to projection**: Projection store is updated in memory
3. **Checkpoint**: Periodically flush projection state to disk
4. **Recovery**: On restart, replay log from last checkpoint

---

## Projection Engine

The projection engine transforms log events into queryable state.

### Default Projections

Every table has a default projection that mirrors its current state:

```
Event: INSERT INTO records (id, name) VALUES (1, 'Alpha')
       ↓
Projection: records
       ↓
B+Tree: key=1 → {id: 1, name: "Alpha"}
```

```
Event: UPDATE records SET name = 'Beta' WHERE id = 1
       ↓
Projection: records
       ↓
B+Tree: key=1 → {id: 1, name: "Beta"} (new version)
                {id: 1, name: "Alpha"} (old version, with deleted_at set)
```

### Projection Definitions

Custom projections are defined with SQL:

```sql
CREATE PROJECTION entity_activities AS
SELECT
    e.id as entity_id,
    e.name as entity_name,
    COUNT(a.id) as total_activities,
    MAX(a.timestamp) as last_activity_date
FROM entities e
LEFT JOIN activities a ON a.entity_id = e.id
GROUP BY e.id, e.name;
```

### Incremental Application

Projections are updated incrementally, not recomputed:

```rust
impl ProjectionEngine {
    fn apply(&mut self, event: &Event) -> Result<()> {
        // Determine which projections are affected
        let affected = self.projections_for_stream(&event.stream);

        for projection in affected {
            // Compute delta (what changed)
            let delta = projection.compute_delta(event)?;

            // Apply delta to projection store
            self.store.apply_batch(event.position, delta)?;
        }

        Ok(())
    }
}
```

For aggregations, this means updating counts/sums incrementally:
- INSERT: Add to count, add to sum
- UPDATE: Adjust based on old vs new values
- DELETE: Subtract from count, subtract from sum

### Projection Consistency

Each projection tracks its applied position:

```rust
struct ProjectionState {
    /// Last log position applied to this projection
    applied_position: LogPosition,
    /// Projection store
    store: ProjectionStore,
}
```

A projection is consistent when its `applied_position` matches the requested query position.

### Applied Index in Projection Rows

Each projection row embeds an `AppliedIndex` that tracks which log entry created or last modified it:

```
┌─────────────────────────────────────────────────────────────────┐
│ Projection Row with AppliedIndex                                 │
│                                                                  │
│  Patient "P-1234":                                               │
│  ┌────────────────────────────────────────────────────────────┐ │
│  │  name: "Jane Doe"                                          │ │
│  │  dob: "1990-03-15"                                         │ │
│  │  ...                                                       │ │
│  │  applied_index: { offset: 847, hash: "a3f2c..." }         │ │
│  └────────────────────────────────────────────────────────────┘ │
│                                                                  │
│  The applied_index enables:                                      │
│  1. Projection-level verification (row → source log entry)      │
│  2. Causal consistency (read-your-writes)                       │
│  3. Compliance proof (this state came from this log entry)      │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

**AppliedIndex structure**:
```rust
pub struct AppliedIndex {
    pub offset: Offset,   // Log position that produced this row
    pub hash: Hash,       // Hash at that position (for direct verification)
}
```

**Why include the hash?** Including the hash allows verification without walking the chain:
1. Fetch the log record at `offset`
2. Check that its hash matches `applied_index.hash`
3. If match, the projection row is verified against its source

This is critical for compliance scenarios where you need to prove a specific projection state came from a specific log entry.

---

## Query Layer

VerityDB supports a minimal SQL subset optimized for lookups.

### Supported Operations

**DDL (Schema)**:
```sql
CREATE TABLE name (columns...)
DROP TABLE name
CREATE INDEX name ON table (columns)
CREATE PROJECTION name AS SELECT...
```

**DML (Write)**:
```sql
INSERT INTO table (columns) VALUES (values)
UPDATE table SET columns WHERE condition
DELETE FROM table WHERE condition
```

**Query (Read)**:
```sql
SELECT columns FROM table WHERE condition ORDER BY column LIMIT n
```

### WHERE Clause Support

| Operator | Example | Index Use |
|----------|---------|-----------|
| `=` | `WHERE id = 1` | Yes (point lookup) |
| `<`, `>`, `<=`, `>=` | `WHERE date > '2024-01-01'` | Yes (range scan) |
| `IN` | `WHERE status IN ('a', 'b')` | Yes (multiple lookups) |
| `AND` | `WHERE a = 1 AND b = 2` | Yes (if composite index) |
| `OR` | `WHERE a = 1 OR b = 2` | Partial |

### Query Execution

```rust
impl QueryExecutor {
    fn execute(&self, query: &Query, position: LogPosition) -> Result<ResultSet> {
        // 1. Parse SQL
        let ast = sqlparser::parse(&query.sql)?;

        // 2. Plan: choose index, determine scan range
        let plan = self.planner.plan(&ast)?;

        // 3. Execute against projection store at specified position
        let snapshot = self.store.snapshot_at(position)?;
        let results = plan.execute(&snapshot)?;

        // 4. Apply ORDER BY, LIMIT
        let results = self.apply_ordering(results, &ast)?;

        Ok(results)
    }
}
```

### What's NOT Supported

VerityDB intentionally excludes:

- **Subqueries**: Complex queries should use projections
- **Window functions**: Use analytics tools instead
- **HAVING**: Aggregate filtering is not supported
- **JOINs in queries**: JOINs are for projection definitions only

Rationale: Queries are lookups. Complex computation happens at write time via projections.

---

## Developer Experience

VerityDB offers three levels of abstraction:

### Level 1: Tables (Default)

For most use cases, work with tables:

```rust
use vdb::Verity;

let db = Verity::open("./data").await?;
let tenant = db.tenant(TenantId::new(1));

// Insert
tenant.execute(
    "INSERT INTO records (id, name) VALUES (?, ?)",
    params![1, "Record Alpha"]
).await?;

// Query
let results = tenant.query(
    "SELECT * FROM records WHERE id = ?",
    params![1]
).await?;
```

### Level 2: Events + History

When you need audit capabilities:

```rust
// Get the event that created a row
let events = tenant.query(
    "SELECT * FROM __events WHERE stream = 'records' AND data->>'id' = '1'"
).await?;

// Point-in-time query
let historical = tenant.query_at(
    "SELECT * FROM records WHERE id = 1",
    LogPosition::new(12345)  // As of this position
).await?;

// Read your own writes
let position = tenant.insert(...).await?;
let result = tenant.query_at(
    "SELECT * FROM records WHERE id = 1",
    position
).await?;  // Guaranteed to see the insert
```

### Level 3: Custom Projections

For complex read models:

```rust
// Define a projection via SQL
tenant.execute(
    "CREATE PROJECTION entity_summary AS
     SELECT e.id, e.name, COUNT(a.id) as activity_count
     FROM entities e
     LEFT JOIN activities a ON a.entity_id = e.id
     GROUP BY e.id, e.name"
).await?;

// Query the materialized view
let summary = tenant.query(
    "SELECT * FROM entity_summary WHERE id = 1"
).await?;
```

### Consistency Modes

Every query can specify its consistency requirement:

```rust
// Eventual: Read local state immediately (fastest)
tenant.query("SELECT ...")
    .consistency(Consistency::Eventual)
    .execute().await?;

// Causal: Read-your-writes guarantee
tenant.query("SELECT ...")
    .consistency(Consistency::Causal)
    .execute().await?;

// Linearizable: See all committed writes (slowest)
tenant.query("SELECT ...")
    .consistency(Consistency::Linearizable)
    .execute().await?;

// At Position: Read as of specific log position
tenant.query_at("SELECT ...", position).await?;
```

---

## Multitenancy

Multitenancy is a first-class concept in VerityDB.

### Tenant Isolation

Each tenant has:
- **Separate log partitions**: Tenant data is not interleaved
- **Separate projections**: Each tenant has independent B+trees
- **Separate encryption keys**: Per-tenant envelope encryption
- **Separate quotas**: Storage and throughput limits

```
┌─────────────────────────────────────────────────────────────────┐
│ Multi-Tenant VerityDB                                            │
│                                                                  │
│  ┌─────────────────┐ ┌─────────────────┐ ┌─────────────────┐    │
│  │   Tenant A      │ │   Tenant B      │ │   Tenant C      │    │
│  │                 │ │                 │ │                 │    │
│  │  ┌───────────┐  │ │  ┌───────────┐  │ │  ┌───────────┐  │    │
│  │  │    Log    │  │ │  │    Log    │  │ │  │    Log    │  │    │
│  │  └───────────┘  │ │  └───────────┘  │ │  └───────────┘  │    │
│  │  ┌───────────┐  │ │  ┌───────────┐  │ │  ┌───────────┐  │    │
│  │  │Projections│  │ │  │Projections│  │ │  │Projections│  │    │
│  │  └───────────┘  │ │  └───────────┘  │ │  └───────────┘  │    │
│  │                 │ │                 │ │                 │    │
│  │  Key: K_A       │ │  Key: K_B       │ │  Key: K_C       │    │
│  └─────────────────┘ └─────────────────┘ └─────────────────┘    │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

### Regional Placement

Tenants can be assigned to specific regions for compliance:

```rust
// Create tenant with regional constraint
db.create_tenant(TenantConfig {
    id: TenantId::new(1),
    region: Region::UsEast,  // Sensitive data stays in designated region
    retention: Duration::from_secs(86400 * 2555),  // 7 years (configurable)
})?;
```

The directory service routes requests to the correct region:

```rust
impl Directory {
    fn route(&self, tenant_id: TenantId) -> NodeSet {
        let placement = self.placements.get(&tenant_id)?;
        self.nodes_in_region(placement.region)
    }
}
```

### Per-Tenant Encryption

Each tenant's data is encrypted with a unique key:

```
┌────────────────────────────────────────────────────────────────┐
│ Encryption Hierarchy                                            │
│                                                                 │
│  ┌─────────────┐                                                │
│  │ Master Key  │  (HSM/KMS managed)                             │
│  └──────┬──────┘                                                │
│         │                                                       │
│         ▼                                                       │
│  ┌─────────────┐ ┌─────────────┐ ┌─────────────┐               │
│  │  KEK_A      │ │  KEK_B      │ │  KEK_C      │  Per-tenant   │
│  │  (wrapped)  │ │  (wrapped)  │ │  (wrapped)  │  key-encrypt  │
│  └──────┬──────┘ └──────┬──────┘ └──────┬──────┘  keys         │
│         │               │               │                       │
│         ▼               ▼               ▼                       │
│  ┌─────────────┐ ┌─────────────┐ ┌─────────────┐               │
│  │  DEK_A      │ │  DEK_B      │ │  DEK_C      │  Per-tenant   │
│  │  (data)     │ │  (data)     │ │  (data)     │  data-encrypt │
│  └─────────────┘ └─────────────┘ └─────────────┘  keys         │
│                                                                 │
└────────────────────────────────────────────────────────────────┘
```

---

## Wire Protocol

VerityDB uses a custom binary protocol for maximum control and efficiency.

### Design Principles

- **Fixed-size headers**: Predictable parsing, no buffering guesswork
- **Length-prefixed**: Messages are self-delimiting
- **Checksummed**: CRC32 for integrity
- **Versioned**: Protocol version in header for evolution

### Message Format

```
┌────────────────────────────────────────────────────────────────┐
│ Message Format                                                  │
│                                                                 │
│  ┌─────────┬─────────┬──────────┬──────────┬──────────────────┐│
│  │ Magic   │ Version │ Length   │ Checksum │     Payload      ││
│  │ (4 B)   │ (2 B)   │ (4 B)    │ (4 B)    │     (var)        ││
│  └─────────┴─────────┴──────────┴──────────┴──────────────────┘│
│                                                                 │
│  Magic: 0x56444220 ("VDB ")                                     │
│  Version: Protocol version (e.g., 1)                            │
│  Length: Payload length in bytes                                │
│  Checksum: CRC32 of payload                                     │
│  Payload: Cap'n Proto encoded message                           │
│                                                                 │
└────────────────────────────────────────────────────────────────┘
```

### Request/Response

```
Client                          Server
   │                              │
   │  Request (Execute/Query)     │
   ├─────────────────────────────►│
   │                              │
   │  Response (Result/Error)     │
   │◄─────────────────────────────┤
   │                              │
```

### Connection Lifecycle

1. **Connect**: TCP connection established
2. **Handshake**: Version negotiation, authentication
3. **Ready**: Connection ready for requests
4. **Request/Response**: Commands and queries
5. **Close**: Graceful shutdown

---

## Secure Data Sharing

VerityDB includes first-party support for securely sharing data with third-party services while protecting sensitive information.

### Data Sharing Layer

The data sharing layer sits between the protocol layer and the core database, intercepting and transforming data before it leaves the system:

```
┌──────────────────────────────────────────────────────────────────┐
│                    Data Sharing Layer                             │
│                                                                   │
│  ┌─────────────────────────────────────────────────────────────┐ │
│  │ Access Control                                               │ │
│  │ • Scoped tokens (time-bound, field-limited)                 │ │
│  │ • Purpose tracking (why is data being accessed?)            │ │
│  │ • Consent verification                                       │ │
│  └─────────────────────────────────────────────────────────────┘ │
│                              ↓                                    │
│  ┌─────────────────────────────────────────────────────────────┐ │
│  │ Transformation Pipeline                                      │ │
│  │ • Redaction: Remove sensitive fields entirely               │ │
│  │ • Generalization: Reduce precision (age → age range)        │ │
│  │ • Pseudonymization: Replace with consistent tokens          │ │
│  │ • Field encryption: Encrypt for specific recipients         │ │
│  └─────────────────────────────────────────────────────────────┘ │
│                              ↓                                    │
│  ┌─────────────────────────────────────────────────────────────┐ │
│  │ Audit                                                        │ │
│  │ • Log all data exports                                       │ │
│  │ • Track what was shared, when, with whom                    │ │
│  │ • Cryptographic proof of export contents                    │ │
│  └─────────────────────────────────────────────────────────────┘ │
│                                                                   │
└──────────────────────────────────────────────────────────────────┘
```

### Anonymization Techniques

VerityDB supports multiple anonymization strategies:

| Technique | Description | Use Case |
|-----------|-------------|----------|
| **Redaction** | Complete removal of sensitive fields | Untrusted third parties |
| **Generalization** | Reduce precision (exact date → year, zip → first 3 digits) | Research datasets |
| **Pseudonymization** | Replace with consistent tokens, reversible with key | Trusted partners, internal analytics |

### Token-Based Access

Third-party access is controlled via scoped tokens:

```rust
// Create a scoped access token
let token = tenant.create_access_token(AccessTokenConfig {
    // What can be accessed
    tables: vec!["records"],
    fields: vec!["id", "name", "region"],  // Excludes sensitive fields

    // How data is transformed
    transformations: vec![
        Transformation::Generalize("created_at", DatePrecision::Month),
    ],

    // Constraints
    expires_at: Timestamp::now() + Duration::hours(24),
    max_queries: Some(100),
    purpose: "Analytics integration",
})?;
```

### MCP Integration (Future)

VerityDB will provide an MCP server for LLM and AI agent access:

```rust
// MCP tools automatically enforce access controls
// Query tool: Reads with automatic redaction
// Export tool: Bulk exports with transformation
// Verify tool: Cryptographic proof verification
```

All MCP access is:
- Scoped by access token
- Automatically redacted based on token permissions
- Fully audited

---

## Idempotency & Duplicate Prevention

Network failures can cause clients to retry transactions without knowing if the original succeeded. Without idempotency tracking, this leads to duplicate records—a compliance violation in regulated industries.

### Idempotency ID Design

Every transaction includes a client-generated idempotency ID:

```rust
/// 16-byte unique identifier per transaction.
/// Client generates before first attempt; retries use same ID.
pub struct IdempotencyId([u8; 16]);

impl IdempotencyId {
    /// Generate a new random idempotency ID (client-side)
    pub fn generate() -> Self {
        let mut bytes = [0u8; 16];
        getrandom::getrandom(&mut bytes).expect("random bytes");
        Self(bytes)
    }
}
```

### Kernel Tracking

The kernel maintains a map of committed idempotency IDs:

```
┌─────────────────────────────────────────────────────────────────┐
│ Idempotency Tracker                                              │
│                                                                  │
│  ┌────────────────────────────────────────────────────────────┐ │
│  │  IdempotencyId      │  Offset  │  Timestamp                │ │
│  ├────────────────────────────────────────────────────────────┤ │
│  │  a1b2c3d4...        │  1234    │  2024-01-15 10:30:00     │ │
│  │  e5f6a7b8...        │  1235    │  2024-01-15 10:30:01     │ │
│  │  ...                │  ...     │  ...                      │ │
│  └────────────────────────────────────────────────────────────┘ │
│                                                                  │
│  Storage: ~1% overhead per FoundationDB measurements             │
│  Cleanup: Entries older than min_age (e.g., 24 hours) removed   │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

### Transaction Flow

1. **First attempt**: Client generates `IdempotencyId`, sends transaction
2. **Commit**: Kernel records `(IdempotencyId, Offset, Timestamp)` atomically with data
3. **Retry (network failure)**: Client retries with same `IdempotencyId`
4. **Duplicate detected**: Kernel returns existing `Offset` without re-applying

### Commitment Proof Query

Clients can query whether a transaction committed:

```rust
/// Query whether a specific idempotency ID was committed.
/// Returns (Offset, Timestamp) if committed, None otherwise.
pub fn query_commitment(&self, id: &IdempotencyId) -> Option<(Offset, Timestamp)>;
```

This is essential for compliance—clients can prove a transaction occurred without re-executing it.

---

## Transparent Repair

When a replica detects data corruption (via checksum mismatch), it automatically repairs from a healthy peer rather than failing.

### Repair Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│ Transparent Repair Flow                                          │
│                                                                  │
│  1. Read request at offset 1234                                  │
│     ┌────────────────────────────────────────────────────────┐  │
│     │  Storage.read(1234, expected_checksum: 0xABCD)         │  │
│     └────────────────────────────────────────────────────────┘  │
│                              │                                   │
│                              ▼                                   │
│  2. Checksum mismatch detected                                   │
│     ┌────────────────────────────────────────────────────────┐  │
│     │  Local: 0x1234  ≠  Expected: 0xABCD                    │  │
│     └────────────────────────────────────────────────────────┘  │
│                              │                                   │
│                              ▼                                   │
│  3. Fetch from healthy peer                                      │
│     ┌────────────────────────────────────────────────────────┐  │
│     │  Peer.fetch_block(offset: 1234, checksum: 0xABCD)      │  │
│     └────────────────────────────────────────────────────────┘  │
│                              │                                   │
│                              ▼                                   │
│  4. Verify and write repaired data                               │
│     ┌────────────────────────────────────────────────────────┐  │
│     │  Verify checksum, write to local storage               │  │
│     └────────────────────────────────────────────────────────┘  │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

### Physical vs Logical Repair

VerityDB uses **physical repair** (fetching exact bytes) rather than **logical repair** (re-deriving from log):

| Approach | Description | Trade-offs |
|----------|-------------|------------|
| **Physical** | Fetch exact bytes from peer | Maintains byte-for-byte consistency; simpler verification |
| **Logical** | Re-apply log to derive state | More complex; may introduce subtle divergence |

Physical repair ensures that all replicas remain byte-identical, which simplifies consistency verification.

### Block Identification

Every block is identified by an `(address, checksum)` pair:

```rust
pub struct BlockId {
    pub address: u64,     // Byte offset in file
    pub checksum: Crc32,  // Expected CRC32
}
```

This pair uniquely identifies a block version. If a peer has a different checksum at the same address, the blocks represent different data.

---

## Generation-Based Recovery

Recovery events create natural audit checkpoints with explicit tracking of what data might have been lost.

### Generation Concept

```
┌─────────────────────────────────────────────────────────────────┐
│ Generation Timeline                                               │
│                                                                  │
│  Gen 1                     Gen 2                    Gen 3       │
│  ┌──────────────────────┐  ┌──────────────────────┐  ┌────────  │
│  │ Normal operation     │  │ Normal operation     │  │ Normal   │
│  │ Log: 0 → 5000       │  │ Log: 5001 → 12000   │  │ Log: ... │
│  └──────────────────────┘  └──────────────────────┘  └────────  │
│            │                          │                          │
│            ▼                          ▼                          │
│     ┌─────────────┐            ┌─────────────┐                  │
│     │ Recovery    │            │ Recovery    │                  │
│     │ Record      │            │ Record      │                  │
│     │             │            │             │                  │
│     │ known: 4950 │            │ known: 11980│                  │
│     │ point: 5000 │            │ point: 12000│                  │
│     │ discarded:  │            │ discarded:  │                  │
│     │ 4951-5000   │            │ None        │                  │
│     └─────────────┘            └─────────────┘                  │
│                                                                  │
│     50 prepares discarded      Clean recovery                    │
│     (explicit loss record)     (no data loss)                    │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

### Recovery Record

Each recovery creates an explicit record:

```rust
pub struct RecoveryRecord {
    /// New generation after recovery
    pub generation: Generation,
    /// Previous generation before recovery
    pub previous_generation: Generation,
    /// Last offset known to be committed (durable acknowledgment sent)
    pub known_committed: Offset,
    /// Recovery point (where log continues)
    pub recovery_point: Offset,
    /// Range of discarded prepares (EXPLICIT LOSS TRACKING)
    pub discarded_range: Option<Range<Offset>>,
    /// When recovery occurred
    pub timestamp: Timestamp,
    /// Why recovery was triggered
    pub reason: RecoveryReason,
}
```

### Compliance Implications

The `discarded_range` field is critical for compliance:

1. **Audit trail**: Regulators can see exactly what data might have been lost
2. **Incident reporting**: Precise scope for data loss notifications
3. **Recovery verification**: Confirm that only uncommitted data was discarded
4. **Root cause analysis**: Timestamp and reason enable forensics

### Recovery Reasons

```rust
pub enum RecoveryReason {
    /// Normal node restart (clean shutdown)
    NodeRestart,
    /// Lost quorum, had to recover from remaining replicas
    QuorumLoss,
    /// Detected and recovered from storage corruption
    CorruptionDetected,
    /// Operator-initiated recovery
    ManualIntervention,
}
```

---

## Superblock Design

The superblock provides atomic, crash-safe updates to critical metadata using a 4-copy pattern.

### Structure

```
┌─────────────────────────────────────────────────────────────────┐
│ Superblock Layout (on disk)                                      │
│                                                                  │
│  ┌───────────────┐ ┌───────────────┐ ┌───────────────┐ ┌──────  │
│  │ Copy 0        │ │ Copy 1        │ │ Copy 2        │ │ Copy 3 │
│  │               │ │               │ │               │ │        │
│  │ version: 42   │ │ version: 42   │ │ version: 42   │ │ ver: 41│
│  │ prev_hash     │ │ prev_hash     │ │ prev_hash     │ │ prev   │
│  │ checkpoint    │ │ checkpoint    │ │ checkpoint    │ │ ckpt   │
│  │ manifest      │ │ manifest      │ │ manifest      │ │ mani   │
│  │ checksum      │ │ checksum      │ │ checksum      │ │ csum   │
│  └───────────────┘ └───────────────┘ └───────────────┘ └──────  │
│                                                                  │
│  Write order: 0 → 1 → 2 → 3 (with fsync between each)           │
│  Read: majority vote on version + checksum validation            │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

### Superblock Contents

```rust
pub struct Superblock {
    /// Monotonically increasing version number
    pub version: u64,
    /// Hash of previous superblock (chain for verification)
    pub prev_hash: Hash,
    /// Current checkpoint offset and hash
    pub checkpoint_offset: Offset,
    pub checkpoint_hash: Hash,
    /// VSR view number
    pub view: u64,
    /// Current generation
    pub generation: Generation,
    /// Free space management state
    pub free_set_checksum: Crc32,
    /// Checksum of this superblock
    pub checksum: Crc32,
}
```

### Atomic Update Protocol

1. **Prepare**: Compute new superblock with incremented version
2. **Write copies**: Write to copies 0, 1, 2, 3 in order (fsync each)
3. **Read verification**: Read back and verify all 4 copies match

### Crash Recovery

On startup, read all 4 copies and apply majority vote:

```rust
fn recover_superblock(copies: [Option<Superblock>; 4]) -> Superblock {
    // Group by (version, checksum) pair
    let mut votes: HashMap<(u64, Crc32), Vec<Superblock>> = HashMap::new();
    for copy in copies.into_iter().flatten() {
        if copy.verify_checksum() {
            votes.entry((copy.version, copy.checksum))
                .or_default()
                .push(copy);
        }
    }

    // Return highest version with 2+ valid copies
    votes.into_iter()
        .filter(|(_, v)| v.len() >= 2)
        .max_by_key(|((ver, _), _)| *ver)
        .map(|(_, v)| v[0].clone())
        .expect("at least one valid superblock pair required")
}
```

### Durability Guarantees

- **Survives 1 copy corruption**: 3 valid copies remain (majority)
- **Survives 2 copy corruptions**: 2 valid copies remain (tie-break by version)
- **Survives 3 copy corruptions**: 1 valid copy remains (degraded but recoverable)
- **All 4 corrupt**: System cannot start (catastrophic failure)

---

## Deployment Models

### Single-Node

For development, testing, and small deployments:

```
┌─────────────────────────────────────────┐
│            Single Node                   │
│                                          │
│  ┌────────────────────────────────────┐ │
│  │           vdb-server               │ │
│  │  ┌──────────┐ ┌──────────────────┐ │ │
│  │  │   Log    │ │   Projections    │ │ │
│  │  └──────────┘ └──────────────────┘ │ │
│  └────────────────────────────────────┘ │
│                                          │
│  Data: ./data/                           │
└─────────────────────────────────────────┘
```

- No consensus overhead
- Immediate consistency
- Data loss if node fails

### Multi-Node (3 or 5 replicas)

For production deployments requiring fault tolerance:

```
┌─────────────────────────────────────────────────────────────────┐
│                       3-Node Cluster                             │
│                                                                  │
│  ┌──────────────┐   ┌──────────────┐   ┌──────────────┐        │
│  │   Node 1     │   │   Node 2     │   │   Node 3     │        │
│  │   (Leader)   │   │   (Backup)   │   │   (Backup)   │        │
│  │              │   │              │   │              │        │
│  │  ┌────────┐  │   │  ┌────────┐  │   │  ┌────────┐  │        │
│  │  │  Log   │  │   │  │  Log   │  │   │  │  Log   │  │        │
│  │  └────────┘  │   │  └────────┘  │   │  └────────┘  │        │
│  │  ┌────────┐  │   │  ┌────────┐  │   │  ┌────────┐  │        │
│  │  │Project │  │   │  │Project │  │   │  │Project │  │        │
│  │  └────────┘  │   │  └────────┘  │   │  └────────┘  │        │
│  └──────────────┘   └──────────────┘   └──────────────┘        │
│         │                  │                  │                 │
│         └──────────────────┼──────────────────┘                 │
│                            │                                    │
│                     VSR Consensus                               │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

- Tolerates 1 failure (3 nodes) or 2 failures (5 nodes)
- Automatic leader election
- Consistent writes across replicas

### Multi-Region

For global deployments with regional data residency:

```
┌─────────────────────────────────────────────────────────────────────────┐
│                         Multi-Region Deployment                          │
│                                                                          │
│  ┌─────────────────────┐            ┌─────────────────────┐             │
│  │     US-East         │            │     EU-West         │             │
│  │                     │            │                     │             │
│  │  ┌───────────────┐  │            │  ┌───────────────┐  │             │
│  │  │ Cluster A     │  │            │  │ Cluster B     │  │             │
│  │  │ (US tenants)  │  │            │  │ (EU tenants)  │  │             │
│  │  └───────────────┘  │            │  └───────────────┘  │             │
│  │                     │            │                     │             │
│  └─────────────────────┘            └─────────────────────┘             │
│              │                                  │                        │
│              └──────────────┬───────────────────┘                        │
│                             │                                            │
│                      ┌──────────────┐                                    │
│                      │  Directory   │                                    │
│                      │  (routing)   │                                    │
│                      └──────────────┘                                    │
│                                                                          │
└─────────────────────────────────────────────────────────────────────────┘
```

- Tenant data stays in designated region
- Directory routes requests appropriately
- Cross-region queries not supported (by design)

---

## Summary

VerityDB's architecture is built on a single, powerful invariant: **the log is the source of truth**. This enables:

- **Provable correctness**: State is a pure function of the log
- **Complete audit trails**: Nothing happens without a log entry
- **Tamper evidence**: Cryptographic hash chains detect modifications
- **Flexible consistency**: Choose the right tradeoff per request
- **Point-in-time queries**: Any historical state is reconstructible

For more details, see:
- [VERITASERUM.md](VERITASERUM.md) - Coding philosophy and standards
- [TESTING.md](TESTING.md) - Testing strategy and simulation
- [COMPLIANCE.md](COMPLIANCE.md) - Audit and encryption architecture
- [PERFORMANCE.md](PERFORMANCE.md) - Performance optimization guidelines
- [OPERATIONS.md](OPERATIONS.md) - Deployment and operations guide
