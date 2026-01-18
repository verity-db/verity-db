# VerityDB Implementation Plan

## Overview

**VerityDB** is being rebuilt as the "TigerBeetle for healthcare" - a compliance-native database built on correctness and compliance by design, not ease of getting started.

**Key Value Proposition**: One ordered log → deterministic apply → snapshot (TigerBeetle invariant)

**Architecture**:
- VSR-replicated append-only log as source of truth
- Custom embedded projection engine (B+tree with MVCC)
- SQL subset query layer
- Multitenancy with per-tenant isolation
- Compliance primitives (audit, encryption, retention)

---

## Current State (Post-Pivot Cleanup)

### Active Crates

| Crate | Status | Purpose |
|-------|--------|---------|
| `vdb-types` | ✅ Active | Core IDs, placement, EventPersister trait |
| `vdb-storage` | ✅ Active | Append-only log with CRC32 checksums |
| `vdb-kernel` | ✅ Active | Functional core (Command → State → Effects) |
| `vdb-vsr` | ✅ Active | GroupReplicator trait + SingleNodeGroupReplicator stub |
| `vdb-directory` | ✅ Active | Placement routing (PHI regional enforcement) |
| `vdb-runtime` | ✅ Active | Orchestrator (propose → commit → apply → execute) |
| `vdb` | ✅ Stub | User-facing SDK (to be implemented) |
| `vdb-wire` | ✅ Stub | Cap'n Proto schemas |
| `vdb-server` | ✅ Stub | RPC server daemon |
| `vdb-client` | ✅ Stub | Client SDK |
| `vdb-admin` | ✅ Stub | CLI tooling |

### Removed

| Crate | Reason |
|-------|--------|
| `vdb-projections` | Deleted - SQLite-specific implementation |

### New Crates (To Be Created)

| Crate | Purpose |
|-------|---------|
| `vdb-crypto` | Hash chain, Ed25519 signatures, envelope encryption |
| `vdb-store` | Custom B+tree/MVCC projection store |
| `vdb-query` | SQL subset parser and executor |

---

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                           VerityDB                              │
│                                                                 │
│  ┌─────────────┐   ┌─────────────┐   ┌─────────────────────┐   │
│  │  vdb-types  │   │ vdb-crypto  │   │    vdb-storage      │   │
│  │  (IDs, etc) │   │(hash chain) │   │  (append-only log)  │   │
│  └─────────────┘   └─────────────┘   └─────────────────────┘   │
│                           │                    │                │
│                           ▼                    ▼                │
│  ┌─────────────────────────────────────────────────────────┐   │
│  │                      vdb-kernel                          │   │
│  │              (pure functional state machine)             │   │
│  │           Command → apply_committed → Effects            │   │
│  └─────────────────────────────────────────────────────────┘   │
│                           │                                     │
│                           ▼                                     │
│  ┌──────────────┐   ┌──────────────┐   ┌──────────────────┐    │
│  │   vdb-vsr    │   │  vdb-store   │   │    vdb-query     │    │
│  │ (consensus)  │   │ (B+tree/MVCC)│   │  (SQL subset)    │    │
│  └──────────────┘   └──────────────┘   └──────────────────┘    │
│                           │                                     │
│                           ▼                                     │
│  ┌─────────────────────────────────────────────────────────┐   │
│  │                      vdb-runtime                         │   │
│  │                    (orchestrator)                        │   │
│  └─────────────────────────────────────────────────────────┘   │
│                           │                                     │
│                           ▼                                     │
│  ┌──────────────┐   ┌──────────────┐   ┌──────────────────┐    │
│  │     vdb      │   │  vdb-server  │   │    vdb-admin     │    │
│  │    (SDK)     │   │   (daemon)   │   │     (CLI)        │    │
│  └──────────────┘   └──────────────┘   └──────────────────┘    │
└─────────────────────────────────────────────────────────────────┘
```

---

## Implementation Phases

### Phase 1: Foundation (Cleanup & Crypto) ← CURRENT

**Goal**: Remove SQLite dependencies, add crypto primitives

**Completed**:
- [x] Delete `vdb-projections` crate
- [x] Remove SQLite/sqlx dependencies from workspace
- [x] Clean up `vdb-types` (remove sqlx derive)
- [x] Clean up `vdb-runtime` (remove projections dependency)
- [x] Clean up `vdb` crate (stub for new SDK)
- [x] Update PLAN.md with new direction

**Next Steps**:
- [ ] Create `vdb-crypto` crate
  - Blake3 hash chain (`chain_hash(prev, data) -> Hash`)
  - Ed25519 signatures for records
  - Envelope encryption for tenant data
- [ ] Extend `vdb-types`
  - Add `RecordHeader` with hash chain fields
  - Add `AppliedIndex` for projection tracking
- [ ] Update `vdb-storage`
  - Add `prev_hash` field to Record
  - Add offset index for O(1) lookups
  - Add checkpoint support

### Phase 2: Custom Projection Store

**Goal**: Build vdb-store with B+tree and MVCC

- [ ] Create `vdb-store` crate
- [ ] Implement page-based storage (4KB pages)
- [ ] Implement B+tree for primary key lookups
- [ ] Implement secondary indexes
- [ ] Implement MVCC snapshots

**API Target**:
```rust
pub trait ProjectionStore: Send + Sync {
    fn apply(&self, idx: AppliedIndex, batch: WriteBatch) -> Result<()>;
    fn applied_index(&self) -> Result<AppliedIndex>;
    fn get(&self, key: &[u8]) -> Result<Option<Bytes>>;
    fn scan_prefix(&self, prefix: &[u8], limit: usize) -> Result<Vec<(Bytes, Bytes)>>;
    fn snapshot(&self) -> Result<Box<dyn Snapshot>>;
}
```

### Phase 3: Query Layer

**Goal**: SQL subset parser and executor

- [ ] Create `vdb-query` crate
- [ ] Use `sqlparser` for parsing
- [ ] Support: SELECT, WHERE (=, <, >, IN), ORDER BY, LIMIT
- [ ] Query planner (index selection)
- [ ] Query executor (against projection store)

### Phase 4: SDK & Runtime Integration

**Goal**: User-facing API with tenant isolation

- [ ] Implement `Verity` struct
- [ ] Implement `TenantHandle`
- [ ] Wire runtime to vdb-store
- [ ] Implement apply loop (log → projection)

**API Target**:
```rust
pub struct Verity { ... }

impl Verity {
    pub fn tenant(&self, id: TenantId) -> TenantHandle;
}

impl TenantHandle {
    pub async fn append(&self, stream: &str, record: impl Serialize) -> Result<Offset>;
    pub async fn query(&self, sql: &str) -> Result<QueryResult>;
    pub fn read_at(&self, offset: Offset) -> ConsistencyGuard;
}
```

### Phase 5: VSR Consensus (Future)

**Goal**: Replace SingleNodeGroupReplicator with full VSR

- [ ] Implement VSR protocol
- [ ] Prepare/Commit phases
- [ ] View changes (leader election)
- [ ] Transport layer
- [ ] Snapshot transfer for new replicas

---

## Design Principles

### Functional Core / Imperative Shell
- **Kernel (pure)**: Commands → State + Effects. No IO, no clocks, deterministic.
- **Shell (impure)**: RPC, auth, VSR transport, storage IO.

### TigerBeetle-Inspired Invariants
- Ordered log is the source of truth
- Projection state = pure function of (initial_state, log_entries)
- Consensus before acknowledge
- Deterministic replay

### Healthcare Compliance
- Per-tenant encryption keys
- PHI regional enforcement via placement
- Full audit trail in append-only log
- MVCC snapshots for point-in-time queries

---

## Verification Plan

### Build & Test
```bash
cargo build --workspace
cargo test --workspace
cargo clippy --workspace
```

### Demo Target (Post Phase 4)
```rust
use vdb::Verity;

let db = Verity::open("./data").await?;
let tenant = db.tenant(TenantId::new(1));

// Append a record
let offset = tenant.append("patients", json!({
    "id": 1,
    "name": "John Doe",
})).await?;

// Query with SQL subset
let results = tenant.query("SELECT * FROM patients WHERE id = 1").await?;
```

---

## Dependencies

```toml
[workspace.dependencies]
# Core
anyhow = "1"
thiserror = "2"
bytes = { version = "1", features = ["serde"] }
tokio = { version = "1", features = ["full"] }
tracing = "0.1"
async-trait = "0.1"

# Cap'n Proto
capnp = "0.25"
capnp-rpc = "0.25"

# SQL parsing (for query layer)
sqlparser = "0.60"

# Serialization
serde = { version = "1", features = ["derive"] }
serde_json = "1"

# Testing
proptest = "1"
tempfile = "3"
```
