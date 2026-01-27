# Compliance Architecture

VerityDB is designed for any industry where data integrity, auditability, and provable correctness are non-negotiable. Whether you're in healthcare, finance, legal, government, or any other regulated field, this document describes the compliance-related architecture: audit trails, cryptographic guarantees, encryption, and regulatory support.

---

## Table of Contents

1. [Overview](#overview)
2. [Transaction Idempotency for Compliance](#transaction-idempotency-for-compliance)
3. [Recovery Audit Trail](#recovery-audit-trail)
4. [Audit Trail Architecture](#audit-trail-architecture)
5. [Hash Chaining](#hash-chaining)
6. [Cryptographic Sealing](#cryptographic-sealing)
7. [Per-Tenant Encryption](#per-tenant-encryption)
8. [Retention and Legal Hold](#retention-and-legal-hold)
9. [Point-in-Time Reconstruction](#point-in-time-reconstruction)
10. [Regulator-Friendly Exports](#regulator-friendly-exports)
11. [Compliance Checklist](#compliance-checklist)

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
| **No Duplicates** | Transaction-level idempotency IDs prevent double-processing |
| **Recovery Transparency** | Explicit logging of any data discarded during recovery |

### Supported Frameworks

VerityDB's architecture supports compliance with multiple regulatory frameworks:

| Framework | Industry | Key Requirements | VerityDB Support |
|-----------|----------|------------------|------------------|
| **HIPAA** | Healthcare | Audit trails, access controls, encryption | Full |
| **GDPR** | All (EU) | Right to erasure, data portability, consent | Full |
| **SOC 2** | Technology | Security, availability, processing integrity | Full |
| **21 CFR Part 11** | Pharma/Medical Devices | Electronic records, signatures, timestamps | Full |
| **CCPA** | All (California) | Data access, deletion, opt-out | Full |
| **GLBA** | Finance | Data protection, access controls | Full |
| **FERPA** | Education | Student data privacy, access controls | Full |

The same architectural primitives—immutable logs, hash chaining, encryption, and audit trails—provide the foundation for compliance across all frameworks.

---

## Transaction Idempotency for Compliance

In regulated industries, duplicate transactions (e.g., double-charging a patient, double-booking a trade) are compliance violations. VerityDB prevents duplicates through transaction-level idempotency.

### The Problem

Network failures can cause clients to retry without knowing if the original succeeded:

```
Client                    Server
  │                          │
  │  Transaction (ID: abc)   │
  ├─────────────────────────►│  ← Server commits
  │                          │
  │     (network failure)    │  ← Response lost
  │      ◄───────X───────    │
  │                          │
  │  Retry (ID: abc)         │
  ├─────────────────────────►│  ← Without idempotency: DUPLICATE!
  │                          │     With idempotency: Return original result
```

### How VerityDB Prevents Duplicates

Every transaction includes a client-generated idempotency ID:

```rust
// Client generates ID before first attempt
let idempotency_id = IdempotencyId::generate();

// First attempt
let result = client.execute_with_id(idempotency_id, transaction).await;

// If network fails, retry with SAME ID
let result = client.execute_with_id(idempotency_id, transaction).await;
// Returns same result without re-executing
```

### Commitment Proof

Clients can query whether a transaction committed:

```rust
/// Query the commitment status of a transaction.
/// Returns cryptographic proof suitable for audit.
pub struct CommitmentProof {
    /// The idempotency ID that was queried
    pub idempotency_id: IdempotencyId,
    /// Log offset where transaction was committed (if any)
    pub offset: Option<Offset>,
    /// Timestamp of commitment
    pub committed_at: Option<Timestamp>,
    /// Hash at the committed offset (for verification)
    pub hash: Option<Hash>,
}
```

This proof is essential for compliance:
- **Dispute resolution**: Prove a transaction did or did not occur
- **Audit trail**: Link business events to log positions
- **Recovery**: Verify state after system recovery

### Compliance Implications

| Requirement | How Idempotency Helps |
|-------------|----------------------|
| **No duplicate records** | Retries return existing result, not new record |
| **Audit accuracy** | Each business event maps to exactly one log entry |
| **Dispute resolution** | Commitment proof provides cryptographic evidence |
| **Recovery verification** | Clients can verify transactions survived recovery |

---

## Recovery Audit Trail

VerityDB explicitly tracks what data might have been lost during recovery, providing complete transparency for compliance.

### Generation-Based Recovery

Each recovery event creates a new "generation" with an explicit record:

```
┌─────────────────────────────────────────────────────────────────┐
│ Recovery Audit Trail                                             │
│                                                                  │
│  Generation 1: Normal operation                                  │
│  ├─ Offset 0-4950: Committed and acknowledged                   │
│  ├─ Offset 4951-5000: Prepared but not committed               │
│  └─ Recovery triggered: QuorumLoss                              │
│                                                                  │
│  ┌────────────────────────────────────────────────────────────┐ │
│  │ RECOVERY RECORD (logged to audit trail)                     │ │
│  │                                                             │ │
│  │   generation: 2                                             │ │
│  │   previous_generation: 1                                    │ │
│  │   known_committed: 4950                                     │ │
│  │   recovery_point: 4950                                      │ │
│  │   discarded_range: Some(4951..5001)  ← EXPLICIT LOSS        │ │
│  │   timestamp: 2024-01-15T10:30:00Z                          │ │
│  │   reason: QuorumLoss                                        │ │
│  └────────────────────────────────────────────────────────────┘ │
│                                                                  │
│  Generation 2: Normal operation continues                        │
│  ├─ Offset 4951+: New transactions                              │
│  └─ ...                                                          │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

### What Gets Recorded

| Field | Description | Compliance Use |
|-------|-------------|----------------|
| `generation` | New generation number | Correlate events to recovery epochs |
| `previous_generation` | Prior generation | Chain of custody |
| `known_committed` | Last definitely-committed offset | Data loss boundary |
| `recovery_point` | Where log continues | Gap identification |
| `discarded_range` | Offsets that were discarded | **Explicit loss reporting** |
| `timestamp` | When recovery occurred | Incident timeline |
| `reason` | Why recovery was triggered | Root cause analysis |

### Regulatory Reporting

The explicit `discarded_range` enables precise incident reporting:

```rust
// Generate compliance report for data loss incident
fn generate_loss_report(recovery: &RecoveryRecord) -> IncidentReport {
    match &recovery.discarded_range {
        Some(range) => IncidentReport {
            occurred_at: recovery.timestamp,
            affected_records: range.end - range.start,
            first_affected_offset: range.start,
            last_affected_offset: range.end - 1,
            reason: recovery.reason.description(),
            remediation: "Affected clients notified; transactions can be retried",
        },
        None => IncidentReport {
            occurred_at: recovery.timestamp,
            affected_records: 0,
            // Clean recovery - no data loss
            remediation: "No action required",
        },
    }
}
```

### Compliance Framework Mapping

| Framework | Requirement | How Recovery Tracking Helps |
|-----------|-------------|----------------------------|
| **HIPAA** | Report breaches affecting 500+ individuals | `discarded_range` gives exact count |
| **SOC 2** | Processing integrity controls | Recovery records prove data handling |
| **GDPR** | Document data processing activities | Complete audit trail of all recovery events |
| **21 CFR Part 11** | Audit trail for electronic records | Generation transitions logged with timestamps |

---

## Audit Trail Architecture

Every state change in VerityDB is captured in the append-only log with full metadata.

### Timestamp Guarantees

VerityDB uses **wall-clock timestamps with monotonic guarantees** for audit trail compliance:

```rust
pub struct Timestamp(u64);  // Nanoseconds since Unix epoch

impl Timestamp {
    /// Create timestamp ensuring monotonicity within the system.
    /// Returns max(current_wall_clock, last_timestamp + 1ns).
    pub fn now_monotonic(last: Option<Timestamp>) -> Timestamp;
}
```

**Why wall-clock?** Regulatory frameworks require human-readable timestamps:
- HIPAA audit logs must show when records were accessed/modified
- Legal discovery references calendar dates and times
- 21 CFR Part 11 requires accurate timestamps for electronic signatures

**Why monotonic?** Prevents ordering anomalies:
- Clock skew or NTP adjustments could produce out-of-order timestamps
- Monotonicity ensures `event[n].timestamp >= event[n-1].timestamp`
- Worst case: multiple events share the same timestamp (still ordered by position)

**Implementation**: `max(now(), last_timestamp + 1ns)` - if wall clock goes backwards, increment by 1ns instead.

### Event Metadata

Each event includes:

```rust
struct EventMetadata {
    /// Unique position in the log
    position: LogPosition,

    /// When the event was committed (wall clock, monotonic within system)
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
WHERE stream = 'records'
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
    use sha2::{Sha256, Digest};
    let mut hasher = Sha256::new();
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

### Verified Reads with Checkpoints

For production workloads, verifying from genesis on every read is too expensive. Checkpoints provide verification anchors that bound the cost:

```
┌─────────────────────────────────────────────────────────────────┐
│ Verified Read: Without vs With Checkpoints                       │
│                                                                  │
│ WITHOUT CHECKPOINTS (O(n)):                                      │
│ Read offset 5000 → verify 5000 → 4999 → ... → 0 (genesis)       │
│                    [5000 hash checks]                            │
│                                                                  │
│ WITH CHECKPOINTS (O(k) where k = records since checkpoint):      │
│ Read offset 5000 → find checkpoint at 4500                       │
│                  → verify 5000 → 4999 → ... → 4500 (stop)       │
│                    [500 hash checks]                             │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

**Checkpoint trust model**:
- Checkpoints are log records (in the hash chain, tamper-evident)
- A checkpoint is trusted if previously verified to genesis
- Full genesis verification runs periodically (e.g., nightly) or on-demand
- Interactive reads verify only from the nearest checkpoint

**Verification levels**:

| Level | When Used | Verification Scope |
|-------|-----------|-------------------|
| **Full** | Nightly job, audit export, startup | Genesis to tip |
| **Checkpoint** | Normal reads (default) | Nearest checkpoint to target |
| **None** | Read-only analytics replicas | Trust projection state |

**Compliance note**: For audit exports and regulator-facing proofs, always use full verification. Checkpoint verification is a performance optimization for interactive reads where the underlying chain has been fully verified.

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

- **Algorithm**: AES-256-GCM (AEAD, FIPS 197)
- **Key Size**: 256 bits
- **Nonce**: 96 bits, derived from position (no reuse)

```rust
fn encrypt_event(event: &Event, dek: &DataKey) -> EncryptedEvent {
    // Nonce derived from position (unique, never reused)
    let nonce = derive_nonce(event.position);

    // Authenticated encryption
    let cipher = Aes256Gcm::new(dek.as_ref());
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
    streams := ARRAY['records', 'activities']
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
SELECT * FROM records AS OF POSITION 12345
WHERE id = 1;

-- Query state as of timestamp
SELECT * FROM records AS OF TIMESTAMP '2024-01-15 10:30:00'
WHERE id = 1;

-- Query state as of system time (database time, not event time)
SELECT * FROM records AS OF SYSTEM TIME '2024-01-15 10:30:00'
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

### Cross-Framework Requirements

Most regulatory frameworks share common requirements. VerityDB addresses them uniformly:

| Requirement | VerityDB Feature | Frameworks |
|-------------|------------------|------------|
| Complete audit trails | Every change logged with actor, timestamp, correlation | All |
| Data integrity | Hash chaining, CRC checksums, tamper evidence | All |
| Access controls | Per-tenant isolation, RBAC (application layer) | All |
| Encryption at rest | Per-tenant AES-256-GCM (FIPS) | All |
| Encryption in transit | TLS 1.3, optional mutual TLS | All |
| Data retention | Configurable policies, legal holds | All |
| Right to deletion | Cryptographic deletion | GDPR, CCPA |
| Data portability | Standard export formats (JSON, CSV, Parquet) | GDPR |

### HIPAA Technical Safeguards

| Requirement | VerityDB Feature |
|-------------|------------------|
| Access controls | Per-tenant isolation, RBAC (application layer) |
| Audit controls | Complete audit trail with actor, timestamp |
| Integrity controls | Hash chaining, CRC checksums |
| Transmission security | TLS, optional mutual TLS |
| Encryption | Per-tenant AES-256-GCM at rest (FIPS) |

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

### Third-Party Data Sharing Compliance

When sharing data with external services (analytics, LLMs, partners), VerityDB ensures:

| Requirement | VerityDB Feature |
|-------------|------------------|
| Data minimization | Field-level access controls, redaction |
| Purpose limitation | Purpose tracking in consent ledger |
| Consent tracking | Audit of what was shared, when, with whom |
| Anonymization | Redaction, generalization, pseudonymization |
| Audit trail | Complete log of all data exports |

---

## FIPS 140-3 Compliance

VerityDB uses **FIPS-approved algorithms for all compliance-critical operations**. Internal operations may use additional high-performance algorithms where FIPS compliance is not required.

### Algorithm Selection

| Purpose | Algorithm | FIPS Standard | Status |
|---------|-----------|---------------|--------|
| **Compliance Hashing** | SHA-256 | FIPS 180-4 | ✅ Approved |
| **Internal Hashing** | BLAKE3 | N/A (internal only) | ✅ Performance |
| **Signatures** | Ed25519 | FIPS 186-5 | ✅ Approved |
| **Encryption** | AES-256-GCM | FIPS 197 + SP 800-38D | ✅ Approved |
| **Key Derivation** | HKDF-SHA256 | SP 800-56C | ✅ Approved |
| **Random Numbers** | OS CSPRNG | SP 800-90A/B | ✅ Approved |

### Hash Algorithm Strategy

VerityDB uses a **boundary-aware hashing strategy** that maintains FIPS compliance for regulatory-critical operations while enabling high-performance hashing internally.

**Compliance Boundary (SHA-256 - FIPS 180-4)**:
- Log record hash chains (tamper evidence)
- Checkpoint sealing signatures
- Audit exports and third-party proofs
- Any data that may be examined by regulators or auditors

**Internal Operations (BLAKE3)**:
- Content addressing and deduplication
- Merkle tree construction for snapshots
- Internal consistency verification
- Streaming message fingerprinting

**Boundary Enforcement**: The `HashPurpose` enum in code prevents accidental use of BLAKE3 for compliance-critical operations:

```rust
match purpose {
    HashPurpose::Compliance => SHA-256,  // Audit trails, exports, proofs
    HashPurpose::Internal => BLAKE3,     // Dedup, Merkle trees, fingerprints
}
```

**Auditor Note**: All externally-verifiable proofs use FIPS-approved SHA-256. BLAKE3 is used only for internal performance optimization and never appears in audit trails, checkpoints, or exported data.

### Why This Approach?

VerityDB is designed for regulated industries where FIPS compliance is non-negotiable:

1. **Clear boundary**: Compliance paths use FIPS; internal paths may use faster algorithms
2. **Audit simplicity**: Auditors see FIPS algorithms for all external-facing operations
3. **Veritaserum alignment**: "Simplicity is security" within each boundary
4. **Customer reality**: Healthcare, finance, and federal customers require FIPS for auditable data

### Regulatory Framework Compliance

| Framework | Requirement | VerityDB Status |
|-----------|-------------|-----------------|
| **HIPAA** | Strong encryption, audit trails | ✅ Fully compliant |
| **PCI DSS** | AES-256, SHA-256 | ✅ Fully compliant |
| **FISMA** | FIPS 140-3 algorithms | ✅ Fully compliant |
| **GDPR** | Strong cryptography | ✅ Fully compliant |
| **SOC 2** | Industry standard crypto | ✅ Fully compliant |
| **21 CFR Part 11** | Electronic signatures, audit trails | ✅ Fully compliant |

### FIPS 140-3 Certification Roadmap

| Milestone | Target | Status |
|-----------|--------|--------|
| FIPS-approved algorithms | Phase 1 | ✅ Complete |
| HSM/KMS integration | Phase 8 | 🔜 Planned |
| FIPS validation testing | Post-v1.0 | 🔜 Planned |
| CMVP submission | TBD | 🔜 Planned |

### Performance Considerations

The dual-hash strategy optimizes for both compliance and performance:

- **Compliance paths (SHA-256)**: ~500 MB/s is sufficient for audit log throughput since these paths are I/O-bound (fsync dominates)
- **Internal paths (BLAKE3)**: ~3-5x faster than SHA-256, parallel-friendly for large data operations
- **AES-GCM**: Hardware acceleration (AES-NI) provides excellent performance on modern CPUs
- **Best of both**: FIPS compliance where required, maximum performance where not

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
- **Transaction idempotency**: Duplicate transactions are impossible; commitment proofs available
- **Recovery transparency**: Explicit tracking of any data loss during recovery

The goal is not just to pass audits, but to make compliance violations architecturally impossible.
