# Secure Data Sharing

Craton includes first-party support for securely sharing data with third-party services while protecting sensitive information. This document describes the data sharing architecture, anonymization capabilities, and integration patterns.

---

## Table of Contents

1. [Overview](#overview)
2. [Why Secure Data Sharing?](#why-secure-data-sharing)
3. [Architecture](#architecture)
4. [Anonymization Techniques](#anonymization-techniques)
5. [Token-Based Access Control](#token-based-access-control)
6. [Export Audit Trail](#export-audit-trail)
7. [MCP Integration](#mcp-integration)
8. [Compliance Considerations](#compliance-considerations)

---

## Overview

Many applications need to interact with external services—analytics platforms, LLMs, third-party APIs, or partner systems—without compromising sensitive data. Craton provides built-in capabilities to:

- **Anonymize or pseudonymize** data before sending it out
- **Encrypt sensitive fields** so only authorized recipients can decrypt them
- **Audit all data exports** to maintain a verifiable record of what was shared, when, and with whom

This ensures that even when you integrate with external systems, your compliance guarantees remain intact.

---

## Why Secure Data Sharing?

### The Problem

Traditional approaches to data sharing create compliance risks:

1. **Full access grants**: Giving third parties direct database access exposes all data
2. **Manual exports**: Ad-hoc CSV exports lack audit trails and consistency
3. **Application-level filtering**: Business logic can miss edge cases, leading to data leaks
4. **No consent tracking**: Difficult to prove what was shared and why

### The Solution

Craton treats data sharing as a first-class concern:

| Challenge | Craton Solution |
|-----------|-------------------|
| Over-exposure | Field-level access controls, scoped tokens |
| Audit gaps | Complete export audit trail with cryptographic proof |
| Consent tracking | Purpose and consent recorded for every export |
| Anonymization | Built-in redaction, generalization, pseudonymization |
| Revocation | Time-bound tokens, instant revocation |

---

## Architecture

### Data Sharing Layer

The data sharing layer sits between the protocol layer and the core database:

```
┌─────────────────────────────────────────────────────────────────────┐
│                         Data Sharing Layer                           │
│                                                                      │
│  ┌────────────────────────────────────────────────────────────────┐ │
│  │ 1. Access Control                                               │ │
│  │    • Validate access token                                      │ │
│  │    • Check scopes (tables, fields, time ranges)                │ │
│  │    • Verify consent/purpose                                     │ │
│  └────────────────────────────────────────────────────────────────┘ │
│                               ↓                                      │
│  ┌────────────────────────────────────────────────────────────────┐ │
│  │ 2. Query Rewriting                                              │ │
│  │    • Remove unauthorized fields from SELECT                     │ │
│  │    • Add filters for authorized time ranges                    │ │
│  │    • Enforce row-level security policies                       │ │
│  └────────────────────────────────────────────────────────────────┘ │
│                               ↓                                      │
│  ┌────────────────────────────────────────────────────────────────┐ │
│  │ 3. Transformation Pipeline                                      │ │
│  │    • Apply redaction rules                                      │ │
│  │    • Apply generalization rules                                 │ │
│  │    • Apply pseudonymization with tenant-specific keys          │ │
│  │    • Apply field-level encryption for recipients               │ │
│  └────────────────────────────────────────────────────────────────┘ │
│                               ↓                                      │
│  ┌────────────────────────────────────────────────────────────────┐ │
│  │ 4. Audit                                                        │ │
│  │    • Log export with hash of contents                          │ │
│  │    • Record recipient, purpose, timestamp                      │ │
│  │    • Generate cryptographic proof                               │ │
│  └────────────────────────────────────────────────────────────────┘ │
│                                                                      │
└─────────────────────────────────────────────────────────────────────┘
```

### Data Flow

```
┌──────────┐     ┌──────────────┐     ┌──────────────┐     ┌──────────┐
│ External │     │    Access    │     │ Transformation│     │  Core    │
│ Service  │────►│   Control    │────►│   Pipeline   │◄────│ Database │
└──────────┘     └──────────────┘     └──────────────┘     └──────────┘
     │                  │                    │                   │
     │                  │                    │                   │
     │           ┌──────┴───────┐     ┌──────┴───────┐          │
     │           │ Token Store  │     │ Audit Log    │          │
     │           └──────────────┘     └──────────────┘          │
     │                                       │                   │
     └───────────────────────────────────────┘
                    Audit Trail
```

---

## Anonymization Techniques

Craton supports multiple anonymization strategies, configurable per access token or export.

### Redaction

Complete removal of sensitive fields:

```rust
// Configuration
Transformation::Redact {
    fields: vec!["ssn", "credit_card", "password_hash"],
}

// Before: { "id": 1, "name": "Alice", "ssn": "123-45-6789" }
// After:  { "id": 1, "name": "Alice" }
```

**Use cases**: Untrusted third parties, public datasets, analytics where field is irrelevant

### Generalization

Reduce precision to prevent re-identification:

```rust
// Date generalization
Transformation::Generalize {
    field: "date_of_birth",
    precision: DatePrecision::Year,  // 1985-03-15 → 1985
}

// Numeric generalization
Transformation::Generalize {
    field: "age",
    precision: NumericPrecision::Range(10),  // 34 → "30-39"
}

// Geographic generalization
Transformation::Generalize {
    field: "zip_code",
    precision: ZipPrecision::First3,  // 94107 → "941"
}
```

**Use cases**: Research datasets, aggregate analytics, demographic analysis

### Pseudonymization

Replace identifiers with consistent tokens:

```rust
// Configuration
Transformation::Pseudonymize {
    fields: vec!["user_id", "email"],
    algorithm: PseudonymAlgorithm::Hmac {
        // Tokens are consistent within tenant, different across tenants
        key_derivation: KeyDerivation::TenantScoped,
    },
}

// Before: { "user_id": "alice@example.com", "action": "login" }
// After:  { "user_id": "tok_a7b3c9d2e1f0", "action": "login" }
```

**Properties**:
- **Consistent**: Same input always produces same token (within scope)
- **Reversible**: With the key, original value can be recovered
- **Scoped**: Different tenants get different tokens for same value

**Use cases**: Trusted partners, internal analytics, data that needs correlation

### Comparison

| Technique | Reversible | Maintains Relationships | Data Utility | Privacy Level |
|-----------|------------|------------------------|--------------|---------------|
| Redaction | No | No | Low | Highest |
| Generalization | No | Partial | Medium | High |
| Pseudonymization | With key | Yes | High | Medium |

---

## Token-Based Access Control

### Access Token Structure

```rust
struct AccessToken {
    /// Unique identifier for this token
    id: TokenId,

    /// Which tenant this token is for
    tenant_id: TenantId,

    /// Human-readable name/description
    name: String,

    /// What tables can be accessed
    allowed_tables: Vec<TableName>,

    /// What fields within those tables (if empty, all non-redacted)
    allowed_fields: Option<Vec<FieldName>>,

    /// Time range of data that can be accessed
    time_range: Option<(Timestamp, Timestamp)>,

    /// Maximum number of queries/exports
    max_operations: Option<u64>,

    /// When this token expires
    expires_at: Timestamp,

    /// Purpose of this access (for audit)
    purpose: String,

    /// Transformations to apply
    transformations: Vec<Transformation>,

    /// Who created this token
    created_by: ActorId,

    /// When it was created
    created_at: Timestamp,
}
```

### Creating Access Tokens

```rust
// Create a scoped token for analytics partner
let token = tenant.create_access_token(AccessTokenConfig {
    name: "Analytics Partner Q1 2024",

    // Scope: what data
    allowed_tables: vec!["orders", "products"],
    allowed_fields: Some(vec!["order_id", "product_id", "quantity", "region"]),
    time_range: Some((
        Timestamp::parse("2024-01-01"),
        Timestamp::parse("2024-03-31"),
    )),

    // Constraints
    expires_at: Timestamp::now() + Duration::days(30),
    max_operations: Some(1000),

    // Transformations
    transformations: vec![
        Transformation::Redact { fields: vec!["customer_email"] },
        Transformation::Generalize {
            field: "order_date",
            precision: DatePrecision::Week,
        },
    ],

    // Audit
    purpose: "Q1 2024 sales analysis per data sharing agreement #DSA-2024-001",
})?;
```

### Token Lifecycle

```
┌─────────────┐     ┌─────────────┐     ┌─────────────┐     ┌─────────────┐
│   Create    │────►│   Active    │────►│   Expired   │────►│   Deleted   │
└─────────────┘     └─────────────┘     └─────────────┘     └─────────────┘
                          │
                          │ (manual)
                          ▼
                    ┌─────────────┐
                    │   Revoked   │
                    └─────────────┘
```

### Revocation

Tokens can be revoked immediately:

```rust
// Revoke a specific token
tenant.revoke_access_token(token_id, "Security incident #INC-2024-042")?;

// Revoke all tokens for a purpose
tenant.revoke_tokens_by_purpose("Analytics Partner *")?;

// List all active tokens
let tokens = tenant.list_access_tokens(TokenFilter::Active)?;
```

---

## Export Audit Trail

### Audit Record Structure

Every data access is logged:

```rust
struct ExportAuditRecord {
    /// Unique export identifier
    export_id: ExportId,

    /// Which token was used
    token_id: TokenId,

    /// What was accessed
    query: String,  // or "bulk_export"
    tables_accessed: Vec<TableName>,
    row_count: u64,

    /// Transformations applied
    transformations_applied: Vec<Transformation>,

    /// Cryptographic proof
    content_hash: Hash,  // Hash of the exported data
    proof: ExportProof,  // Links to log position

    /// Metadata
    timestamp: Timestamp,
    client_ip: Option<IpAddr>,
    purpose: String,  // From token
}
```

### Querying the Audit Trail

```sql
-- All exports for a tenant
SELECT * FROM __export_audit
WHERE tenant_id = 123
ORDER BY timestamp DESC;

-- Exports by a specific token
SELECT * FROM __export_audit
WHERE token_id = 'tok_abc123';

-- Exports containing specific data
SELECT * FROM __export_audit
WHERE 'customers' = ANY(tables_accessed)
  AND timestamp > '2024-01-01';
```

### Cryptographic Proof

Each export includes a proof that can be verified:

```rust
struct ExportProof {
    /// Hash of exported content
    content_hash: Hash,

    /// Log position when export occurred
    log_position: LogPosition,

    /// Hash of log at that position
    log_hash: Hash,

    /// Signature from database
    signature: Signature,
}
```

This proves:
1. The exact data that was exported
2. When the export occurred (linked to log position)
3. The database attested to this export

---

## MCP Integration

### Overview

Craton will provide an MCP (Model Context Protocol) server for LLM and AI agent access. This allows AI systems to query data while respecting access controls.

### MCP Tools

```typescript
// Query tool - read data with automatic redaction
{
  name: "craton_query",
  description: "Query Craton with automatic access control enforcement",
  parameters: {
    sql: "string",      // The SQL query
    token: "string",    // Access token for authorization
  }
}

// Export tool - bulk data export
{
  name: "craton_export",
  description: "Export data from Craton with transformations",
  parameters: {
    tables: "string[]",
    format: "json | csv",
    token: "string",
  }
}

// Verify tool - verify data integrity
{
  name: "craton_verify",
  description: "Verify the integrity of previously exported data",
  parameters: {
    export_id: "string",
    content_hash: "string",
  }
}
```

### Safety Features

MCP access includes additional safety measures:

1. **Query validation**: Prevents data exfiltration patterns
2. **Rate limiting**: Per-token and global limits
3. **Result size limits**: Configurable maximum rows per query
4. **Audit emphasis**: All MCP access prominently logged

### Example MCP Session

```
User: What were our top 10 products last quarter?

LLM: [Calls craton_query with analytics token]

Craton:
- Validates token scope includes "products" and "orders"
- Rewrites query to exclude unauthorized fields
- Applies generalization to dates
- Logs the access
- Returns transformed results

LLM: Based on the data, your top products were...
```

---

## Compliance Considerations

### GDPR Data Sharing

| Requirement | Craton Feature |
|-------------|------------------|
| Lawful basis | Purpose tracking in tokens |
| Data minimization | Field-level scoping |
| Right to be informed | Export audit trail |
| Accountability | Cryptographic proof of exports |

### HIPAA Minimum Necessary

Craton enforces the "minimum necessary" standard:

- Tokens specify exactly which fields are needed
- Generalization reduces precision where full values aren't required
- Redaction removes fields entirely when not needed

### Audit Requirements

For regulated industries, Craton provides:

- Complete record of all data exports
- Cryptographic proof linking exports to database state
- Retention of audit records independent of data retention
- Export of audit trails for compliance review

---

## Summary

Craton's secure data sharing capabilities ensure that:

1. **Data is protected**: Anonymization techniques prevent over-exposure
2. **Access is controlled**: Scoped tokens limit what can be accessed
3. **Everything is audited**: Complete trail of all data sharing
4. **Compliance is maintained**: Built-in support for regulatory requirements

For implementation details, see:
- [ARCHITECTURE.md](ARCHITECTURE.md) - System architecture
- [COMPLIANCE.md](COMPLIANCE.md) - Regulatory compliance
- [PLAN.md](../PLAN.md) - Implementation roadmap (Phases 8-9)
