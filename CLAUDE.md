# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

VerityDB is a compliance-first, verifiable database for regulated industries (healthcare, finance, legal). Built on a single principle: **All data is an immutable, ordered log. All state is a derived view.**

## Build & Test Commands

```bash
# Build
just build                    # Debug build
just build-release            # Release build

# Testing
just test                     # Run all tests
just nextest                  # Faster test runner (preferred)
just test-one <name>          # Run single test

# Code Quality (run before commits)
just pre-commit               # fmt-check + clippy + test
just clippy                   # Linting (enforces -D warnings)
just fmt                      # Format code

# Full CI locally
just ci                       # fmt-check + clippy + test + doc-check
just ci-full                  # Above + security audits

# Live development with bacon
bacon                         # Watch mode (default)
bacon test-pkg -- vdb-crypto  # Test specific package
```

## Architecture

```
┌──────────────────────────────────────────────────┐
│  Kernel (pure state machine: Cmd → State + FX)   │
├──────────────────────────────────────────────────┤
│  Append-Only Log (hash-chained, CRC32)           │
├──────────────────────────────────────────────────┤
│  Crypto (SHA-256, BLAKE3, AES-256-GCM, Ed25519)  │
└──────────────────────────────────────────────────┘
```

**Crate Structure** (`crates/`):
- `vdb` - Facade, re-exports all modules
- `vdb-types` - Entity IDs (TenantId, StreamId, Offset), data classification
- `vdb-crypto` - Cryptographic primitives (hash chains, signatures, encryption)
- `vdb-storage` - Binary append-only log with CRC32 checksums
- `vdb-kernel` - Pure functional state machine (Commands → State + Effects)
- `vdb-directory` - Placement routing for multi-tenant isolation

## Core Design Patterns

### Functional Core / Imperative Shell (FCIS)

**Mandatory pattern.** The kernel is pure and deterministic. All IO lives at the edges.

```rust
// Core (pure): No IO, no clocks, no randomness
fn apply_committed(state: State, cmd: Command) -> Result<(State, Vec<Effect>)>

// Shell (impure): Executes effects, handles IO
impl Runtime {
    fn execute_effect(&mut self, effect: Effect) -> Result<()>
}
```

For types requiring randomness (crypto keys), use struct-level FCIS:
- `from_random_bytes()` - Pure core, `pub(crate)` to prevent weak input
- `generate()` - Impure shell that provides randomness

### Make Illegal States Unrepresentable

- Use enums over booleans
- Use newtypes over primitives (`TenantId(u64)` not `u64`)
- Encode state machines in types (compile-time enforcement)

### Parse, Don't Validate

Validate at boundaries once, then use typed representations throughout.

### Assertion Density

Every function should have 2+ assertions (preconditions and postconditions). Write assertions in pairs at write and read sites.

## Dual-Hash Cryptography

- **SHA-256**: Compliance-critical paths (hash chains, checkpoints, exports)
- **BLAKE3**: Internal hot paths (content addressing, Merkle trees)

Use `HashPurpose` enum to enforce the boundary at compile time.

## Key Constraints

- **No unsafe code** - Workspace lint denies it
- **No recursion** - Use bounded loops with explicit limits
- **No unwrap in library code** - Use `expect()` with reason for invariants
- **70-line soft limit** per function
- **Rust 1.85 MSRV** - Pinned in `rust-toolchain.toml`

## Error Handling

- `thiserror` for library error types
- `anyhow` for application code with context
- Rich error context via `#[from]` and `#[error(...)]`

## Testing Conventions

- Unit tests in `src/tests.rs` per crate
- Property-based testing with `proptest`
- Parametrized tests with `test-case`
