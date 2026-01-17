# VerityDB Implementation Plan

## Overview

**VerityDB** is a compliance-native, healthcare-focused system-of-record database. It combines:
- High-performance event log with durable store
- Viewstamped Replication (VSR) for quorum-based HA
- Cap'n Proto for wire protocol (and event serialization)
- SQLCipher-encrypted SQLite projections
- First-class PHI placement controls (regional vs global)
- Audit-first design for HIPAA compliance

**Key differentiator**: "PHI stays regional by construction, non-PHI can be global."

**Dogfooding strategy**: VerityDB will be developed alongside Notebar (clinic management SaaS) to validate the architecture under real clinical workloads.

---

## Architecture Principles

### Functional Core / Imperative Shell
- **Kernel (pure)**: Commands → State + Effects. No IO, no clocks, deterministic.
- **Shell (impure)**: RPC, auth, VSR transport, storage IO, SQLite connections.

### Vertical Slices
Organize by domain capability, not horizontal layers:
- `streams` - create stream, append, retention
- `consumers` - fetch, ack, redelivery
- `projections` - create, apply, snapshots
- `policy` - PHI guard, RBAC, placement
- `audit` - immutable audit events

### Developer Experience

VerityDB provides a **transparent SQL experience**:
- Developer uses normal SQL migrations and any Rust ORM (sqlx, diesel, sea-orm)
- System auto-generates events from INSERT/UPDATE/DELETE via SQLite `preupdate_hook`
- Event sourcing happens completely "under the hood"
- Reads go directly to SQLite (fast path)
- Full audit trail, point-in-time recovery, and HIPAA compliance "for free"

**Future (v2)**: libsql-compatible HTTP protocol for JS/TS ORM support (drizzle, prisma)

---

## Workspace Structure

```
veritydb/
  Cargo.toml                      # Workspace root
  PLAN.md                         # This file
  rust-toolchain.toml
  schemas/
    veritydb.capnp                # Cap'n Proto schemas
  crates/
    vdb/                          # User-facing VerityDb facade (default member)
    vdb-types/                    # IDs, enums, placement, data_class
    vdb-wire/                     # Cap'n Proto codecs
    vdb-kernel/                   # Functional core (commands → effects)
    vdb-vsr/                      # VSR replication engine
    vdb-storage/                  # Append-only segment store
    vdb-directory/                # Placement router (stream → group mapping)
    vdb-projections/              # SQLCipher projection runtime (dual-mode)
    vdb-runtime/                  # Orchestrator: propose → commit → apply → execute
    vdb-server/                   # Cap'n Proto RPC server
    vdb-client/                   # Client SDK
    vdb-admin/                    # CLI tooling
```

---

## Implementation Phases

### Phase 1: Foundation (Milestone A) ✅ COMPLETE

**Goal**: Core types, kernel, single-node storage - enough to prove architecture.

#### 1.1 Create Workspace Structure ✅
- [x] Root `Cargo.toml` with workspace dependencies
- [x] `rust-toolchain.toml` (stable)
- [x] Create all crate directories with `Cargo.toml`

#### 1.2 `vdb-types` - Shared Types ✅
```rust
// Core IDs
pub struct TenantId(pub u64);
pub struct StreamId(pub u64);
pub struct Offset(pub u64);
pub struct GroupId(pub u64);

// Data classification (compliance-critical)
pub enum DataClass { PHI, NonPHI, Deidentified }

// Placement (PHI regional enforcement)
pub enum Placement {
    Region(Region),  // PHI must stay here
    Global,          // Non-PHI can replicate globally
}
```

#### 1.3 `vdb-kernel` - Functional Core ✅
- [x] `command.rs` - Command enum (CreateStream, AppendBatch)
- [x] `effects.rs` - Effect enum (StorageAppend, WakeProjection, Audit)
- [x] `state.rs` - In-memory state (streams metadata)
- [x] `kernel.rs` - `apply_committed(State, Command) -> (State, Vec<Effect>)`

#### 1.4 `vdb-storage` - Append-Only Log ✅
- [x] Segment file format: `[offset:u64][len:u32][payload][crc32:u32]`
- [x] `append_batch(stream, events, expected_offset, fsync)`
- [x] `read_from(stream, from, max_bytes)` with zero-copy
- [x] CRC checksums per record

#### 1.5 `vdb-directory` - Placement Router ✅
- [x] `group_for_placement(placement) -> GroupId`
- [x] PHI streams → regional group
- [x] Non-PHI streams → global group

#### 1.6 `vdb-vsr` - Consensus Abstraction ✅
- [x] `trait GroupReplicator { async fn propose(group, cmd) }`
- [x] `SingleNodeGroupReplicator` - dev mode (commits immediately)

#### 1.7 `vdb-runtime` - Orchestrator ✅
- [x] `Runtime<R: GroupReplicator>`
- [x] `create_stream()` - route via directory, propose
- [x] `append()` - validate placement, propose
- [x] `execute_effects()` - StorageAppend works, WakeProjection is stubbed

---

### Phase 2: Projections (Milestone B) ← CURRENT

**Goal**: Embedded connection wrapper with transparent event capture via SQLite `preupdate_hook`.

**Current Status**: Steps 1-5 complete (partial). Core infrastructure done:
- ✅ Dual-hook architecture (preupdate + commit hooks)
- ✅ ChangeEvent types and SqlValue serialization
- ✅ SchemaCache for column lookups
- ✅ EventPersister trait and RuntimeHandle implementation
- ✅ VerityDb facade with auto-ID stream creation
- 🔄 Remaining: migrate(), schema tracking, replay/recovery, real-time subscriptions

#### 2.1 Architecture Overview

```
Developer Code (sqlx, diesel, sea-orm - any Rust ORM)
        │
        ▼
sqlx::query("INSERT INTO patients ...").execute(&db.pool()).await?
        │
        ▼
┌──────────────────────────────────────────────────────────────────────┐
│                    VerityDb Connection Pool                          │
│  ┌────────────────────────────────────────────────────────────────┐  │
│  │  Wrapped SqlitePool                                            │  │
│  │  - Each connection has preupdate_hook registered               │  │
│  │  - Hooks fire BEFORE any INSERT/UPDATE/DELETE                  │  │
│  │  - Hook captures event → writes to event log                   │  │
│  └────────────────────────────────────────────────────────────────┘  │
│                              │                                       │
│          ┌───────────────────┴───────────────────┐                  │
│          ▼                                       ▼                  │
│  ┌───────────────┐                      ┌───────────────┐           │
│  │  Event Log    │                      │  SQLite DB    │           │
│  │  (durable)    │                      │  (projection) │           │
│  │               │◄─────────────────────│  (encrypted)  │           │
│  └───────────────┘      replays to      └───────────────┘           │
└──────────────────────────────────────────────────────────────────────┘
```

#### 2.2 Dual-Hook Architecture for Strong Consistency ✅

VerityDB uses **two cooperating SQLite hooks** to ensure events are durably persisted
BEFORE SQLite commits. This is critical for HIPAA compliance.

| Hook | When it fires | What we do | Can abort? |
|------|---------------|------------|------------|
| `preupdate_hook` | Before each row change | Capture `ChangeEvent` into buffer | No |
| `commit_hook` | Just before COMMIT | Persist buffer to event log, return `false` to rollback on failure | **YES** |

```rust
// HOOK 1: Captures events into a buffer
handle.set_preupdate_hook(move |result| {
    let event = capture_change_event(result, &schema_cache);
    ctx.buffer.lock().unwrap().push(event);
});

// HOOK 2: Persists events, returns false to rollback on failure
handle.set_commit_hook(move || {
    let events = std::mem::take(&mut *ctx.buffer.lock().unwrap());
    if events.is_empty() { return true; }

    let serialized = serialize_events(&events);
    match ctx.persister.persist_blocking(stream_id, serialized) {
        Ok(_) => true,   // Success → commit proceeds
        Err(_) => false, // Failure → ROLLBACK
    }
});
```

**Why dual-hook (not just preupdate_hook)?**
- `preupdate_hook` has no return value - can't abort transaction
- `commit_hook` returns `bool` - SQLite respects `false` as "rollback"
- Events are batched per transaction (natural batching like TigerBeetle)
- Strong consistency: SQLite only commits if event log succeeds

**TigerBeetle-inspired design:**
- Wait for consensus/durability BEFORE acknowledging to client
- Batch events per transaction for throughput
- No data can exist in SQLite that isn't in the event log

#### 2.3 Event Flow (Strong Consistency)

```
1. User: INSERT INTO patients (name) VALUES ('John')
       ↓
2. SQLite begins transaction
       ↓
3. preupdate_hook fires → captures ChangeEvent::Insert to buffer
       ↓
4. SQLite prepares the row write (not yet committed)
       ↓
5. User calls COMMIT (or transaction auto-commits)
       ↓
6. commit_hook fires:
   a. Drain events from buffer
   b. Serialize to bytes
   c. Call persister.persist_blocking() → blocks until VSR consensus
   d. If success: return true → SQLite commits
   e. If failure: return false → SQLite ROLLBACK
       ↓
7. Return success to caller (only if both event log AND SQLite committed)
```

**Strong consistency guarantee:**
- Event is durably logged BEFORE SQLite commits
- If event log fails, SQLite rolls back (no orphan data)
- If crash after event log but before SQLite commit, replay reconstructs state
- Client NEVER sees "success" unless data is durable in event log

#### 2.4 Key Data Structures

**VerityDb Wrapper:**
```rust
pub struct VerityDb {
    /// Wrapped pool with preupdate hooks
    pool: SqlitePool,
    /// Event log for durability
    event_log: EventLog,
    /// Stream metadata
    stream_id: StreamId,
}

impl VerityDb {
    /// Open a database (creates if not exists)
    pub async fn open(path: impl AsRef<Path>, config: &Config) -> Result<Self>;

    /// Get the pool for use with sqlx/diesel
    pub fn pool(&self) -> &SqlitePool;

    /// Run migrations (captured as SchemaChange events)
    pub async fn migrate(&self, migrations: &[&str]) -> Result<()>;

    /// Replay from event log (for recovery or new replica)
    pub async fn replay_from(&self, offset: Offset) -> Result<()>;
}
```

**CRUD Events:**
```rust
pub enum ChangeEvent {
    /// Schema change (CREATE TABLE, ALTER, etc.)
    SchemaChange {
        statement: String,
        table: Option<String>,
    },
    /// Row inserted
    Insert {
        table: String,
        row_id: i64,
        columns: Vec<String>,
        values: Vec<SqlValue>,
    },
    /// Row updated (full before/after snapshot)
    Update {
        table: String,
        row_id: i64,
        old_values: HashMap<String, SqlValue>,
        new_values: HashMap<String, SqlValue>,
    },
    /// Row deleted
    Delete {
        table: String,
        row_id: i64,
        deleted_row: HashMap<String, SqlValue>,
    },
}

pub enum SqlValue {
    Null,
    Integer(i64),
    Real(f64),
    Text(String),
    Blob(Vec<u8>),
}
```

#### 2.5 `vdb-projections` Module Structure

```
crates/vdb-projections/src/
    lib.rs                 # Main exports
    error.rs               # Extended error types (DecodeError, InvalidDdlStatement, etc.)
    pool.rs                # SQLite connection pools (existing)
    checkpoint.rs          # Checkpoint tracking (existing)
    event.rs               # Event trait, EventEnvelope (existing)
    schema.rs              # SchemaCache for table→column lookups

    events/
        mod.rs             # Events module entry
        change.rs          # ChangeEvent, TableName, ColumnName, RowId, SqlStatement
        values.rs          # SqlValue type with TryFrom<SqliteValueRef>

    realtime/
        mod.rs             # Realtime module entry
        hook.rs            # PreUpdateHandler with preupdate_hook
        subscribe.rs       # TableUpdate, FilteredReceiver (TODO)
```

#### 2.6 Implementation Steps

- [x] **Step 1**: Enable preupdate_hook feature ✅
  - Add `sqlite-preupdate-hook` feature to `vdb-projections/Cargo.toml`
  - Test: verify hook fires for INSERT/UPDATE/DELETE

- [x] **Step 2**: Implement ChangeEvent types ✅
  - `events/change.rs` - `ChangeEvent` enum with Insert/Update/Delete/SchemaChange
  - `events/values.rs` - `SqlValue` type with `TryFrom<SqliteValueRef>`
  - Type-safe newtypes: `TableName`, `ColumnName`, `RowId`, `SqlStatement`
  - Serialization via serde

- [x] **Step 3**: Implement dual-hook architecture ✅
  - `realtime/hook.rs` - `TransactionHooks` with `set_preupdate_hook` + `set_commit_hook`
  - `HookContext` - shared state (buffer, schema_cache, persister, stream_id)
  - `preupdate_hook` captures events into buffer
  - `commit_hook` persists buffer via `EventPersister`, returns `false` on failure
  - Skip internal tables via `TableName::is_internal()`

- [x] **Step 3b**: Schema cache for column lookups ✅
  - `schema.rs` - `SchemaCache` with `RwLock<HashMap<TableName, Vec<ColumnName>>>`
  - `populate_from_db()` - loads existing tables at startup via `PRAGMA table_info`
  - `register_table()` - called during migrations
  - `get_columns()` - used by hook for column name lookups

- [x] **Step 3c**: EventPersister trait ✅
  - `vdb-types` - `EventPersister` trait for sync-to-async bridge
  - `PersistError` enum (ConsensusFailed, StorageError, ShuttingDown)
  - Abstraction allows Runtime or direct Storage implementation

- [x] **Step 4**: Implement EventPersister for Runtime ✅
  - Refactored `Runtime` for concurrent access (interior mutability via `RwLock<State>`)
  - Added `append_raw()` method that bypasses expected_offset validation
  - Created `RuntimeHandle` wrapper implementing `EventPersister`
  - Used `tokio::task::block_in_place` for sync→async bridge
  - Split `vdb-runtime/src/lib.rs` into modules: `runtime.rs`, `handle.rs`, `error.rs`

- [x] **Step 5**: Create VerityDb wrapper (partial) ✅
  - Created new `vdb` crate as user-facing facade (default workspace member)
  - `VerityDb` struct owns `ProjectionDb` + `RuntimeHandle` + `SchemaCache` + `StreamId`
  - `open()` - initializes pools, creates stream with auto-ID, installs hooks ✅
  - `pool()` - exposes read pool for ORM usage ✅
  - Added `VerityDbConfig` for database configuration
  - Added kernel support: `Command::CreateStreamWithAutoId`, `State::with_new_stream()`
  - TODO: `migrate()` - run DDL and capture as SchemaChange events
  - TODO: `write_pool()` exposure for write operations

- [ ] **Step 6**: Schema tracking
  - Capture `CREATE TABLE` as `SchemaChange` event
  - Store schema in event log (enables replay)

- [ ] **Step 7**: Replay/Recovery
  - `VerityDb::replay_from(offset)` to reconstruct state
  - Skip preupdate hooks during replay (avoid re-logging)

- [ ] **Step 8**: Real-time Subscriptions (for SSE/WebSocket)
  - Add `broadcast::Sender<TableUpdate>` to `VerityDb` struct
  - Define `TableUpdate { table, action, row_id, offset }` type
  - Define `UpdateAction { Insert, Update, Delete }` enum
  - Broadcast `TableUpdate` after SQLite write completes
  - Implement `subscribe()` → returns `broadcast::Receiver<TableUpdate>`
  - Implement `subscribe_to(table)` → returns `FilteredReceiver<TableUpdate>`
  - Perfect for SSE with Datastar, WebSockets, or background jobs

- [ ] **Step 9**: Read consistency infrastructure
  - Add `ReadConsistency` enum to `vdb-types`
  - Add `wait_for_offset()` to `ProjectionDb`
  - Add `ConsistencyTimeout` error variant

- [ ] **Step 10**: Session offset tracking
  - Write operations return `Offset`
  - Client SDK tracks session offset automatically
  - HTTP headers: `X-Read-After`, `X-Served-At`, `X-Write-Offset`

- [ ] **Step 11**: Synchronized (linearizable) reads
  - Add `get_commit_index()` to `GroupReplicator` trait
  - Implement read barrier using commit index
  - Wire to HTTP header: `X-Read-Consistency: synchronized`

#### 2.7 Files Modified ✅

| File | Change | Status |
|------|--------|--------|
| `crates/vdb-projections/Cargo.toml` | Add `sqlite-preupdate-hook` feature, `bytes` dep | ✅ |
| `crates/vdb-projections/src/lib.rs` | Export ChangeEvent, SqlValue, newtypes, schema, TransactionHooks, SchemaCache | ✅ |
| `crates/vdb-projections/src/error.rs` | Add DecodeError, UnsupportedSqliteType, InvalidDdlStatement | ✅ |
| `crates/vdb-projections/src/pool.rs` | Add Serialize/Deserialize to PoolConfig, Debug to ProjectionDb | ✅ |
| `crates/vdb-projections/src/schema.rs` | Change `from_db()` to builder pattern returning `Self` | ✅ |
| `crates/vdb-types/src/lib.rs` | Add `impl Add for StreamId` for auto-increment | ✅ |
| `crates/vdb-vsr/src/lib.rs` | Add `Clone` bound to `GroupReplicator` trait | ✅ |
| `crates/vdb-kernel/src/command.rs` | Add `CreateStreamWithAutoId` variant | ✅ |
| `crates/vdb-kernel/src/state.rs` | Add `next_stream_id` field, `with_new_stream()` method | ✅ |
| `crates/vdb-kernel/src/kernel.rs` | Handle `CreateStreamWithAutoId` in `apply_committed()` | ✅ |
| `crates/vdb-runtime/src/lib.rs` | Refactor to re-export from modules | ✅ |
| `Cargo.toml` | Add `vdb` to workspace, set as default member | ✅ |

#### 2.8 New Files Created ✅

| File | Purpose | Status |
|------|---------|--------|
| `crates/vdb-projections/src/schema.rs` | SchemaCache for table→column lookups | ✅ |
| `crates/vdb-projections/src/events/mod.rs` | Events module entry | ✅ |
| `crates/vdb-projections/src/events/change.rs` | ChangeEvent, TableName, ColumnName, RowId, SqlStatement | ✅ |
| `crates/vdb-projections/src/events/values.rs` | SqlValue with TryFrom<SqliteValueRef> | ✅ |
| `crates/vdb-projections/src/realtime/mod.rs` | Realtime module entry | ✅ |
| `crates/vdb-projections/src/realtime/hook.rs` | TransactionHooks with preupdate_hook + commit_hook | ✅ |
| `crates/vdb-projections/src/realtime/subscribe.rs` | TableUpdate, FilteredReceiver | Stub only |
| `crates/vdb-runtime/src/runtime.rs` | Core Runtime struct and methods | ✅ |
| `crates/vdb-runtime/src/handle.rs` | RuntimeHandle for sync→async bridging | ✅ |
| `crates/vdb-runtime/src/error.rs` | RuntimeError type | ✅ |
| `crates/vdb/Cargo.toml` | User-facing vdb crate manifest | ✅ |
| `crates/vdb/src/lib.rs` | VerityDb facade struct | ✅ |
| `crates/vdb/src/config.rs` | VerityDbConfig for database configuration | ✅ |

#### 2.9 Read Consistency Controls

VerityDB provides two read consistency levels, with **Default (causal)** as the default:

| Level | Domain Name | Technical Term | Use Case |
|-------|-------------|----------------|----------|
| Default | **Default** | Causal | All normal reads - "you see your own writes" |
| Opt-in | **Synchronized** | Linearizable | Critical commits - sign-offs, billing, dispensing |

**Why "Default" is causal:**
- Healthcare users expect writes to be immediately visible
- Prevents confusing "my data disappeared" scenarios
- Most reads don't need absolute latest - just their own writes
- Safe default that "just works" without developer thought

**When to use "Synchronized":**
- Clinical note sign-off (only one signature wins)
- Medication dispensing (prevent double-dispense)
- Billing submission (prevent duplicate charges)
- Any operation where "only one can win" semantics matter

**Implementation:**
- Each write returns an `offset` for session tracking
- Reads include `session_offset` for causal consistency (automatic in SDK)
- Synchronized reads query leader for commit index before serving

**API Types:**

```rust
/// Read consistency level for projection queries
pub enum ReadConsistency {
    /// Default: You always see your own writes.
    /// Waits until projection has applied up to your session's last write offset.
    Default { session_offset: Offset },

    /// Synchronized: You see the absolute latest committed data.
    /// Queries leader for current commit index and waits for that offset.
    Synchronized,
}
```

**HTTP Headers:**

| Header | Direction | Purpose |
|--------|-----------|---------|
| `X-Read-After` | Request | Session offset for causal reads |
| `X-Read-Consistency` | Request | Set to `synchronized` for linearizable reads |
| `X-Served-At` | Response | Offset served (for subsequent causal reads) |
| `X-Write-Offset` | Response | New offset after write (for session tracking) |

**Files to Modify:**

| File | Change |
|------|--------|
| `crates/vdb-types/src/lib.rs` | Add `ReadConsistency` enum |
| `crates/vdb-projections/src/error.rs` | Add `ConsistencyTimeout` error |
| `crates/vdb-projections/src/pool.rs` | Add `wait_for_offset()` method |
| `crates/vdb-vsr/src/lib.rs` | Add `get_commit_index()` to `GroupReplicator` trait |

---

### Phase 3: Wire Protocol (Milestone C)

**Goal**: Cap'n Proto RPC, end-to-end client-server communication.

#### 3.1 `vdb-wire` - Cap'n Proto
- [ ] Schema: `PublishReq`, `PublishRes`, `CreateStreamReq`
- [ ] `build.rs` with capnpc
- [ ] Re-export generated types

#### 3.2 `vdb-server` - RPC Server
- [ ] TCP listener with Cap'n Proto RPC
- [ ] `VerityDb::publish()` implementation
- [ ] `VerityDb::create_stream()` implementation
- [ ] mTLS placeholder

#### 3.3 `vdb-client` - SDK
- [ ] `VerityClient::connect(addr)`
- [ ] `publish(tenant, stream, payloads, opts) -> Offsets`
- [ ] `Durability::LocalQuorum | GeoDurable`

---

### Phase 4: VSR Clustering (Milestone D)

**Goal**: 3-node quorum durability, leader failover.

#### 4.1 Real VSR Implementation
- [ ] `VsrGroup` - manages one consensus group
- [ ] Prepare/Commit phases
- [ ] Quorum tracking (majority)
- [ ] View change protocol
- [ ] Log persistence (command log + commit index)

#### 4.2 Cluster Membership
- [ ] Static config initially
- [ ] Reconfiguration command (later)

#### 4.3 Snapshotting
- [ ] Periodic state machine snapshots
- [ ] Snapshot install for new/lagging replicas

#### 4.4 Network Layer
- [ ] Node-to-node communication
- [ ] Heartbeats
- [ ] Timeout/election

---

### Phase 5: Multi-Region + Compliance (Milestone E)

**Goal**: PHI regional enforcement, geo-replication, full audit trail.

#### 5.1 Multi-Group Architecture
- [ ] Multiple VSR groups (per-region PHI groups)
- [ ] Global non-PHI group (control plane)
- [ ] Async cross-region replication for DR

#### 5.2 Policy Enforcement
- [ ] Policy is replicated metadata
- [ ] Append rejects if placement violated
- [ ] Audit event on every policy decision

#### 5.3 Encryption
- [ ] Envelope encryption for records (key_id in metadata)
- [ ] Key rotation command
- [ ] BYOK/KMS integration stub

#### 5.4 Backup/Restore
- [ ] Closed segment upload to object storage
- [ ] Projection snapshot backup
- [ ] Restore workflow

#### 5.5 Audit Stream
- [ ] Immutable audit stream per tenant
- [ ] Every command/effect logged
- [ ] Export format for regulators

---

### Phase 6: SDK + CLI Polish (Milestone F)

**Goal**: Great developer experience for healthcare startups.

#### 6.1 `vdb-admin` CLI
- [ ] `stream create --tenant --stream --data-class --placement`
- [ ] `projection init --config`
- [ ] `migrate apply --db --dir`
- [ ] `projection replay --from`
- [ ] `projection run` (continuous)
- [ ] `projection status`

#### 6.2 First-Party Projection Templates
- [ ] `LatestById` - common healthcare pattern
- [ ] `Timeline` - patient timeline view
- [ ] Config-driven schema generation

---

### Phase 7: libsql Wire Protocol (Milestone G) - FUTURE

**Goal**: HTTP API for JS/TS ORM support (drizzle, prisma) without requiring custom drivers.

#### 7.1 Why libsql Protocol

- Drizzle already supports `@libsql/client` driver
- Prisma has libsql adapter
- SQLite-compatible (not PostgreSQL)
- Existing ecosystem - no need to get ORMs to add support for custom protocol

#### 7.2 Options

1. **Fork/extend libsql-server** - Add event capture to Turso's SQLite server
2. **Implement libsql HTTP API** - Same protocol, existing `@libsql/client` works

#### 7.3 Architecture

```
vdb-server/src/
    libsql/
        mod.rs         # libsql HTTP protocol module
        api.rs         # /v2/pipeline endpoint
        hrana.rs       # Hrana protocol (libsql wire format)
```

Connection URL: `libsql://localhost:8080`

#### 7.4 Usage with Drizzle (no changes needed)

```typescript
import { drizzle } from 'drizzle-orm/libsql';
import { createClient } from '@libsql/client';

const client = createClient({
  url: 'libsql://localhost:8080',  // VerityDB server
  authToken: '...'
});
const db = drizzle(client);

// Works unchanged - events captured server-side
await db.insert(patients).values({ name: 'John' });
const rows = await db.select().from(patients);
```

---

## Example Usage

### Standard Usage (Transparent SQL)

```rust
// Developer just uses normal SQL - feels like SQLite
let db = VerityDb::open("./clinic.db", &config).await?;

// INSERT → event captured via preupdate_hook, logged, then write completes
sqlx::query("INSERT INTO patients (name, dob) VALUES (?, ?)")
    .bind("John Doe")
    .bind("1990-01-15")
    .execute(db.pool())
    .await?;

// SELECT → fast path, directly to SQLite
let patients: Vec<Patient> = sqlx::query_as("SELECT * FROM patients")
    .fetch_all(db.pool())
    .await?;

// Under the hood: full audit trail, point-in-time recovery, compliance "for free"
```

### With Migrations

```rust
let db = VerityDb::open("./clinic.db", &config).await?;

// Migrations are captured as SchemaChange events
db.migrate(&[
    "CREATE TABLE patients (id INTEGER PRIMARY KEY, name TEXT, dob TEXT)",
    "CREATE TABLE appointments (id INTEGER PRIMARY KEY, patient_id INTEGER, scheduled_at TEXT)",
]).await?;

// Now use with any ORM
sqlx::query("INSERT INTO patients (name, dob) VALUES (?, ?)")
    .bind("Jane Smith")
    .bind("1985-03-22")
    .execute(db.pool())
    .await?;
```

### Replay for Recovery

```rust
// After crash or for new replica, replay from event log
let db = VerityDb::open("./clinic.db", &config).await?;
db.replay_from(Offset::ZERO).await?;  // Reconstructs all state from events
```

---

## Verification Plan

### Phase 2 Verification (Current)
```bash
# Build and test
cargo build --workspace
cargo test --workspace

# Integration test: CRUD mode
# 1. Create migrations
# 2. Open CRUD mode database
# 3. Execute INSERT
# 4. Verify event in log
# 5. Verify row in projection
```

### Phase 3 Verification
```bash
# Start server
cargo run -p vdb-server -- --data-dir ./data --region au-syd

# Publish from client
cargo run -p vdb-client -- publish --tenant 1 --stream notes --payload "hello"

# Verify projection
sqlite3 ./data/projections/t_1/notes.db "SELECT * FROM projection_meta;"
```

---

## Dependencies

```toml
[workspace.dependencies]
# Core
anyhow = "1"
thiserror = "1"
bytes = "1"
tokio = { version = "1", features = ["full"] }
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
async-trait = "0.1"
futures = "0.3"

# Cap'n Proto
capnp = "0.19"
capnp-rpc = "0.19"
capnpc = "0.19"

# SQLite (sqlx for async, libsqlite3-sys for SQLCipher)
sqlx = { version = "0.8", features = ["runtime-tokio", "sqlite"] }
libsqlite3-sys = { version = "0.30", features = ["bundled-sqlcipher"] }

# SQL Parsing (for CRUD mode)
sqlparser = "0.53"

# Serialization
serde = { version = "1", features = ["derive"] }
serde_json = "1"

# Concurrency
dashmap = "6"

# Utilities
uuid = { version = "1", features = ["v4", "serde"] }
time = { version = "0.3", features = ["serde"] }
tokio-util = { version = "0.7", features = ["compat"] }
tempfile = "3"
proptest = "1"
```

---

## Design Decisions (Confirmed)

1. **Region naming**: User-defined with sensible AWS defaults (e.g., `ap-southeast-2`, `us-east-1`, `eu-west-1`).

2. **Key management**: KMS interface abstraction with multiple providers:
   - `EnvKeyProvider` - reads from environment variables (self-hosted, local dev)
   - `FileKeyProvider` - reads from protected config file
   - `AwsKmsProvider` - (future) integrates with AWS KMS

3. **Event serialization**: Cap'n Proto for all events. Matches wire protocol, enables zero-copy reads.

4. **Event capture**: SQLite `preupdate_hook` via sqlx `sqlite-preupdate-hook` feature:
   - Synchronous capture before write completes
   - No SQL parsing needed (SQLite provides table, operation, values)
   - Works with any Rust ORM using the connection
   - Full row snapshots for UPDATE/DELETE (enables point-in-time recovery)

5. **Real-time subscriptions**: Broadcast-based pub/sub via `tokio::sync::broadcast`:
   - `subscribe()` for all updates, `subscribe_to(table)` for filtered
   - Lightweight notifications (table, action, row_id) - client re-queries for data
   - Multiple subscribers supported (perfect for SSE/WebSocket)
   - Fires after SQLite write completes

6. **Product vs Crates distinction**:
   - **VerityDB Product** (`vdb-projections`, `vdb-server`): Simple transparent SQL with automatic CRUD capture. Target audience is healthcare startups who want compliance without complexity.
   - **VerityDB Crates** (`vdb-kernel`, `vdb-storage`, `vdb-runtime`, etc.): Building blocks for advanced users who want DDD/CQRS/Event Sourcing. Use directly for full control.

   This keeps the product focused while enabling power users (like Notebar) to leverage the infrastructure.

7. **Wire protocol (future)**: libsql-compatible HTTP API for JS/TS ORM support.
   Avoids need to get drizzle/prisma to add custom protocol support.

8. **Priority**: Dogfood core infrastructure in Notebar ASAP. Minimal viable single-node first, then iterate.

---

## Product Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                    Shared Infrastructure                     │
│  vdb-kernel │ vdb-storage │ vdb-vsr │ vdb-directory          │
└─────────────────────────────────────────────────────────────┘
        │                                    │
        ▼                                    ▼
┌─────────────────────┐          ┌─────────────────────────────┐
│   VerityDB Product  │          │    Direct Crate Usage       │
│   (vdb-projections) │          │    (e.g., Notebar)          │
│                     │          │                             │
│ • Transparent SQL   │          │ • Custom domain events      │
│ • Auto CRUD capture │          │ • Custom projections        │
│ • "Compliance free" │          │ • Full DDD/CQRS control     │
└─────────────────────┘          └─────────────────────────────┘
     ↑                                ↑
  Healthcare                     Power users /
  startups                       Internal apps
```

---

## Notebar Integration (Dogfooding)

Notebar will use the **lower-level crates directly** for full DDD/CQRS/Event Sourcing control.
This dogfoods the core infrastructure (storage, replication, consensus) while allowing custom domain modeling.

```rust
use vdb_kernel::{Command, State, apply_committed};
use vdb_storage::Storage;
use vdb_runtime::Runtime;
use vdb_types::{StreamId, Offset, DataClass, Placement};

// Custom domain events (defined in Notebar, not VerityDB)
#[derive(Serialize, Deserialize)]
pub enum NotebarEvent {
    PatientRegistered { patient_id: i64, source: String },
    NoteCreated { note_id: i64, patient_id: i64, note_type: String },
    NoteSigned { note_id: i64, signed_by: i64 },
    AppointmentScheduled { appointment_id: i64, patient_id: i64, slot: DateTime },
}

// Append domain events directly to storage
let event = NotebarEvent::PatientRegistered { patient_id: 1, source: "web".into() };
let bytes = serde_json::to_vec(&event)?;
runtime.append(stream_id, vec![bytes.into()], expected_offset).await?;

// Custom projections read from event log
let events = storage.read_from(stream_id, last_offset, max_bytes).await?;
for event in events {
    let domain_event: NotebarEvent = serde_json::from_slice(&event.payload)?;
    // Apply to custom projection (SQLite, in-memory, etc.)
}
```

### Notebar Domain Events
1. `PatientRegistered`, `PatientUpdated`, `PatientVerified`
2. `NoteCreated`, `NoteAmended`, `NoteSigned` (clinical notes with audit trail)
3. `AppointmentScheduled`, `AppointmentRescheduled`, `AppointmentCancelled`
4. `InvoiceGenerated`, `PaymentReceived`

### Notebar Projections (Custom SQLite DBs)
- `patients_current` - latest patient state
- `notes_current` - latest version of each note
- `appointments_current` - current appointment state
- `patient_timeline` - chronological patient activity
