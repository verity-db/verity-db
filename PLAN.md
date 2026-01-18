# VerityDB Implementation Plan

## Overview

**VerityDB** is a compliance-native database designed for regulated industries. Built on a single architectural principle: **all data is an immutable, ordered log; all state is a derived view**.

Inspired by TigerBeetle's approach to financial transactions, VerityDB prioritizes correctness and auditability over flexibility and convenience.

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
| **Cryptography** | blake3 + ed25519-dalek + ChaCha20-Poly1305 | Well-audited crates, no custom crypto |
| **Wire protocol** | Custom binary protocol | Like TigerBeetle/Iggy, maximum control |

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
| `vdb-crypto` | ✅ Active | Cryptographic primitives (hash chains via Blake3) |
| `vdb-storage` | ✅ Active | Append-only log with CRC32 checksums (sync I/O) |
| `vdb-kernel` | ✅ Active | Pure functional state machine (Command → State + Effects) |
| `vdb-directory` | ✅ Active | Placement routing, tenant-to-shard mapping |

### Planned Crates (Future Phases)

| Crate | Phase | Purpose |
|-------|-------|---------|
| `vdb-sim` | Phase 2 | VOPR simulation harness for deterministic testing |
| `vdb-vsr` | Phase 3 | Viewstamped Replication consensus |
| `vdb-store` | Phase 4 | B+tree projection store with MVCC |
| `vdb-query` | Phase 5 | SQL subset parser and executor |
| `vdb-wire` | Phase 7 | Binary wire protocol definitions |
| `vdb-server` | Phase 7 | RPC server daemon |
| `vdb-client` | Phase 7 | Low-level RPC client |
| `vdb-admin` | Phase 7 | CLI administration tool |

---

## Implementation Phases

### Phase 1: Foundation (Crypto & Storage) ← CURRENT

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
- [x] Implement Blake3 hash chain in `vdb-crypto` (`chain_hash(prev, data) -> ChainHash`)

**Next**:
- [ ] Extend `vdb-crypto`
  - Ed25519 signatures for records
  - ChaCha20-Poly1305 envelope encryption for tenant data
- [ ] Extend `vdb-storage`
  - Add `prev_hash` field to Record
  - Add offset index for O(1) lookups
  - Add checkpoint support
- [ ] Extend `vdb-types`
  - Add `RecordHeader` with hash chain fields
  - Add `AppliedIndex` for projection tracking

### Phase 2: Deterministic Simulation Testing

**Goal**: Build VOPR simulation harness before VSR implementation

- [ ] Create `vdb-sim` crate (simulation harness)
  - Simulated time (discrete event)
  - Simulated network (message queues, partitions, delays)
  - Simulated storage (failure injection)
- [ ] Implement invariant checkers
  - Log consistency checker
  - Hash chain verifier
  - Linearizability checker
- [ ] Build VOPR binary
  - Seed-based reproducibility
  - Fault injection configuration
  - Shrinking for minimal reproductions

### Phase 3: Consensus (VSR)

**Goal**: Implement Viewstamped Replication with full simulation testing

- [ ] Implement VSR protocol in `vdb-vsr`
  - Normal operation (Prepare/PrepareOK/Commit)
  - View changes (StartViewChange/DoViewChange/StartView)
  - Repair mechanisms (log repair, state transfer)
- [ ] Test every line under simulation
  - Node crashes and restarts
  - Network partitions
  - Message reordering and loss
- [ ] SingleNodeReplicator as degenerate case

### Phase 4: Custom Projection Store

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

## Documentation

| Document | Purpose |
|----------|---------|
| [ARCHITECTURE.md](docs/ARCHITECTURE.md) | System design and data flow |
| [VERITASERUM.md](docs/VERITASERUM.md) | Coding philosophy and standards |
| [TESTING.md](docs/TESTING.md) | Testing strategy and VOPR |
| [COMPLIANCE.md](docs/COMPLIANCE.md) | Audit trails and encryption |
| [PERFORMANCE.md](docs/PERFORMANCE.md) | Performance guidelines |
| [OPERATIONS.md](docs/OPERATIONS.md) | Deployment and operations |

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

# Cryptography
blake3 = "1"
ed25519-dalek = "2"
chacha20poly1305 = "0.10"

# Serialization
serde = { version = "1", features = ["derive"] }
serde_json = "1"

# SQL parsing
sqlparser = "0.60"

# Testing
proptest = "1"
tempfile = "3"
```
