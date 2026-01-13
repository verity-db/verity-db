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

#### 2.2 How `preupdate_hook` Works

SQLite provides a callback that fires **synchronously before** each row modification.
sqlx supports this via the `sqlite-preupdate-hook` feature:

```rust
// Enable in Cargo.toml:
// sqlx = { version = "0.8", features = ["runtime-tokio", "sqlite", "sqlite-preupdate-hook"] }

let mut conn = pool.acquire().await?;
conn.lock_handle().await?.preupdate_hook(|action| {
    // Called before each INSERT/UPDATE/DELETE
    match action {
        SqlitePreUpdateHookAction::Insert { table, new_row, .. } => { ... }
        SqlitePreUpdateHookAction::Update { table, old_row, new_row, .. } => { ... }
        SqlitePreUpdateHookAction::Delete { table, old_row, .. } => { ... }
    }
});
```

**Why this approach is ideal:**
- Synchronous (event captured before write completes)
- Semantic (table name, operation, column values - not just bytes)
- Universal (works with any SQLite client using the connection)
- No SQL parsing needed (SQLite tells us what's happening)

#### 2.3 Event Flow

```
1. User: INSERT INTO patients (name) VALUES ('John')
       ↓
2. SQLite begins INSERT transaction
       ↓
3. preupdate_hook fires (BEFORE insert completes)
       ↓
4. Hook extracts: table="patients", op=INSERT, values={"name": "John"}
       ↓
5. Hook serializes ChangeEvent::Insert and appends to event log
       ↓
6. SQLite completes the INSERT to projection DB
       ↓
7. Return success to caller
```

**Key property**: Event is durably logged BEFORE the SQLite write completes.
If crash after step 5 but before step 6, replay will reconstruct state.

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
    lib.rs                 # VerityDb wrapper, main exports
    error.rs               # Extended error types
    pool.rs                # SQLite connection pools (existing)
    checkpoint.rs          # Checkpoint tracking (existing)
    event.rs               # Event trait, EventEnvelope (existing)

    events/
        mod.rs             # Events module entry
        change.rs          # ChangeEvent (Insert/Update/Delete/Schema)
        values.rs          # SqlValue type

    realtime/
        mod.rs             # Realtime module entry
        hook.rs            # preupdate_hook implementation
        subscribe.rs       # TableUpdate, FilteredReceiver
```

#### 2.6 Implementation Steps

- [ ] **Step 1**: Enable preupdate_hook feature
  - Add `sqlite-preupdate-hook` feature to `vdb-projections/Cargo.toml`
  - Test: verify hook fires for INSERT/UPDATE/DELETE

- [ ] **Step 2**: Implement ChangeEvent types
  - `events/change.rs` - `ChangeEvent` enum
  - `events/values.rs` - `SqlValue` type
  - Serialization to/from bytes (serde)

- [ ] **Step 3**: Implement hook callback
  - `realtime/hook.rs` - Register hook on connection
  - Skip internal tables (`_vdb_*`, `sqlite_*`)
  - Extract column values via `sqlite3_preupdate_old/new`

- [ ] **Step 4**: Implement VerityDb wrapper
  - `lib.rs` - `VerityDb` struct
  - Register preupdate hooks on connection creation
  - Wire hook → `Runtime::append()`

- [ ] **Step 5**: Integrate with Runtime
  - Wire `preupdate_hook` → `Runtime::append()`
  - Ensure synchronous commit before SQLite completes

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

#### 2.7 Files to Modify

| File | Change |
|------|--------|
| `crates/vdb-projections/Cargo.toml` | Add `sqlite-preupdate-hook` feature |
| `crates/vdb-projections/src/lib.rs` | Export `VerityDb`, ChangeEvent, realtime types |
| `crates/vdb-runtime/src/lib.rs` | Wire `WakeProjection` to projection engine |
| `crates/vdb-runtime/Cargo.toml` | Add vdb-projections dependency |

#### 2.8 New Files to Create

| File | Purpose |
|------|---------|
| `crates/vdb-projections/src/events/mod.rs` | Events module entry |
| `crates/vdb-projections/src/events/change.rs` | ChangeEvent (Insert/Update/Delete/Schema) |
| `crates/vdb-projections/src/events/values.rs` | SqlValue type |
| `crates/vdb-projections/src/realtime/mod.rs` | Realtime module entry |
| `crates/vdb-projections/src/realtime/hook.rs` | preupdate_hook implementation |
| `crates/vdb-projections/src/realtime/subscribe.rs` | TableUpdate, FilteredReceiver |

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
