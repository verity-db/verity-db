# Veritaserum: VerityDB Coding Philosophy

> *"Veritaserum is a powerful truth serum. Three drops of this and You-Know-Who himself would spill his darkest secrets."*

This document defines the coding philosophy and style for VerityDB. Like its namesake, our code must be transparent, honest, and leave no room for deception—not to users, not to ourselves, not to the compiler.

VerityDB is a compliance-first database. Our code must embody the same principles we promise our users: correctness by construction, auditability by design, and no hidden surprises.

---

## Table of Contents

1. [Philosophy](#philosophy)
2. [The Five Principles](#the-five-principles)
3. [Code Style](#code-style)
4. [Error Handling](#error-handling)
5. [Dependency Policy](#dependency-policy)
6. [Documentation Standards](#documentation-standards)
7. [Examples](#examples)

---

## Philosophy

### Correctness Over Convenience

We are not building a database for rapid prototyping. We are building a system of record for regulated industries. Every shortcut we take becomes a liability our users inherit.

- **Do it right the first time.** Refactoring a compliance-critical system is expensive and dangerous.
- **Boring is good.** Clever code is a maintenance burden. Obvious code is a gift.
- **The compiler is your ally.** If the compiler can't verify it, neither can auditors.

### Compliance by Construction

Invalid states should be impossible to represent, not merely checked at runtime. If a tenant's data could theoretically leak to another tenant, we have already failed—even if no bug currently triggers it.

- **Make illegal states unrepresentable.** Use the type system to enforce invariants.
- **Parse, don't validate.** Transform untrusted input into trusted types at system boundaries.
- **Audit trails are not optional.** If it's not in the log, it didn't happen.

### Simplicity is Security

Every abstraction is attack surface. Every dependency is trust extended. Every line of code is a potential bug.

- **Minimize surface area.** Fewer features, fewer bugs, fewer CVEs.
- **Prefer std.** The Rust standard library is well-audited and stable.
- **Question every dependency.** Does it earn its place in our trusted computing base?

---

## The Five Principles

### 1. Functional Core, Imperative Shell (FCIS)

The kernel of VerityDB is a pure, deterministic state machine. All side effects live at the edges.

**The Core (Pure)**:
- Takes commands and current state
- Returns new state and effects to execute
- No I/O, no clocks, no randomness
- Trivially testable with unit tests

**The Shell (Impure)**:
- Handles RPC, authentication, network I/O
- Manages storage, file handles, sockets
- Provides clocks, random numbers when needed
- Executes effects produced by the core

```rust
// GOOD: Pure core function
fn apply_command(state: &State, cmd: Command) -> (State, Vec<Effect>) {
    match cmd {
        Command::CreateTenant { id, config } => {
            let new_state = state.with_tenant(id, config);
            let effects = vec![Effect::LogEvent(Event::TenantCreated { id })];
            (new_state, effects)
        }
        // ...
    }
}

// BAD: Impure core function
fn apply_command(state: &mut State, cmd: Command) -> Result<()> {
    match cmd {
        Command::CreateTenant { id, config } => {
            state.add_tenant(id, config);
            log::info!("Created tenant {}", id);  // Side effect in core!
            self.storage.write(&state)?;          // I/O in core!
            Ok(())
        }
    }
}
```

**Why This Matters**:
- Deterministic replay: Given the same log, we get the same state
- Testing: The core can be tested exhaustively without mocks
- Simulation: We can run thousands of simulated nodes in a single process
- Debugging: Reproduce any bug by replaying the log

### 2. Make Illegal States Unrepresentable

Use Rust's type system to prevent bugs at compile time, not runtime.

**Use enums over booleans**:
```rust
// BAD: Boolean blindness
struct Request {
    is_authenticated: bool,
    is_admin: bool,
}

// GOOD: States are explicit
enum RequestAuth {
    Anonymous,
    Authenticated(UserId),
    Admin(AdminId),
}
```

**Use newtypes over primitives**:
```rust
// BAD: Primitives can be confused
fn transfer(from: u64, to: u64, amount: u64) -> Result<()>;

// GOOD: Types prevent mixups
fn transfer(from: TenantId, to: TenantId, amount: Credits) -> Result<()>;
```

**Use Option/Result over sentinel values**:
```rust
// BAD: Magic values
const NOT_FOUND: i64 = -1;
fn find_offset(key: &[u8]) -> i64;

// GOOD: Explicit absence
fn find_offset(key: &[u8]) -> Option<Offset>;
```

**Encode state machines in types**:
```rust
// BAD: Runtime state checking
struct Transaction {
    state: TransactionState,
}
impl Transaction {
    fn commit(&mut self) -> Result<()> {
        if self.state != TransactionState::Prepared {
            return Err(Error::InvalidState);
        }
        // ...
    }
}

// GOOD: Compile-time state enforcement
struct PreparedTransaction { /* ... */ }
struct CommittedTransaction { /* ... */ }

impl PreparedTransaction {
    fn commit(self) -> CommittedTransaction {
        // Cannot be called on non-prepared transaction
        CommittedTransaction { /* ... */ }
    }
}
```

### 3. Parse, Don't Validate

Validate at system boundaries once, then use typed representations throughout.

**The Pattern**:
1. Untrusted input arrives (bytes, JSON, user input)
2. Parse into strongly-typed representation (or reject)
3. All internal code works with known-valid types
4. Never re-validate what's already been parsed

```rust
// BAD: Validate repeatedly
fn process_tenant_id(id: &str) -> Result<()> {
    if !is_valid_tenant_id(id) {
        return Err(Error::InvalidTenantId);
    }
    // ... use id as &str, must remember to validate again if passed around
}

// GOOD: Parse once, use safely
struct TenantId(u64);

impl TenantId {
    pub fn parse(s: &str) -> Result<Self, ParseError> {
        let id: u64 = s.parse()?;
        if id == 0 {
            return Err(ParseError::ZeroId);
        }
        Ok(TenantId(id))
    }
}

fn process_tenant(id: TenantId) {
    // id is guaranteed valid by construction
}
```

**Apply at boundaries**:
- Network: Parse wire protocol into Request types
- Storage: Parse bytes into Record types
- Config: Parse TOML/JSON into Config types
- User input: Parse strings into domain types

### 4. Assertion Density

Every function should have at least two assertions: one precondition and one postcondition.

**Assertions Are Documentation**:
```rust
fn apply_batch(log: &mut Log, batch: WriteBatch) -> AppliedIndex {
    // Precondition: batch must be non-empty
    assert!(!batch.is_empty(), "empty batch submitted");

    // Precondition: batch must be in order
    debug_assert!(batch.is_sorted(), "batch entries out of order");

    let start_idx = log.next_index();

    for entry in batch {
        log.append(entry);
    }

    let end_idx = log.next_index();

    // Postcondition: we wrote exactly batch.len() entries
    assert_eq!(
        end_idx.0 - start_idx.0,
        batch.len() as u64,
        "applied count mismatch"
    );

    AppliedIndex(end_idx.0 - 1)
}
```

**Paired Assertions**:
Write assertions in pairs—one at the write site, one at the read site:
```rust
// Write site
fn write_record(storage: &mut Storage, record: &Record) {
    let checksum = crc32(&record.data);
    storage.write_u32(checksum);
    storage.write(&record.data);
}

// Read site
fn read_record(storage: &Storage, offset: Offset) -> Record {
    let stored_checksum = storage.read_u32(offset);
    let data = storage.read_bytes(offset + 4);
    let computed_checksum = crc32(&data);

    // Paired assertion: verify what write_record wrote
    assert_eq!(
        stored_checksum, computed_checksum,
        "record corruption at offset {:?}", offset
    );

    Record { data }
}
```

**Debug vs Release Assertions**:
- `assert!()`: Critical invariants, always checked
- `debug_assert!()`: Expensive checks, debug builds only

```rust
fn process_entries(entries: &[Entry]) {
    // Cheap: always check
    assert!(!entries.is_empty());

    // Expensive: debug only
    debug_assert!(entries.windows(2).all(|w| w[0].index < w[1].index));
}
```

### 5. Explicit Control Flow

Control flow should be visible and bounded. No surprises.

**No Recursion**:
```rust
// BAD: Unbounded recursion
fn traverse(node: &Node) {
    process(node);
    for child in &node.children {
        traverse(child);  // Stack overflow risk
    }
}

// GOOD: Explicit iteration with bounds
fn traverse(root: &Node, max_depth: usize) {
    let mut stack = vec![(root, 0)];

    while let Some((node, depth)) = stack.pop() {
        assert!(depth <= max_depth, "max depth exceeded");
        process(node);

        for child in &node.children {
            stack.push((child, depth + 1));
        }
    }
}
```

**Push Ifs Up, Fors Down**:
```rust
// BAD: Conditionals buried in helpers
fn process_all(items: &[Item], mode: Mode) {
    for item in items {
        process_one(item, mode);  // Branch on mode inside
    }
}

// GOOD: Branch at top level
fn process_all(items: &[Item], mode: Mode) {
    match mode {
        Mode::Fast => {
            for item in items {
                process_fast(item);
            }
        }
        Mode::Safe => {
            for item in items {
                process_safe(item);
            }
        }
    }
}
```

**Explicit Loops Over Hidden Iterators**:
When clarity matters more than brevity:
```rust
// Sometimes OK: Simple transformations
let ids: Vec<_> = records.iter().map(|r| r.id).collect();

// Prefer when logic is complex:
let mut ids = Vec::with_capacity(records.len());
for record in &records {
    if record.is_valid() {
        ids.push(record.id);
    }
}
```

**Bounded Loops**:
```rust
// BAD: Unbounded retry
loop {
    if try_connect().is_ok() {
        break;
    }
}

// GOOD: Explicit bounds
const MAX_RETRIES: usize = 3;
for attempt in 0..MAX_RETRIES {
    if try_connect().is_ok() {
        break;
    }
    assert!(attempt < MAX_RETRIES - 1, "connection failed after {} attempts", MAX_RETRIES);
}
```

---

## Code Style

### Naming Conventions

**General Rules**:
- `snake_case` for functions, variables, modules
- `PascalCase` for types, traits, enums
- `SCREAMING_SNAKE_CASE` for constants
- No abbreviations (exceptions: `id`, `idx`, `len`, `ctx`)

**Domain-Specific Names**:
```rust
// IDs are always suffixed
tenant_id: TenantId
record_id: RecordId
stream_id: StreamId

// Indexes are always suffixed
applied_idx: AppliedIndex
commit_idx: CommitIndex

// Offsets are explicit
byte_offset: ByteOffset
log_offset: LogOffset
```

**Verb Conventions**:
```rust
// Constructors
fn new() -> Self              // Infallible
fn with_config(cfg) -> Self   // Infallible with config
fn try_new() -> Result<Self>  // Fallible

// Conversions
fn as_bytes(&self) -> &[u8]   // Borrowed view
fn to_bytes(&self) -> Vec<u8> // Owned copy
fn into_bytes(self) -> Vec<u8> // Consuming conversion

// Queries
fn is_empty(&self) -> bool    // Boolean predicate
fn len(&self) -> usize        // Count
fn get(&self, k) -> Option<V> // Fallible lookup
fn find(&self, k) -> Option<V> // Search

// Mutations
fn set(&mut self, v)          // Replace
fn push(&mut self, v)         // Append
fn insert(&mut self, k, v)    // Add with key
fn remove(&mut self, k)       // Delete
fn clear(&mut self)           // Reset
```

### Function Structure

**Main Logic First**:
```rust
impl Storage {
    // Public API at the top
    pub fn append(&mut self, record: Record) -> Offset {
        self.validate_record(&record);
        let offset = self.write_record(&record);
        self.update_index(offset, &record);
        offset
    }

    pub fn read(&self, offset: Offset) -> Record {
        // ...
    }

    // Private helpers below
    fn validate_record(&self, record: &Record) {
        // ...
    }

    fn write_record(&mut self, record: &Record) -> Offset {
        // ...
    }
}
```

**70-Line Soft Limit**:
If a function exceeds 70 lines, consider whether it should be split. Not a hard rule, but a signal to reconsider.

**Early Returns for Error Cases**:
```rust
fn process(input: &Input) -> Result<Output> {
    // Guard clauses first
    if input.is_empty() {
        return Err(Error::EmptyInput);
    }
    if !input.is_valid() {
        return Err(Error::InvalidInput);
    }

    // Happy path last
    let result = transform(input);
    Ok(result)
}
```

### Module Organization

```
crate_name/
├── lib.rs           # Public API, re-exports
├── types.rs         # Core types (or types/)
├── error.rs         # Error types
├── traits.rs        # Public traits
├── internal/        # Private implementation
│   ├── mod.rs
│   ├── parser.rs
│   └── writer.rs
└── tests/           # Integration tests
    └── integration.rs
```

---

## Error Handling

### Error Types

Use `thiserror` for library errors, `anyhow` for application errors:

```rust
// Library code: Specific error types
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("record not found at offset {0:?}")]
    NotFound(Offset),

    #[error("checksum mismatch: expected {expected}, got {actual}")]
    ChecksumMismatch { expected: u32, actual: u32 },

    #[error("storage full: {current} / {max} bytes")]
    StorageFull { current: u64, max: u64 },

    #[error(transparent)]
    Io(#[from] std::io::Error),
}

// Application code: Anyhow for convenience
use anyhow::{Context, Result};

fn load_config() -> Result<Config> {
    let path = find_config_path()?;
    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read config from {:?}", path))?;
    let config: Config = toml::from_str(&content)
        .context("failed to parse config")?;
    Ok(config)
}
```

### No Unwrap in Library Code

```rust
// BAD: Panics on error
fn get_record(&self, offset: Offset) -> Record {
    self.storage.read(offset).unwrap()
}

// GOOD: Propagate error
fn get_record(&self, offset: Offset) -> Result<Record, StorageError> {
    self.storage.read(offset)
}

// OK in tests:
#[test]
fn test_read_write() {
    let record = storage.get_record(offset).unwrap();
    assert_eq!(record.data, expected);
}
```

### Expect Over Unwrap for Invariants

When unwrap is justified (truly impossible states), use expect with a reason:

```rust
// BAD: No context
let first = items.first().unwrap();

// GOOD: Documents the invariant
let first = items.first().expect("items guaranteed non-empty by validation above");
```

---

## Dependency Policy

### Tier 1: Trusted Core
Always acceptable, well-audited, maintained:
- `std` (Rust standard library)
- `serde`, `serde_json` (serialization)
- `thiserror`, `anyhow` (errors)
- `tracing` (logging/observability)
- `bytes` (byte manipulation)

### Tier 2: Carefully Evaluated
Use when necessary, evaluate each version:
- `mio` (async I/O primitives)
- `tokio` (async runtime, minimize features)
- `sqlparser` (SQL parsing)
- `proptest` (property testing)

### Tier 3: Cryptography
Never roll our own. Use well-audited crates:
- `blake3` (hashing)
- `ed25519-dalek` (signatures)
- `chacha20poly1305` (encryption)
- `rand` (randomness)

### Dependency Checklist

Before adding a dependency:
1. **Necessity**: Can we implement this ourselves in < 200 lines?
2. **Quality**: Is it well-maintained? Last commit? Issue response time?
3. **Security**: Has it been audited? Any CVEs?
4. **Size**: How much does it add to compile time and binary size?
5. **Stability**: Is the API stable? Do they follow semver?
6. **Transitive deps**: What does it pull in?

```toml
# Document why each dependency exists
[dependencies]
# Core serialization, stable, well-audited
serde = { version = "1", features = ["derive"] }

# Error handling for library code
thiserror = "2"

# Structured logging
tracing = "0.1"
```

---

## Documentation Standards

### Module Documentation

Every public module needs a doc comment explaining:
- What it does
- When to use it
- Key types and functions

```rust
//! Storage engine for VerityDB's append-only log.
//!
//! This module provides [`Log`], the core abstraction for durable,
//! ordered event storage. Events are written once and never modified.
//!
//! # Architecture
//!
//! The log is divided into segments. Each segment is a file containing
//! sequential records with CRC checksums.
//!
//! # Example
//!
//! ```
//! use vdb_storage::{Log, LogConfig};
//!
//! let log = Log::open(LogConfig::default())?;
//! let offset = log.append(&record)?;
//! let retrieved = log.read(offset)?;
//! ```
```

### Function Documentation

Public functions need:
- Brief description
- Parameters (if not obvious)
- Return value
- Errors (if Result)
- Panics (if any)

```rust
/// Appends a record to the log and returns its offset.
///
/// The record is durably written before this function returns.
/// The returned offset can be used to read the record back.
///
/// # Errors
///
/// Returns [`StorageError::StorageFull`] if the log has reached
/// its configured maximum size.
///
/// # Panics
///
/// Panics if `record.data` is empty. Empty records are not permitted.
pub fn append(&mut self, record: &Record) -> Result<Offset, StorageError> {
    // ...
}
```

### Comments: Why, Not What

```rust
// BAD: Describes what the code does (obvious from reading it)
// Increment counter by one
counter += 1;

// GOOD: Explains why
// Compensate for off-by-one in legacy index format
counter += 1;

// BAD: Obvious
// Check if empty
if items.is_empty() {

// GOOD: Explains business logic
// Empty batches waste a log entry, so reject early
if items.is_empty() {
```

---

## Examples

### Good vs Bad: State Machine

```rust
// BAD: State as enum, transitions checked at runtime
pub struct Connection {
    state: ConnectionState,
}

pub enum ConnectionState {
    Disconnected,
    Connecting,
    Connected,
}

impl Connection {
    pub fn send(&mut self, msg: Message) -> Result<()> {
        // Runtime check - easy to forget
        if self.state != ConnectionState::Connected {
            return Err(Error::NotConnected);
        }
        self.do_send(msg)
    }
}

// GOOD: State encoded in types, compiler enforces transitions
pub struct Disconnected;
pub struct Connecting { attempt: usize }
pub struct Connected { socket: Socket }

impl Disconnected {
    pub fn connect(self) -> Connecting {
        Connecting { attempt: 1 }
    }
}

impl Connecting {
    pub fn connected(self, socket: Socket) -> Connected {
        Connected { socket }
    }

    pub fn failed(self) -> Disconnected {
        Disconnected
    }
}

impl Connected {
    // Only available on Connected - compile error otherwise
    pub fn send(&mut self, msg: Message) -> Result<()> {
        self.socket.write(&msg)
    }

    pub fn disconnect(self) -> Disconnected {
        Disconnected
    }
}
```

### Good vs Bad: Parse Don't Validate

```rust
// BAD: Validation scattered, can be forgotten
fn create_tenant(name: &str, region: &str) -> Result<Tenant> {
    // Validation here...
    if name.is_empty() {
        return Err(Error::EmptyName);
    }
    if !VALID_REGIONS.contains(&region) {
        return Err(Error::InvalidRegion);
    }

    // ...but what if another function forgets to validate?
    do_create_tenant(name, region)
}

fn do_create_tenant(name: &str, region: &str) -> Result<Tenant> {
    // Caller validated... right?
    Ok(Tenant { name: name.to_string(), region: region.to_string() })
}

// GOOD: Parse at boundary, types guarantee validity
pub struct TenantName(String);
pub struct Region(String);

impl TenantName {
    pub fn parse(s: &str) -> Result<Self, ParseError> {
        if s.is_empty() {
            return Err(ParseError::EmptyName);
        }
        if s.len() > 255 {
            return Err(ParseError::NameTooLong);
        }
        Ok(TenantName(s.to_string()))
    }
}

impl Region {
    pub fn parse(s: &str) -> Result<Self, ParseError> {
        if !VALID_REGIONS.contains(&s) {
            return Err(ParseError::InvalidRegion(s.to_string()));
        }
        Ok(Region(s.to_string()))
    }
}

// Inner function can't receive invalid data
fn create_tenant(name: TenantName, region: Region) -> Tenant {
    Tenant { name, region }
}
```

### Good vs Bad: Error Handling

```rust
// BAD: Generic error, no context
fn load_tenant(id: TenantId) -> Result<Tenant, Box<dyn Error>> {
    let data = storage.read(id)?;
    let tenant = serde_json::from_slice(&data)?;
    Ok(tenant)
}

// GOOD: Specific errors with context
#[derive(Debug, Error)]
pub enum LoadTenantError {
    #[error("tenant {0} not found")]
    NotFound(TenantId),

    #[error("storage error loading tenant {tenant_id}")]
    Storage {
        tenant_id: TenantId,
        #[source]
        source: StorageError,
    },

    #[error("tenant {tenant_id} data is corrupted")]
    Corrupted {
        tenant_id: TenantId,
        #[source]
        source: serde_json::Error,
    },
}

fn load_tenant(id: TenantId) -> Result<Tenant, LoadTenantError> {
    let data = storage.read(id).map_err(|e| {
        if matches!(e, StorageError::NotFound(_)) {
            LoadTenantError::NotFound(id)
        } else {
            LoadTenantError::Storage { tenant_id: id, source: e }
        }
    })?;

    let tenant = serde_json::from_slice(&data)
        .map_err(|e| LoadTenantError::Corrupted { tenant_id: id, source: e })?;

    Ok(tenant)
}
```

---

## Summary

Veritaserum is not just a coding style—it's a commitment to building software worthy of trust. Every principle exists because compliance-critical systems cannot afford ambiguity.

When in doubt:
- **Make it compile-time** over runtime
- **Make it explicit** over implicit
- **Make it boring** over clever
- **Make it traceable** over convenient

The code we write will be audited, scrutinized, and depended upon. Write code that you would trust with your own medical records.
