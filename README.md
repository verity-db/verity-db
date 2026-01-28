# Craton

**The compliance-first database for industries where data integrity is non-negotiable.**

Craton is a verifiable, durable, multi-node database engine designed for environments where correctness, auditability, and trust matter more than convenience.

Built around a single architectural principle:

> **All data is an immutable, ordered log. All state is a derived view.**

This architecture makes Craton uniquely suited for healthcare, finance, legal, government, and any domain where you must be able to prove what happened, when, and why.

---

## Why Craton?

Most databases optimize for flexibility and throughput. Compliance, auditability, and provable correctness are bolted on later—if at all.

**In regulated industries, that's not good enough.**

Craton is designed so that:

- Every change is immutable and ordered
- Every state can be reconstructed from first principles
- Every snapshot can be cryptographically verified
- **Compliance is structural, not procedural**

---

## Core Architecture

```
┌────────────────────────────────────────┐
│           Client SDK / API              │
└──────────────────┬─────────────────────┘
                   │
                   ▼
┌────────────────────────────────────────┐
│         Projection Engine               │  ← Fast, queryable state
│       (derived, rebuildable)            │
└──────────────────┬─────────────────────┘
                   │
                   ▼
┌────────────────────────────────────────┐
│       Replicated Log (VSR)              │  ← System of record
│     (append-only, hash-chained)         │
└────────────────────────────────────────┘
```

**Log-First**: The append-only log is the system of record. All state is derived.

**Deterministic Replay**: Projections are pure, replayable functions of the log.

**Consensus by Default**: Writes are replicated using Viewstamped Replication (VSR). Totally ordered.

**Embedded Projections**: Local projections for fast reads, even in multi-node deployments.

---

## Key Features

### Compliance by Construction

- **Immutable audit trail**: Every change logged with actor, timestamp, and correlation
- **Hash chaining**: Cryptographic tamper evidence for every record
- **Cryptographic sealing**: Signed checkpoints for third-party verification
- **Point-in-time queries**: Reconstruct any historical state

### Multitenancy as a First-Class Concept

- Per-tenant log partitions and projections
- Per-tenant encryption keys (envelope encryption)
- Per-tenant retention and legal hold
- Regional placement for data residency compliance

### Explicit Consistency

| Mode | Description |
|------|-------------|
| **Eventual** | Read local state immediately |
| **Causal** | Read-your-writes, monotonic reads |
| **Linearizable** | Guaranteed to observe latest committed write |

Consistency is explicit—choose the right tradeoff per request.

### Developer Experience

Work at the right level of abstraction:

```rust
// Level 1: Tables (default)
tenant.execute("INSERT INTO records (id, name) VALUES (?, ?)", &[1, "Alpha"]).await?;
let rows = tenant.query("SELECT * FROM records WHERE id = ?", &[1]).await?;

// Level 2: Events + History
let events = tenant.query("SELECT * FROM __events WHERE stream = 'records'").await?;
let historical = tenant.query_at("SELECT * FROM records", LogPosition::new(12345)).await?;

// Level 3: Custom Projections
tenant.execute("CREATE PROJECTION record_summary AS SELECT ...").await?;
```

---

## What Craton Is (and Is Not)

### Craton Is

- A system of record for regulated data
- A durable, ordered event log
- A projection engine for fast application reads
- A foundation for audit trails and forensic reconstruction

### Craton Is Not

- A general-purpose SQL database
- A drop-in replacement for Postgres
- An analytics warehouse or OLAP engine
- A message queue

Craton intentionally limits scope to remain **predictable**, **verifiable**, and **defensible**.

---

## Open Source & Licensing

Craton follows an **open-core model** with a clear philosophy:

> **You are not buying software. You are buying assurance.**

### Open Source (Credibility Layer)

The core engine is fully open source and production-capable:

- Replicated log with hash chaining
- Projection engine
- SQL subset query layer
- SDKs for common languages
- Single-node and self-hosted multi-node operation

**License**: Apache 2.0

The open source version is **correct and complete**. Nothing is artificially limited.

### Commercial Offerings (Assurance Layer)

Paid offerings provide assurance and transfer operational burden:

| Tier | What You Get |
|------|--------------|
| **Compliance Tooling** | Cryptographic sealing, audit reports, regulator-friendly exports |
| **Key Management** | KMS/HSM integration, key rotation, secure key escrow |
| **Attestations** | Third-party timestamp anchoring, auditor co-signatures |
| **Managed Cloud** | We operate it. SLAs, monitoring, backups included. |
| **Enterprise Support** | Dedicated support, custom integrations, training |

**The commercial offering exists to transfer risk, not to gate correctness.**

---

## Who Craton Is For

**Good fit**:
- Healthcare platforms (EHR, clinical data)
- Financial services (audit trails, transaction records)
- Legal and compliance systems (chain of custody, evidence)
- Government and public sector (regulated records)
- Any system that must withstand audits or legal scrutiny

**Probably not a good fit**:
- Ad-hoc analytics
- Complex SQL joins and reporting
- General-purpose OLAP workloads

---

## Documentation

| Document | Description |
|----------|-------------|
| [PLAN.md](PLAN.md) | Implementation roadmap and crate structure |
| [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) | System design and data flow |
| [docs/CRATONICS.md](docs/CRATONICS.md) | Coding philosophy and standards |
| [docs/TESTING.md](docs/TESTING.md) | Testing strategy and simulation |
| [docs/COMPLIANCE.md](docs/COMPLIANCE.md) | Audit trails and encryption |
| [docs/DATA_SHARING.md](docs/DATA_SHARING.md) | Secure third-party data sharing |
| [docs/PERFORMANCE.md](docs/PERFORMANCE.md) | Performance guidelines |
| [docs/OPERATIONS.md](docs/OPERATIONS.md) | Deployment and operations |

---

## Status

> **Early Development**

The core architecture is under active development. Interfaces will change.

Current priorities:

1. **Correctness**: Get the invariants right
2. **Clarity**: Make the code understandable
3. **Testability**: Build simulation testing infrastructure

Performance and tooling will follow.

---

## Getting Involved

- **Read the docs**: Start with [ARCHITECTURE.md](docs/ARCHITECTURE.md)
- **Review the code**: Especially [docs/CRATONICS.md](docs/CRATONICS.md) for standards
- **Open issues**: Design discussions welcome
- **Challenge assumptions**: Correctness improves through scrutiny

---

## Philosophy

> *"If you cannot explain where a piece of data came from, you do not control it."*

Craton is built so that you always can.
