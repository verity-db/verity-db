# VerityDB Architecture

This document describes the architecture of VerityDB, a compliance-first database designed for regulated industries. It explains the core invariants, component responsibilities, and data flow that make VerityDB uniquely suited for healthcare, finance, and other domains requiring provable correctness.

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
- **Meet regulations**: HIPAA, SOC2, and similar frameworks require auditability by default

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
| `vdb-crypto` | Cryptographic primitives (hash chains, signatures, encryption) | blake3, ed25519-dalek |
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
CREATE TABLE patients (
    id       BIGINT PRIMARY KEY,
    name     TEXT NOT NULL,
    dob      DATE,
    region   TEXT
);

-- DML appends to the log and updates projection
INSERT INTO patients (id, name, dob, region)
VALUES (1, 'John Doe', '1980-01-15', 'us-east');

-- Queries read from projection
SELECT * FROM patients WHERE id = 1;
```

Under the hood:
- `INSERT` becomes an event appended to the tenant's log
- The projection engine applies the event to update the `patients` projection
- `SELECT` reads from the materialized projection (B+tree)

### Level 2: Events (Audit/History)

When you need the audit trail, query the event stream directly:

```sql
-- See all events in a stream
SELECT * FROM __events
WHERE stream = 'patients'
ORDER BY position DESC
LIMIT 10;

-- Point-in-time reconstruction
SELECT * FROM patients AS OF POSITION 12345
WHERE id = 1;
```

Events are the source of truth. Tables are just a convenient view.

### Level 3: Custom Projections (Advanced)

For complex read models, define custom projections:

```sql
-- Projection with JOIN (computed at write time, not query time)
CREATE PROJECTION patient_summary AS
SELECT
    p.id,
    p.name,
    COUNT(v.id) as visit_count,
    MAX(v.date) as last_visit
FROM patients p
LEFT JOIN visits v ON v.patient_id = p.id
GROUP BY p.id, p.name;

-- Query the pre-computed view
SELECT * FROM patient_summary WHERE id = 1;
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
- **Prev Hash**: Blake3 hash of the previous record (32 bytes)
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

### Append Operation

Appending to the log is the only write operation:

```rust
impl Log {
    pub fn append(&mut self, event: Event) -> Result<LogPosition, StorageError> {
        // 1. Compute hash chain
        let prev_hash = self.last_hash();
        let hash = blake3::hash(&[&prev_hash, &event.to_bytes()]);

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
Event: INSERT INTO patients (id, name) VALUES (1, 'John')
       ↓
Projection: patients
       ↓
B+Tree: key=1 → {id: 1, name: "John"}
```

```
Event: UPDATE patients SET name = 'Jane' WHERE id = 1
       ↓
Projection: patients
       ↓
B+Tree: key=1 → {id: 1, name: "Jane"} (new version)
                {id: 1, name: "John"} (old version, with deleted_at set)
```

### Projection Definitions

Custom projections are defined with SQL:

```sql
CREATE PROJECTION patient_visits AS
SELECT
    p.id as patient_id,
    p.name as patient_name,
    COUNT(v.id) as total_visits,
    MAX(v.date) as last_visit_date
FROM patients p
LEFT JOIN visits v ON v.patient_id = p.id
GROUP BY p.id, p.name;
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
    "INSERT INTO patients (id, name) VALUES (?, ?)",
    params![1, "John Doe"]
).await?;

// Query
let results = tenant.query(
    "SELECT * FROM patients WHERE id = ?",
    params![1]
).await?;
```

### Level 2: Events + History

When you need audit capabilities:

```rust
// Get the event that created a row
let events = tenant.query(
    "SELECT * FROM __events WHERE stream = 'patients' AND data->>'id' = '1'"
).await?;

// Point-in-time query
let historical = tenant.query_at(
    "SELECT * FROM patients WHERE id = 1",
    LogPosition::new(12345)  // As of this position
).await?;

// Read your own writes
let position = tenant.insert(...).await?;
let result = tenant.query_at(
    "SELECT * FROM patients WHERE id = 1",
    position
).await?;  // Guaranteed to see the insert
```

### Level 3: Custom Projections

For complex read models:

```rust
// Define a projection via SQL
tenant.execute(
    "CREATE PROJECTION patient_summary AS
     SELECT p.id, p.name, COUNT(v.id) as visits
     FROM patients p
     LEFT JOIN visits v ON v.patient_id = p.id
     GROUP BY p.id, p.name"
).await?;

// Query the materialized view
let summary = tenant.query(
    "SELECT * FROM patient_summary WHERE id = 1"
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
    region: Region::UsEast,  // PHI stays in US-East
    retention: Duration::from_secs(86400 * 2555),  // 7 years
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
