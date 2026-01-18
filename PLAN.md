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

### Phase 2: Projections & Multi-Tenant SDK (Milestone B) ← CURRENT

**Goal**: Tenant-first SDK with transparent event capture, per-tenant SQLite files, LRU pool caching, and lazy migrations.

**Current Status**: Core projection infrastructure complete. Multi-tenant SDK in progress:
- ✅ Dual-hook architecture (preupdate + commit hooks)
- ✅ ChangeEvent types and SqlValue serialization
- ✅ SchemaCache for column lookups
- ✅ EventPersister trait and RuntimeHandle implementation
- ✅ Basic VerityDb facade with auto-ID stream creation
- 🔄 In Progress: Multi-tenant `Verity` + `TenantHandle` redesign
- 🔄 Remaining: LRU pool cache, lazy migrations, replay/recovery, real-time subscriptions

#### 2.1 Multi-Tenant SDK Architecture

**Target Developer Experience:**
```rust
let verity = Verity::from_env()?;           // reads DATABASE_URL once

// per request:
let tenant_id = auth.tenant_id();
let db = verity.tenant(tenant_id).await?;   // returns tenant-scoped handle
sqlx::query("SELECT * FROM users")
    .fetch_all(db.pool())                   // works with any ORM
    .await?;
```

**Core Components:**

```
┌─────────────────────────────────────────────────────────────────┐
│                         Verity                                   │
│  - Root SDK entry point                                         │
│  - Reads DATABASE_URL (e.g., verity:///var/lib/verity/data)     │
│  - Owns TenantPoolCache, MigrationManager, Runtime              │
└─────────────────────────────────────────────────────────────────┘
                              │
                              │ .tenant(tenant_id)
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                      TenantPoolCache                             │
│  - LRU cache keyed by TenantId                                  │
│  - Max cached tenants (default: 100)                            │
│  - Idle timeout eviction (default: 5 min)                       │
│  - Double-check locking for concurrent access                   │
└─────────────────────────────────────────────────────────────────┘
                              │
                              │ get_or_create()
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                       TenantHandle                               │
│  - Tenant-scoped database access                                │
│  - pool() → general-purpose pool (4 conn, with hooks)           │
│  - read_pool() → read-only pool (8 conn, no hooks, optional)    │
│  - Owns: SqlitePools, SchemaCache, StreamId                     │
└─────────────────────────────────────────────────────────────────┘
```

**File Structure (Per-Tenant):**

```
{data_dir}/
  tenants/
    000000000000002a/        # tenant_id=42 in hex (fixed-width)
      data.sqlite           # encrypted SQLite file
    0000000000000100/        # tenant_id=256
      data.sqlite
```

#### 2.1b Legacy Single-Tenant Architecture (for reference)

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

**Verity (Root Entry Point):**
```rust
pub struct Verity<R: GroupReplicator = SingleNodeGroupReplicator> {
    config: VerityConfig,
    pool_cache: Arc<TenantPoolCache>,
    migration_manager: Arc<MigrationManager>,
    runtime: Arc<Runtime<R>>,
}

impl Verity {
    pub fn from_env() -> Result<Self, VerityError>;
    pub fn new(config: VerityConfig, runtime: Arc<Runtime<R>>) -> Result<Self, VerityError>;
    pub async fn tenant(&self, tenant_id: TenantId) -> Result<TenantHandle, VerityError>;
    pub fn warmup(&self, tenant_ids: Vec<TenantId>);
    pub async fn evict(&self, tenant_id: TenantId);
    pub async fn shutdown(self) -> Result<(), VerityError>;
}
```

**TenantHandle (Per-Tenant Access):**
```rust
#[derive(Clone)]
pub struct TenantHandle {
    tenant_id: TenantId,
    db: Arc<TenantDb>,
    last_accessed: Arc<AtomicU64>,
    stream_id: StreamId,
    schema_cache: Arc<SchemaCache>,
}

struct TenantDb {
    pool: SqlitePool,       // general-purpose (4 connections, with hooks)
    read_pool: SqlitePool,  // read-only (8 connections, no hooks)
    path: PathBuf,
}

impl TenantHandle {
    pub fn pool(&self) -> &SqlitePool;        // general-purpose (default)
    pub fn read_pool(&self) -> &SqlitePool;   // read-only for high-read workloads
    pub fn tenant_id(&self) -> TenantId;
    pub fn db_path(&self) -> &Path;
}
```

**VerityConfig:**
```rust
pub struct VerityConfig {
    pub data_dir: PathBuf,                    // from DATABASE_URL
    pub pool: PoolConfig,                     // connection settings
    pub cache: CacheConfig,                   // LRU settings
    pub migration: MigrationConfig,           // lazy migration settings
    pub default_data_class: DataClass,        // PHI | NonPHI
    pub default_placement: Placement,         // Region | Global
    pub key_provider: Option<KeyProviderConfig>,
}

pub struct CacheConfig {
    pub max_cached_tenants: usize,            // default: 100
    pub idle_timeout: Duration,               // default: 5 min
    pub eviction_interval: Duration,          // default: 30 sec
}

pub struct MigrationConfig {
    pub max_concurrent_migrations: usize,     // default: 4
    pub auto_migrate: bool,                   // default: true
    pub source: MigrationSource,              // Embedded | Directory(path)
}
```

**TenantPoolCache:**
```rust
pub struct TenantPoolCache {
    entries: RwLock<HashMap<TenantId, CacheEntry>>,
    lru_order: RwLock<VecDeque<TenantId>>,
    init_locks: DashMap<TenantId, Arc<AsyncMutex<()>>>,  // prevent double-open
    config: CacheConfig,
}
```

**MigrationManager:**
```rust
pub struct MigrationManager {
    semaphore: Arc<Semaphore>,        // limits concurrent migrations
    migrations: Migrations,            // embedded or from directory
    migrated: DashSet<TenantId>,      // tracks migrated tenants this session
}
```

**VerityError:**
```rust
pub enum VerityError {
    // Configuration
    DatabaseUrlNotSet,
    InvalidDatabaseUrl { url: String, reason: String },

    // Tenant
    TenantNotFound(TenantId),
    TenantLocked(TenantId),
    TenantCacheExhausted { max: usize, current: usize },

    // Pool
    PoolCreationFailed { tenant_id: TenantId, source: sqlx::Error },
    ConnectionAcquireFailed(sqlx::Error),

    // Migration
    MigrationFailed { tenant_id: TenantId, source: Box<dyn Error> },
    MigrationTimeout(Duration),

    // Runtime
    EventPersistFailed(PersistError),
    StreamNotFound(TenantId),

    // Encryption
    KeyDerivationFailed,
    MasterKeyUnavailable,

    // I/O
    IoError(std::io::Error),
    ShuttingDown,
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

#### 2.6 Multi-Tenant Design Decisions

##### 2.6.1 Per-Tenant SQLite Files (Not Shared DB)

**Choice**: Each tenant gets their own SQLite file.

**Why**:
- Better isolation (one tenant's corruption doesn't affect others)
- Per-tenant encryption keys (HIPAA compliance)
- Independent WAL files (no write contention between tenants)
- Simpler backup/restore per tenant
- Natural fit for healthcare multi-tenancy

##### 2.6.2 Pool Access Pattern

| Method | Returns | Use Case |
|--------|---------|----------|
| `pool()` | General-purpose pool | **Default** - handles both reads and writes |
| `read_pool()` | Read-only pool | High-read workloads needing max concurrency |

**Why a single general-purpose pool**:
- SQLite WAL mode handles concurrency internally (serializes writes, concurrent reads)
- Most web requests mix reads and writes in the same flow
- Simpler mental model - just use `pool()` for everything
- No need to think about which pool to use

**General-purpose pool settings** (default: 4 connections):
- All connections can do both reads and writes
- Hooks installed on all connections via `after_connect`
- SQLite WAL ensures writes serialize automatically
- 4 connections balances read concurrency with write availability

**Read pool** (optional, default: 8 connections):
- For high-read workloads where you want maximum read throughput
- Opens connections with `mode=ro` (read-only)
- Writes fail immediately with `SQLITE_READONLY`
- No hooks installed (reads don't generate events)

**When to use `read_pool()`**:
- Dashboard/reporting queries that never write
- Search/autocomplete endpoints
- Bulk data exports
- Any read-heavy endpoint where you want >4 concurrent queries

##### 2.6.3 Tenant Path Derivation

```rust
fn derive_tenant_path(config: &VerityConfig, tenant_id: TenantId) -> PathBuf {
    let tenant_hex = format!("{:016x}", u64::from(tenant_id));
    config.data_dir
        .join("tenants")
        .join(&tenant_hex)
        .join("data.sqlite")
}
```

**Properties**:
- Deterministic (same tenant_id → same path)
- Safe filesystem characters (hex only)
- Fixed-width (easy to scan/glob)
- No path traversal risk

##### 2.6.4 Per-Tenant Encryption Keys

```rust
fn derive_tenant_key(master_key: &[u8], tenant_id: TenantId) -> [u8; 32] {
    // HKDF derivation with tenant_id as domain separator
    hkdf_sha256(master_key, info: format!("verity-tenant-{}", tenant_id))
}
```

**Benefits**:
- Compromise of one tenant doesn't expose others
- Keys derived on-demand (no per-tenant key storage)
- Master key rotatable

##### 2.6.5 LRU Pool Cache

**Eviction Strategies**:
1. **Capacity-based**: Evict LRU when `entries.len() > max_cached_tenants`
2. **Idle-based**: Background task evicts pools unused for `idle_timeout`

**Concurrency**: Double-check locking pattern prevents multiple tasks from opening the same tenant simultaneously.

##### 2.6.6 Lazy Migration with Concurrency Limits

**Flow**:
1. `verity.tenant(id)` checks if tenant needs migration
2. Acquires semaphore permit (default: 4 concurrent)
3. Runs pending migrations
4. Updates schema cache
5. Marks tenant as migrated (in-memory)

**For 1000+ tenants**:
- Lazy: Only migrate when tenant is accessed
- Warmup: `verity.warmup(tenant_ids)` queues background init
- Concurrent: Semaphore prevents I/O overload

##### 2.6.7 Auto-Create Tenants on First Access

**Decision**: `tenant(id)` auto-creates the database file on first access.

No explicit registration required. This matches common SaaS patterns:
```rust
// Just works - database created if it doesn't exist
let db = verity.tenant(TenantId::new(42)).await?;
```

The flow:
1. `tenant(id)` → check cache → miss
2. Derive path → create directory if needed
3. Open SQLite (creates file if missing)
4. Run migrations
5. Install hooks
6. Cache and return handle

##### 2.6.8 One Stream Per Tenant

**Decision**: Each tenant gets exactly one event stream.

Stream naming: `tenant_{id}` (e.g., `tenant_42`)

**Why single stream**:
- Simplest mental model
- All events for a tenant in one place
- Easier replay/recovery
- Can filter by table name in events if needed

#### 2.7 Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `DATABASE_URL` | `verity:///path/to/data` | Required |
| `VERITY_MAX_TENANTS` | Max cached tenant pools | 100 |
| `VERITY_IDLE_TIMEOUT_SECS` | Pool idle eviction | 300 |
| `VERITY_MAX_CONCURRENT_MIGRATIONS` | Migration parallelism | 4 |
| `VERITY_POOL_CONNECTIONS` | General-purpose pool size | 4 |
| `VERITY_READ_POOL_CONNECTIONS` | Read-only pool size | 8 |
| `VERITY_ENCRYPTION_KEY` | Master key (hex) | None |

#### 2.8 Implementation Steps

##### Phase A: Core Hook Infrastructure ✅

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

- [x] **Step 5**: Create basic VerityDb wrapper ✅
  - Created new `vdb` crate as user-facing facade (default workspace member)
  - `VerityDb` struct owns `ProjectionDb` + `RuntimeHandle` + `SchemaCache` + `StreamId`
  - `open()` - initializes pools, creates stream with auto-ID, installs hooks
  - Added kernel support: `Command::CreateStreamWithAutoId`, `State::with_new_stream()`

##### Phase B: Multi-Tenant SDK ← CURRENT

- [ ] **Step 6**: Core multi-tenant infrastructure
  - `crates/vdb/src/lib.rs` - `Verity` struct, `tenant()` method
  - `crates/vdb/src/tenant.rs` - `TenantHandle` struct (NEW)
  - `crates/vdb/src/cache.rs` - `TenantPoolCache` with LRU (NEW)
  - `crates/vdb/src/path.rs` - path derivation utilities (NEW)
  - `crates/vdb/src/config.rs` - extend with `CacheConfig`, `MigrationConfig`
  - `crates/vdb/src/error.rs` - `VerityError` enum (NEW)

- [ ] **Step 7**: Pool management
  - LRU cache with `get_or_create` pattern
  - Double-check locking for concurrent tenant access
  - Background eviction task for idle pools
  - Graceful pool shutdown

- [ ] **Step 8**: Migration system
  - `crates/vdb/src/migration.rs` - `MigrationManager` (NEW)
  - Semaphore-limited concurrent migrations
  - Per-tenant migration version tracking
  - Schema cache refresh after migration
  - `warmup()` for background initialization

- [ ] **Step 9**: Integration
  - Per-tenant stream creation in runtime
  - Hook installation on write pools
  - `from_env()` implementation
  - Integration with existing `vdb-projections` and `vdb-runtime`

##### Phase C: Advanced Features

- [ ] **Step 10**: Schema tracking
  - Capture `CREATE TABLE` as `SchemaChange` event
  - Store schema in event log (enables replay)

- [ ] **Step 11**: Replay/Recovery
  - `TenantHandle::replay_from(offset)` to reconstruct state
  - Skip preupdate hooks during replay (avoid re-logging)

- [ ] **Step 12**: Real-time Subscriptions (for SSE/WebSocket)
  - Add `broadcast::Sender<TableUpdate>` to `TenantHandle` struct
  - Define `TableUpdate { table, action, row_id, offset }` type
  - Define `UpdateAction { Insert, Update, Delete }` enum
  - Broadcast `TableUpdate` after SQLite write completes
  - Implement `subscribe()` → returns `broadcast::Receiver<TableUpdate>`
  - Implement `subscribe_to(table)` → returns `FilteredReceiver<TableUpdate>`
  - Perfect for SSE with Datastar, WebSockets, or background jobs

- [ ] **Step 13**: Read consistency infrastructure
  - Add `ReadConsistency` enum to `vdb-types`
  - Add `wait_for_offset()` to `ProjectionDb`
  - Add `ConsistencyTimeout` error variant

- [ ] **Step 14**: Session offset tracking
  - Write operations return `Offset`
  - Client SDK tracks session offset automatically
  - HTTP headers: `X-Read-After`, `X-Served-At`, `X-Write-Offset`

- [ ] **Step 15**: Synchronized (linearizable) reads
  - Add `get_commit_index()` to `GroupReplicator` trait
  - Implement read barrier using commit index
  - Wire to HTTP header: `X-Read-Consistency: synchronized`

#### 2.9 Files Modified ✅

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

#### 2.10 New Files Created

##### Phase A Files ✅

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

##### Phase B Files (Multi-Tenant) - TODO

| File | Purpose | Status |
|------|---------|--------|
| `crates/vdb/src/lib.rs` | Refactor to `Verity` struct with `tenant()` method | TODO |
| `crates/vdb/src/tenant.rs` | `TenantHandle` struct for per-tenant access | TODO |
| `crates/vdb/src/cache.rs` | `TenantPoolCache` with LRU eviction | TODO |
| `crates/vdb/src/path.rs` | Path derivation utilities (`derive_tenant_path`) | TODO |
| `crates/vdb/src/config.rs` | Extend with `CacheConfig`, `MigrationConfig` | TODO |
| `crates/vdb/src/error.rs` | `VerityError` enum | TODO |
| `crates/vdb/src/migration.rs` | `MigrationManager` for lazy migrations | TODO |

##### Target `vdb` Crate Structure

```
crates/vdb/src/
    lib.rs              # Verity entry point, re-exports
    tenant.rs           # TenantHandle, TenantDb
    cache.rs            # TenantPoolCache with LRU
    path.rs             # derive_tenant_path(), derive_tenant_key()
    config.rs           # VerityConfig, CacheConfig, MigrationConfig
    error.rs            # VerityError enum
    migration.rs        # MigrationManager
```

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

### Multi-Tenant Usage (Recommended)

```rust
use vdb::{Verity, TenantId};

// Initialize once at startup (reads DATABASE_URL)
let verity = Verity::from_env()?;

// Per-request: get tenant-scoped handle
async fn handle_request(verity: &Verity, auth: &Auth) -> Result<()> {
    let tenant_id = auth.tenant_id();
    let db = verity.tenant(tenant_id).await?;  // cached, lazy-migrated

    // Works with any ORM - sqlx, diesel, sea-orm
    // Just use pool() for everything - handles both reads and writes
    sqlx::query("INSERT INTO patients (name, dob) VALUES (?, ?)")
        .bind("John Doe")
        .bind("1990-01-15")
        .execute(db.pool())
        .await?;

    let patients: Vec<Patient> = sqlx::query_as("SELECT * FROM patients")
        .fetch_all(db.pool())       // same pool works for reads too
        .await?;

    // Optional: use read_pool() for high-concurrency read-only workloads
    // let reports = sqlx::query_as("SELECT ...").fetch_all(db.read_pool()).await?;

    Ok(())
}
```

### Multi-Tenant with Explicit Config

```rust
use vdb::{Verity, VerityConfig, CacheConfig, MigrationConfig};
use std::time::Duration;

let config = VerityConfig {
    data_dir: "/var/lib/verity/data".into(),
    cache: CacheConfig {
        max_cached_tenants: 200,              // up from default 100
        idle_timeout: Duration::from_secs(600),  // 10 min
        eviction_interval: Duration::from_secs(60),
    },
    migration: MigrationConfig {
        max_concurrent_migrations: 8,         // up from default 4
        auto_migrate: true,
        source: MigrationSource::Directory("./migrations".into()),
    },
    ..Default::default()
};

let verity = Verity::new(config, runtime)?;

// Warmup critical tenants at startup
verity.warmup(vec![
    TenantId::new(1),
    TenantId::new(2),
    TenantId::new(3),
]);
```

### Legacy Single-Tenant Usage

```rust
// For simple single-tenant deployments (still supported)
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
// Multi-tenant: migrations run lazily on first access per tenant
let db = verity.tenant(tenant_id).await?;  // auto-migrates

// Or manually trigger migrations
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
let db = verity.tenant(tenant_id).await?;
db.replay_from(Offset::ZERO).await?;  // Reconstructs all state from events
```

---

## Verification Plan

### Phase 2 Verification (Current)

#### Core Hook Infrastructure
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

#### Multi-Tenant SDK
```bash
# Build
cargo build --workspace

# Unit tests
cargo test -p vdb

# Integration test scenario:
# 1. Create Verity from DATABASE_URL
DATABASE_URL=verity:///tmp/verity-test cargo test -p vdb -- --nocapture

# 2. Access tenant(1), tenant(2), tenant(3)
# 3. Verify separate SQLite files created:
ls -la /tmp/verity-test/tenants/
# 0000000000000001/data.sqlite
# 0000000000000002/data.sqlite
# 0000000000000003/data.sqlite

# 4. Verify pools cached (check logs for cache hits)
# 5. Verify LRU eviction when over capacity (set VERITY_MAX_TENANTS=2)
# 6. Verify migrations run lazily (check migration version table)
# 7. Verify hooks capture events per-tenant
```

#### Multi-Tenant Load Test
```bash
# Test with 100+ tenants
VERITY_MAX_TENANTS=50 cargo test -p vdb multi_tenant_load_test -- --nocapture

# Verify:
# - Pool eviction works under load
# - No deadlocks with concurrent tenant access
# - Migration semaphore limits concurrent migrations
# - Background eviction task cleans up idle pools
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

9. **Multi-tenant SDK architecture**: Tenant-first design with `Verity` + `TenantHandle`:
   - Single `DATABASE_URL` for all tenants (e.g., `verity:///var/lib/verity/data`)
   - Per-tenant SQLite files for isolation, encryption, and backup
   - LRU pool cache with configurable capacity (default: 100 tenants)
   - Lazy migrations with concurrency limits (default: 4 concurrent)
   - Auto-create tenant databases on first access
   - One event stream per tenant for simplicity
   - `pool()` returns general-purpose pool (4 conn) for both reads and writes
   - Optional `read_pool()` (8 conn) for high-read workloads

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
