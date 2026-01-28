//! Audit logging for MCP tool invocations.
//!
//! All MCP tool calls are logged for compliance and security auditing.

use std::collections::HashMap;
use std::sync::RwLock;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use craton_sharing::TokenId;
use craton_types::TenantId;

/// A record of an MCP tool invocation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditRecord {
    /// Unique audit record ID.
    pub id: String,
    /// Timestamp of the invocation.
    pub timestamp: DateTime<Utc>,
    /// Token ID used for authentication.
    pub token_id: TokenId,
    /// Tenant ID.
    pub tenant_id: TenantId,
    /// Tool name invoked.
    pub tool_name: String,
    /// Whether the invocation succeeded.
    pub success: bool,
    /// Error message if failed.
    pub error: Option<String>,
    /// Tables accessed (for query/export tools).
    pub tables_accessed: Vec<String>,
    /// Number of rows returned.
    pub row_count: Option<usize>,
    /// Transformations applied.
    pub transformations: Vec<String>,
    /// Request duration in milliseconds.
    pub duration_ms: u64,
    /// Client identifier (if provided).
    pub client_id: Option<String>,
    /// Additional metadata.
    #[serde(default)]
    pub metadata: HashMap<String, String>,
}

impl AuditRecord {
    /// Creates a new audit record builder.
    pub fn builder(token_id: TokenId, tenant_id: TenantId, tool_name: &str) -> AuditRecordBuilder {
        AuditRecordBuilder {
            token_id,
            tenant_id,
            tool_name: tool_name.to_string(),
            tables_accessed: Vec::new(),
            transformations: Vec::new(),
            client_id: None,
            metadata: HashMap::new(),
        }
    }
}

/// Builder for audit records.
pub struct AuditRecordBuilder {
    token_id: TokenId,
    tenant_id: TenantId,
    tool_name: String,
    tables_accessed: Vec<String>,
    transformations: Vec<String>,
    client_id: Option<String>,
    metadata: HashMap<String, String>,
}

impl AuditRecordBuilder {
    /// Adds a table to the accessed tables list.
    pub fn table(mut self, table: &str) -> Self {
        self.tables_accessed.push(table.to_string());
        self
    }

    /// Adds multiple tables to the accessed tables list.
    pub fn tables(mut self, tables: &[String]) -> Self {
        self.tables_accessed.extend(tables.iter().cloned());
        self
    }

    /// Adds a transformation to the list.
    pub fn transformation(mut self, transform: &str) -> Self {
        self.transformations.push(transform.to_string());
        self
    }

    /// Adds multiple transformations.
    pub fn transformations(mut self, transforms: &[String]) -> Self {
        self.transformations.extend(transforms.iter().cloned());
        self
    }

    /// Sets the client ID.
    pub fn client_id(mut self, id: &str) -> Self {
        self.client_id = Some(id.to_string());
        self
    }

    /// Adds metadata.
    pub fn metadata(mut self, key: &str, value: &str) -> Self {
        self.metadata.insert(key.to_string(), value.to_string());
        self
    }

    /// Builds a successful audit record.
    pub fn success(self, row_count: usize, duration_ms: u64) -> AuditRecord {
        AuditRecord {
            id: generate_audit_id(),
            timestamp: Utc::now(),
            token_id: self.token_id,
            tenant_id: self.tenant_id,
            tool_name: self.tool_name,
            success: true,
            error: None,
            tables_accessed: self.tables_accessed,
            row_count: Some(row_count),
            transformations: self.transformations,
            duration_ms,
            client_id: self.client_id,
            metadata: self.metadata,
        }
    }

    /// Builds a failed audit record.
    pub fn failure(self, error: &str, duration_ms: u64) -> AuditRecord {
        AuditRecord {
            id: generate_audit_id(),
            timestamp: Utc::now(),
            token_id: self.token_id,
            tenant_id: self.tenant_id,
            tool_name: self.tool_name,
            success: false,
            error: Some(error.to_string()),
            tables_accessed: self.tables_accessed,
            row_count: None,
            transformations: self.transformations,
            duration_ms,
            client_id: self.client_id,
            metadata: self.metadata,
        }
    }
}

/// Generates a unique audit record ID.
fn generate_audit_id() -> String {
    use rand::Rng;
    use std::fmt::Write;
    let mut rng = rand::thread_rng();
    let bytes: [u8; 16] = rng.r#gen();
    let mut hex = String::with_capacity(32);
    for b in bytes {
        let _ = write!(hex, "{b:02x}");
    }
    format!("audit-{hex}")
}

/// In-memory audit log for testing and development.
///
/// In production, this would be backed by persistent storage.
pub struct AuditLog {
    records: RwLock<Vec<AuditRecord>>,
    max_records: usize,
}

impl AuditLog {
    /// Creates a new audit log with default capacity.
    pub fn new() -> Self {
        Self::with_capacity(10_000)
    }

    /// Creates a new audit log with specified capacity.
    pub fn with_capacity(max_records: usize) -> Self {
        Self {
            records: RwLock::new(Vec::with_capacity(max_records.min(1000))),
            max_records,
        }
    }

    /// Records an audit entry.
    pub fn record(&self, entry: AuditRecord) {
        if let Ok(mut records) = self.records.write() {
            // Evict oldest records if at capacity
            if records.len() >= self.max_records {
                let drain_count = self.max_records / 10; // Remove 10%
                records.drain(0..drain_count);
            }
            records.push(entry);
        }
    }

    /// Returns all records for a token.
    pub fn records_for_token(&self, token_id: &TokenId) -> Vec<AuditRecord> {
        self.records
            .read()
            .map(|records| {
                records
                    .iter()
                    .filter(|r| &r.token_id == token_id)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Returns all records for a tenant.
    pub fn records_for_tenant(&self, tenant_id: TenantId) -> Vec<AuditRecord> {
        self.records
            .read()
            .map(|records| {
                records
                    .iter()
                    .filter(|r| r.tenant_id == tenant_id)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Returns recent records (up to limit).
    pub fn recent(&self, limit: usize) -> Vec<AuditRecord> {
        self.records
            .read()
            .map(|records| {
                records
                    .iter()
                    .rev()
                    .take(limit)
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    }

    /// Returns the total number of records.
    pub fn len(&self) -> usize {
        self.records.read().map(|r| r.len()).unwrap_or(0)
    }

    /// Returns true if empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Default for AuditLog {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audit_record_builder() {
        let token_id = TokenId::generate();
        let tenant_id = TenantId::new(1);

        let record = AuditRecord::builder(token_id, tenant_id, "craton_query")
            .table("users")
            .transformation("mask:email")
            .success(42, 150);

        assert_eq!(record.tool_name, "craton_query");
        assert!(record.success);
        assert_eq!(record.row_count, Some(42));
        assert_eq!(record.tables_accessed, vec!["users"]);
        assert_eq!(record.transformations, vec!["mask:email"]);
    }

    #[test]
    fn test_audit_log() {
        let log = AuditLog::new();
        let token_id = TokenId::generate();
        let tenant_id = TenantId::new(1);

        let record = AuditRecord::builder(token_id, tenant_id, "craton_query").success(10, 50);

        log.record(record);

        assert_eq!(log.len(), 1);
        assert_eq!(log.records_for_tenant(tenant_id).len(), 1);
    }
}
