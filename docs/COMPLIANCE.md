# Compliance Architecture

VerityDB is designed for regulated industries where data integrity, auditability, and provable correctness are non-negotiable. This document describes the compliance-related architecture: audit trails, cryptographic guarantees, encryption, and regulatory support.

---

## Table of Contents

1. [Overview](#overview)
2. [Audit Trail Architecture](#audit-trail-architecture)
3. [Hash Chaining](#hash-chaining)
4. [Cryptographic Sealing](#cryptographic-sealing)
5. [Per-Tenant Encryption](#per-tenant-encryption)
6. [Retention and Legal Hold](#retention-and-legal-hold)
7. [Point-in-Time Reconstruction](#point-in-time-reconstruction)
8. [Regulator-Friendly Exports](#regulator-friendly-exports)
9. [Compliance Checklist](#compliance-checklist)

---

## Overview

VerityDB provides **compliance by construction**, not compliance by configuration. The architecture makes certain violations impossible:

| Guarantee | How It's Achieved |
|-----------|-------------------|
| **Immutability** | Append-only log; no UPDATE or DELETE on raw events |
| **Auditability** | Every state change is logged with metadata |
| **Tamper Evidence** | Cryptographic hash chain links all events |
| **Non-Repudiation** | Events can be signed with Ed25519 |
| **Data Sovereignty** | Regional placement enforced at routing layer |
| **Isolation** | Per-tenant encryption keys |
| **Retention** | Legal holds prevent deletion; configurable retention |
| **Reconstruction** | Any point-in-time state derivable from log |

### Supported Frameworks

VerityDB's architecture supports compliance with:

- **HIPAA**: Healthcare data protection (audit trails, access controls, encryption)
- **SOC 2**: Security, availability, processing integrity
- **GDPR**: Right to erasure (via cryptographic deletion), data portability
- **21 CFR Part 11**: Electronic records and signatures (audit trails, timestamps)

---

## Audit Trail Architecture

Every state change in VerityDB is captured in the append-only log with full metadata.

### Event Metadata

Each event includes:

```rust
struct EventMetadata {
    /// Unique position in the log
    position: LogPosition,

    /// When the event was committed (wall clock at commit time)
    timestamp: Timestamp,

    /// Which tenant owns this data
    tenant_id: TenantId,

    /// Which stream within the tenant
    stream_id: StreamId,

    /// Who initiated this change (user, system, API key)
    actor: ActorId,

    /// What caused this event (request ID, correlation ID)
    caused_by: Option<CorrelationId>,

    /// Client IP address (if applicable)
    client_ip: Option<IpAddr>,

    /// Type of operation (INSERT, UPDATE, DELETE, etc.)
    operation: OperationType,
}
```

### Audit Queries

Query the audit trail directly:

```sql
-- All changes to a specific record
SELECT * FROM __events
WHERE stream = 'patients'
  AND data->>'id' = '123'
ORDER BY position ASC;

-- All changes by a specific user
SELECT * FROM __events
WHERE actor = 'user:alice@example.com'
  AND timestamp > '2024-01-01'
ORDER BY timestamp DESC;

-- All deletions in a time range
SELECT * FROM __events
WHERE operation = 'DELETE'
  AND timestamp BETWEEN '2024-01-01' AND '2024-02-01';
```

### What Gets Logged

| Operation | Logged Data |
|-----------|-------------|
| INSERT | Full record, actor, timestamp, correlation |
| UPDATE | Old values, new values, actor, timestamp |
| DELETE | Deleted values, actor, timestamp, reason |
| QUERY | Query text, actor, timestamp (configurable) |
| SCHEMA CHANGE | DDL statement, actor, timestamp |
| ACCESS | Record accessed, actor, timestamp (configurable) |

---

## Hash Chaining

Every event is cryptographically linked to its predecessor, creating a tamper-evident chain.

### How It Works

```
Event 0         Event 1         Event 2         Event 3
┌─────────┐     ┌─────────┐     ┌─────────┐     ┌─────────┐
│ data    │     │ data    │     │ data    │     │ data    │
│         │     │         │     │         │     │         │
│ prev: ──┼──┐  │ prev: ──┼──┐  │ prev: ──┼──┐  │ prev: ──┼──┐
│ 00000   │  │  │ a3f2c   │  │  │ 7b1d4   │  │  │ e9c8a   │  │
│         │  │  │         │  │  │         │  │  │         │  │
│ hash:   │  │  │ hash:   │  │  │ hash:   │  │  │ hash:   │  │
│ a3f2c ◄─┼──┘  │ 7b1d4 ◄─┼──┘  │ e9c8a ◄─┼──┘  │ f2b7d   │
└─────────┘     └─────────┘     └─────────┘     └─────────┘
```

Each event's hash includes:
1. The previous event's hash
2. The event's position
3. The event's timestamp
4. The event's data

### Hash Computation

```rust
fn compute_event_hash(prev_hash: &Hash, event: &Event) -> Hash {
    let mut hasher = blake3::Hasher::new();
    hasher.update(prev_hash.as_bytes());
    hasher.update(&event.position.to_le_bytes());
    hasher.update(&event.timestamp.to_le_bytes());
    hasher.update(&event.data);
    hasher.finalize().into()
}
```

### Tamper Detection

If any event is modified, all subsequent hashes become invalid:

```rust
fn verify_hash_chain(log: &Log) -> Result<(), ChainError> {
    let mut prev_hash = Hash::zero();

    for event in log.iter() {
        let expected_hash = compute_event_hash(&prev_hash, &event);

        if event.hash != expected_hash {
            return Err(ChainError::TamperDetected {
                position: event.position,
                expected: expected_hash,
                actual: event.hash,
            });
        }

        prev_hash = event.hash;
    }

    Ok(())
}
```

### Verification Guarantees

| Attack | Detected? | How? |
|--------|-----------|------|
| Modify event | Yes | Hash mismatch |
| Delete event | Yes | Gap in positions + hash chain break |
| Insert event | Yes | Position conflict + hash chain break |
| Reorder events | Yes | Hash chain break |
| Truncate log | Partial | Missing events (if expected count known) |

---

## Cryptographic Sealing

For high-assurance environments, VerityDB supports cryptographically sealed checkpoints.

### Checkpoint Structure

Periodically, the system creates a signed checkpoint:

```rust
struct SealedCheckpoint {
    /// Log position this checkpoint covers
    through_position: LogPosition,

    /// Hash of the event at through_position
    log_hash: Hash,

    /// Merkle root of all projection state
    projection_hash: Hash,

    /// Wall clock time of seal
    sealed_at: Timestamp,

    /// Ed25519 signature over the above
    signature: Signature,

    /// Public key that created the signature
    signer: PublicKey,
}
```

### Sealing Process

```rust
fn create_sealed_checkpoint(
    log: &Log,
    projections: &ProjectionStore,
    signing_key: &SigningKey,
) -> SealedCheckpoint {
    let position = log.last_position();
    let log_hash = log.get(position).unwrap().hash;
    let projection_hash = projections.merkle_root();
    let sealed_at = Timestamp::now();

    let message = [
        position.to_le_bytes().as_slice(),
        log_hash.as_bytes(),
        projection_hash.as_bytes(),
        &sealed_at.to_le_bytes(),
    ].concat();

    let signature = signing_key.sign(&message);

    SealedCheckpoint {
        through_position: position,
        log_hash,
        projection_hash,
        sealed_at,
        signature,
        signer: signing_key.verifying_key(),
    }
}
```

### Third-Party Attestation

For regulatory requirements, checkpoints can be attested by external parties:

1. **Timestamping Authority**: Checkpoint hash submitted to RFC 3161 TSA
2. **Blockchain Anchoring**: Checkpoint hash anchored to public blockchain
3. **Auditor Signature**: External auditor co-signs checkpoint

---

## Per-Tenant Encryption

Each tenant's data is encrypted with a unique key hierarchy.

### Key Hierarchy

```
┌─────────────────────────────────────────────────────────────────┐
│                      Key Hierarchy                               │
│                                                                  │
│  ┌─────────────────────────────────────────────────────────────┐│
│  │  Master Key (MK)                                            ││
│  │  - Stored in HSM/KMS                                        ││
│  │  - Never leaves secure boundary                             ││
│  │  - Used only to wrap KEKs                                   ││
│  └───────────────────────┬─────────────────────────────────────┘│
│                          │                                       │
│          ┌───────────────┼───────────────┐                      │
│          ▼               ▼               ▼                      │
│  ┌───────────────┐┌───────────────┐┌───────────────┐           │
│  │ KEK_Tenant_A  ││ KEK_Tenant_B  ││ KEK_Tenant_C  │           │
│  │ (wrapped)     ││ (wrapped)     ││ (wrapped)     │           │
│  └───────┬───────┘└───────┬───────┘└───────┬───────┘           │
│          │               │               │                      │
│          ▼               ▼               ▼                      │
│  ┌───────────────┐┌───────────────┐┌───────────────┐           │
│  │ DEK_A_1      ││ DEK_B_1      ││ DEK_C_1      │            │
│  │ DEK_A_2      ││ DEK_B_2      ││ DEK_C_2      │            │
│  │ ...          ││ ...          ││ ...          │            │
│  └───────────────┘└───────────────┘└───────────────┘           │
│                                                                  │
│  MK:  Master Key (HSM)                                          │
│  KEK: Key Encryption Key (per tenant, wrapped by MK)            │
│  DEK: Data Encryption Key (per segment/table, wrapped by KEK)   │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

### Encryption Algorithm

- **Algorithm**: ChaCha20-Poly1305 (AEAD)
- **Key Size**: 256 bits
- **Nonce**: 96 bits, derived from position (no reuse)

```rust
fn encrypt_event(event: &Event, dek: &DataKey) -> EncryptedEvent {
    // Nonce derived from position (unique, never reused)
    let nonce = derive_nonce(event.position);

    // Authenticated encryption
    let cipher = ChaCha20Poly1305::new(dek.as_ref());
    let ciphertext = cipher.encrypt(&nonce, event.data.as_ref())
        .expect("encryption failed");

    EncryptedEvent {
        position: event.position,
        encrypted_data: ciphertext,
        key_id: dek.id,
    }
}

fn derive_nonce(position: LogPosition) -> Nonce {
    let mut nonce = [0u8; 12];
    nonce[..8].copy_from_slice(&position.0.to_le_bytes());
    // Remaining 4 bytes are zero (sufficient uniqueness from position)
    Nonce::from(nonce)
}
```

### Key Rotation

Keys can be rotated without re-encrypting existing data:

1. Generate new DEK
2. New events encrypted with new DEK
3. Old events remain readable with old DEK
4. Old DEK kept until all data using it expires

### Cryptographic Deletion

For GDPR "right to erasure", delete the tenant's KEK:

1. Delete KEK from KMS
2. All DEKs become unrecoverable
3. All tenant data becomes unreadable
4. Log entries remain (for audit) but are cryptographically inaccessible

---

## Retention and Legal Hold

VerityDB supports configurable retention policies and legal holds.

### Retention Policies

```rust
struct RetentionPolicy {
    /// Minimum retention period
    min_retention: Duration,

    /// Maximum retention period (data deleted after)
    max_retention: Option<Duration>,

    /// Delete method
    deletion_method: DeletionMethod,
}

enum DeletionMethod {
    /// Keep metadata, delete payload
    TombstoneOnly,

    /// Cryptographic deletion (delete keys)
    CryptoDelete,

    /// Physical deletion (for non-regulated data)
    PhysicalDelete,
}
```

### Legal Hold

Legal holds prevent deletion regardless of retention policy:

```rust
struct LegalHold {
    /// Unique identifier for this hold
    hold_id: HoldId,

    /// Which tenant is affected
    tenant_id: TenantId,

    /// Optional: specific streams affected
    streams: Option<Vec<StreamId>>,

    /// Optional: specific time range
    time_range: Option<(Timestamp, Timestamp)>,

    /// Why this hold exists
    reason: String,

    /// Who placed the hold
    placed_by: ActorId,

    /// When the hold was placed
    placed_at: Timestamp,
}
```

### Hold Operations

```sql
-- Place a legal hold
CALL place_legal_hold(
    tenant_id := 123,
    reason := 'Litigation hold - Case #456',
    streams := ARRAY['patients', 'visits']
);

-- List active holds
SELECT * FROM __legal_holds WHERE tenant_id = 123;

-- Release a hold
CALL release_legal_hold(hold_id := 'hold_abc123');
```

---

## Point-in-Time Reconstruction

Any historical state can be reconstructed from the log.

### How It Works

```rust
/// Reconstruct state as of a specific log position
fn reconstruct_at(
    log: &Log,
    target_position: LogPosition,
) -> ProjectionState {
    let mut state = ProjectionState::empty();

    for event in log.iter().take_while(|e| e.position <= target_position) {
        state.apply(&event);
    }

    state
}
```

### Query Interface

```sql
-- Query state as of specific position
SELECT * FROM patients AS OF POSITION 12345
WHERE id = 1;

-- Query state as of timestamp
SELECT * FROM patients AS OF TIMESTAMP '2024-01-15 10:30:00'
WHERE id = 1;

-- Query state as of system time (database time, not event time)
SELECT * FROM patients AS OF SYSTEM TIME '2024-01-15 10:30:00'
WHERE id = 1;
```

### Use Cases

| Use Case | Query Type |
|----------|------------|
| Audit investigation | AS OF POSITION (exact state) |
| Compliance report | AS OF TIMESTAMP (business time) |
| Bug investigation | AS OF SYSTEM TIME (when data was committed) |
| GDPR data subject request | Full history export |

---

## Regulator-Friendly Exports

VerityDB produces exports suitable for regulatory review.

### Export Formats

```rust
enum ExportFormat {
    /// JSON Lines (one event per line)
    JsonLines,

    /// CSV with full metadata
    Csv,

    /// Parquet (for large exports)
    Parquet,

    /// Native VerityDB format (for migration)
    Native,
}
```

### Export Command

```bash
# Export tenant data
verity export \
    --tenant 123 \
    --from '2024-01-01' \
    --to '2024-12-31' \
    --format jsonl \
    --output export.jsonl

# Export with cryptographic proof
verity export \
    --tenant 123 \
    --include-proof \
    --output export.jsonl.proof
```

### Export Contents

Each export includes:

1. **Data**: All events in the requested range
2. **Metadata**: Positions, timestamps, actors, correlations
3. **Schema**: Table definitions at each schema version
4. **Proof** (optional): Hash chain verification data

### Proof Structure

```json
{
  "export_id": "exp_abc123",
  "tenant_id": 123,
  "range": {
    "from_position": 1000,
    "to_position": 5000
  },
  "hashes": {
    "first_event_prev_hash": "a3f2c...",
    "last_event_hash": "f2b7d...",
    "merkle_root": "9e8d7..."
  },
  "sealed_checkpoint": {
    "position": 5000,
    "signature": "...",
    "signer": "..."
  }
}
```

A regulator can verify:
1. The hash chain is valid within the export
2. The export connects to a sealed checkpoint
3. No events are missing or modified

---

## Compliance Checklist

### HIPAA Technical Safeguards

| Requirement | VerityDB Feature |
|-------------|------------------|
| Access controls | Per-tenant isolation, RBAC (application layer) |
| Audit controls | Complete audit trail with actor, timestamp |
| Integrity controls | Hash chaining, CRC checksums |
| Transmission security | TLS, optional mutual TLS |
| Encryption | Per-tenant AES-256/ChaCha20 at rest |

### SOC 2 Trust Principles

| Principle | VerityDB Feature |
|-----------|------------------|
| Security | Encryption, access isolation, audit logs |
| Availability | Multi-node replication, consensus |
| Processing Integrity | Hash chains, deterministic replay |
| Confidentiality | Per-tenant encryption, isolation |
| Privacy | Cryptographic deletion, retention policies |

### GDPR Requirements

| Requirement | VerityDB Feature |
|-------------|------------------|
| Right to access | Point-in-time queries, full exports |
| Right to rectification | UPDATE logged with old/new values |
| Right to erasure | Cryptographic deletion |
| Data portability | Standard export formats |
| Storage limitation | Configurable retention policies |

---

## Summary

VerityDB provides compliance by construction:

- **Immutable log**: Events cannot be modified or deleted
- **Hash chaining**: Any tampering is detectable
- **Cryptographic sealing**: External verification possible
- **Per-tenant encryption**: Data isolation enforced cryptographically
- **Retention controls**: Legal holds and configurable policies
- **Point-in-time queries**: Any historical state reconstructible
- **Regulator exports**: Verifiable, complete, and portable

The goal is not just to pass audits, but to make compliance violations architecturally impossible.
