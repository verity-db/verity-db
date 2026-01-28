//! Tool execution handler.
//!
//! Executes MCP tool invocations with scope enforcement and audit logging.

use std::sync::{Arc, RwLock};
use std::time::Instant;

use craton::Craton;
use craton_crypto::{HashPurpose, hash_with_purpose};
use craton_query::Value;
use craton_sharing::{ExportScope, TokenId, TokenStore, TransformationType};
use craton_types::TenantId;

use crate::audit::{AuditLog, AuditRecord};
use crate::error::{McpError, McpResult};
use crate::tool::{
    ExportFormat, ExportInput, ExportMetadata, ExportOutput, ListTablesInput, ListTablesOutput,
    QueryInput, QueryOutput, TableInfo, VerifyInput, VerifyOutput,
};

/// Maximum rows returned by a query (can be overridden by scope).
const DEFAULT_MAX_ROWS: usize = 1000;

/// Handler for executing MCP tool invocations.
pub struct ToolHandler {
    /// The Craton database instance.
    db: Arc<Craton>,
    /// Token store for validating access tokens (requires interior mutability).
    tokens: Arc<RwLock<TokenStore>>,
    /// Audit log for recording invocations.
    audit: Arc<AuditLog>,
}

impl ToolHandler {
    /// Creates a new tool handler.
    pub fn new(db: Arc<Craton>, tokens: Arc<RwLock<TokenStore>>, audit: Arc<AuditLog>) -> Self {
        Self { db, tokens, audit }
    }

    /// Executes the query tool.
    pub fn execute_query(&self, input: &QueryInput) -> McpResult<QueryOutput> {
        let start = Instant::now();

        // Validate and get token
        let (token_id, tenant_id, scope) = self.validate_token(&input.token)?;

        // Build audit record
        let mut audit_builder = AuditRecord::builder(token_id, tenant_id, "craton_query");

        // Validate query safety
        self.validate_query_safety(&input.sql)?;

        // Extract tables from query (simplified - real impl would use query parser)
        let tables = extract_tables_from_query(&input.sql);
        audit_builder = audit_builder.tables(&tables);

        // Check scope allows these tables
        for table in &tables {
            if !scope.allows_table(table) {
                let error = format!("access denied for table: {table}");
                self.audit
                    .record(audit_builder.failure(&error, start.elapsed().as_millis() as u64));
                return Err(McpError::ScopeViolation(error));
            }
        }

        // Execute query
        let tenant = self.db.tenant(tenant_id);
        let params = convert_params(&input.params);

        let result = tenant
            .query(&input.sql, &params)
            .map_err(|e| McpError::Query(e.to_string()))?;

        // Apply transformations based on scope
        let (rows, transformations) = apply_scope_transformations(&result, &scope);

        // Enforce row limit
        let max_rows = scope.max_rows.map_or(DEFAULT_MAX_ROWS, |m| m as usize);
        let truncated = rows.len() > max_rows;
        let rows: Vec<Vec<serde_json::Value>> = rows.into_iter().take(max_rows).collect();

        // Record success
        audit_builder = audit_builder.transformations(&transformations);
        self.audit
            .record(audit_builder.success(rows.len(), start.elapsed().as_millis() as u64));

        Ok(QueryOutput {
            columns: result.columns.iter().map(ToString::to_string).collect(),
            row_count: rows.len(),
            rows,
            truncated,
            transformations_applied: transformations,
        })
    }

    /// Executes the export tool.
    pub fn execute_export(&self, input: &ExportInput) -> McpResult<ExportOutput> {
        let start = Instant::now();

        // Validate and get token
        let (token_id, tenant_id, scope) = self.validate_token(&input.token)?;

        // Build audit record
        let mut audit_builder =
            AuditRecord::builder(token_id, tenant_id, "craton_export").tables(&input.tables);

        // Check scope allows these tables
        for table in &input.tables {
            if !scope.allows_table(table) {
                let error = format!("access denied for table: {table}");
                self.audit
                    .record(audit_builder.failure(&error, start.elapsed().as_millis() as u64));
                return Err(McpError::ScopeViolation(error));
            }
        }

        // Export each table
        let tenant = self.db.tenant(tenant_id);
        let limit = input.limit.unwrap_or(1000).min(10000) as usize;
        let max_rows = scope.max_rows.map_or(limit, |m| m as usize).min(limit);

        let mut all_data = serde_json::Map::new();
        let mut total_rows = 0;
        let mut all_transformations = Vec::new();

        for table in &input.tables {
            // Query the table
            let sql = format!("SELECT * FROM {table} LIMIT {max_rows}");
            let result = tenant
                .query(&sql, &[])
                .map_err(|e| McpError::Query(e.to_string()))?;

            // Apply transformations
            let (rows, transforms) = apply_scope_transformations(&result, &scope);
            total_rows += rows.len();

            for t in transforms {
                if !all_transformations.contains(&t) {
                    all_transformations.push(t);
                }
            }

            // Format based on requested format
            let table_data = match input.format {
                ExportFormat::Json => {
                    let columns: Vec<String> =
                        result.columns.iter().map(ToString::to_string).collect();
                    let records: Vec<serde_json::Value> = rows
                        .into_iter()
                        .map(|row| {
                            let obj: serde_json::Map<String, serde_json::Value> = columns
                                .iter()
                                .zip(row.into_iter())
                                .map(|(k, v)| (k.clone(), v))
                                .collect();
                            serde_json::Value::Object(obj)
                        })
                        .collect();
                    serde_json::Value::Array(records)
                }
                ExportFormat::Csv => {
                    let columns: Vec<String> =
                        result.columns.iter().map(ToString::to_string).collect();
                    let mut csv = columns.join(",") + "\n";
                    for row in rows {
                        let line: Vec<String> = row
                            .into_iter()
                            .map(|v| match v {
                                serde_json::Value::String(s) => {
                                    // Escape quotes in CSV
                                    format!("\"{}\"", s.replace('"', "\"\""))
                                }
                                serde_json::Value::Null => String::new(),
                                other => other.to_string(),
                            })
                            .collect();
                        csv.push_str(&line.join(","));
                        csv.push('\n');
                    }
                    serde_json::Value::String(csv)
                }
            };

            all_data.insert(table.clone(), table_data);
        }

        // Generate export ID and content hash
        let data = serde_json::Value::Object(all_data);
        let data_bytes = serde_json::to_vec(&data)?;
        let (_alg, hash_bytes) = hash_with_purpose(HashPurpose::Compliance, &data_bytes);
        let export_id = generate_export_id();

        // Record success
        audit_builder = audit_builder.transformations(&all_transformations);
        self.audit
            .record(audit_builder.success(total_rows, start.elapsed().as_millis() as u64));

        Ok(ExportOutput {
            export_id,
            content_hash: hex_encode(&hash_bytes),
            data,
            metadata: ExportMetadata {
                tables: input.tables.clone(),
                total_rows,
                exported_at: chrono::Utc::now().to_rfc3339(),
                transformations: all_transformations,
            },
        })
    }

    /// Executes the verify tool.
    pub fn execute_verify(&self, input: &VerifyInput) -> McpResult<VerifyOutput> {
        let start = Instant::now();

        // Validate token (just for authentication)
        let (token_id, tenant_id, _scope) = self.validate_token(&input.token)?;

        // Build audit record
        let audit_builder = AuditRecord::builder(token_id, tenant_id, "craton_verify")
            .metadata("export_id", &input.export_id);

        // In a real implementation, we would look up the export_id in a persistent store
        // For now, we just validate the hash format and return a placeholder response
        let hash_valid = input.content_hash.len() == 64
            && input.content_hash.chars().all(|c| c.is_ascii_hexdigit());

        if !hash_valid {
            self.audit.record(
                audit_builder.failure("invalid hash format", start.elapsed().as_millis() as u64),
            );
            return Ok(VerifyOutput {
                verified: false,
                export_metadata: None,
                message: "Invalid content hash format. Expected 64-character hex string."
                    .to_string(),
            });
        }

        // Record success
        self.audit
            .record(audit_builder.success(0, start.elapsed().as_millis() as u64));

        Ok(VerifyOutput {
            verified: true,
            export_metadata: None,
            message: "Hash format is valid. Full verification requires export registry lookup."
                .to_string(),
        })
    }

    /// Executes the list tables tool.
    pub fn execute_list_tables(&self, input: &ListTablesInput) -> McpResult<ListTablesOutput> {
        let start = Instant::now();

        // Validate and get token
        let (token_id, tenant_id, scope) = self.validate_token(&input.token)?;

        // Build audit record
        let audit_builder = AuditRecord::builder(token_id, tenant_id, "craton_list_tables");

        // Get tables from scope
        let tables: Vec<TableInfo> = scope
            .tables
            .iter()
            .map(|table| {
                let columns: Vec<String> = scope.fields.iter().cloned().collect();
                let has_transformations = !scope.transformations.is_empty();
                TableInfo {
                    name: table.clone(),
                    columns,
                    has_transformations,
                }
            })
            .collect();

        // Record success
        self.audit
            .record(audit_builder.success(tables.len(), start.elapsed().as_millis() as u64));

        Ok(ListTablesOutput { tables })
    }

    /// Validates an access token and returns token info with its scope.
    fn validate_token(&self, token_str: &str) -> McpResult<(TokenId, TenantId, ExportScope)> {
        // Parse token ID from hex string
        let token_id = parse_token_id(token_str)?;

        // Get the token and validate it
        let mut store = self
            .tokens
            .write()
            .map_err(|_| McpError::Internal("token store lock poisoned".to_string()))?;

        let scope = store.use_token(token_id).map_err(|e| match e {
            craton_sharing::SharingError::TokenExpired => McpError::TokenExpired,
            craton_sharing::SharingError::TokenRevoked => McpError::TokenRevoked,
            craton_sharing::SharingError::TokenNotFound => {
                McpError::AuthenticationFailed("token not found".to_string())
            }
            _ => McpError::AuthenticationFailed(e.to_string()),
        })?;

        // Get tenant ID from the token
        let token = store
            .get(token_id)
            .ok_or_else(|| McpError::AuthenticationFailed("token not found".to_string()))?;
        let tenant_id = token.tenant_id;

        Ok((token_id, tenant_id, scope))
    }

    /// Validates query safety to prevent data exfiltration patterns.
    #[allow(clippy::unused_self)]
    fn validate_query_safety(&self, sql: &str) -> McpResult<()> {
        let sql_upper = sql.to_uppercase();

        // Block dangerous patterns
        let blocked_patterns = [
            "DROP ",
            "DELETE ",
            "UPDATE ",
            "INSERT ",
            "ALTER ",
            "CREATE ",
            "TRUNCATE ",
            "GRANT ",
            "REVOKE ",
            "INTO OUTFILE",
            "INTO DUMPFILE",
            "LOAD_FILE",
            "INFORMATION_SCHEMA",
            "SLEEP(",
            "BENCHMARK(",
            "UNION ALL SELECT",
        ];

        for pattern in &blocked_patterns {
            if sql_upper.contains(pattern) {
                return Err(McpError::QueryRejected(format!(
                    "query contains blocked pattern: {pattern}"
                )));
            }
        }

        // Must be a SELECT query
        let trimmed = sql_upper.trim();
        if !trimmed.starts_with("SELECT") {
            return Err(McpError::QueryRejected(
                "only SELECT queries are allowed".to_string(),
            ));
        }

        Ok(())
    }
}

/// Parses a token ID from a hex string.
fn parse_token_id(s: &str) -> McpResult<TokenId> {
    if s.len() != 32 {
        return Err(McpError::AuthenticationFailed(
            "invalid token format: expected 32 hex characters".to_string(),
        ));
    }

    let mut bytes = [0u8; 16];
    for (i, chunk) in s.as_bytes().chunks(2).enumerate() {
        let hex_str = std::str::from_utf8(chunk)
            .map_err(|_| McpError::AuthenticationFailed("invalid token encoding".to_string()))?;
        bytes[i] = u8::from_str_radix(hex_str, 16)
            .map_err(|_| McpError::AuthenticationFailed("invalid token hex".to_string()))?;
    }

    Ok(TokenId::from_bytes(bytes))
}

/// Extracts table names from a SQL query (simplified implementation).
fn extract_tables_from_query(sql: &str) -> Vec<String> {
    let sql_upper = sql.to_uppercase();
    let mut tables = Vec::new();

    // Simple pattern: find "FROM <table>" and "JOIN <table>"
    for keyword in ["FROM ", "JOIN "] {
        if let Some(idx) = sql_upper.find(keyword) {
            let after = &sql[idx + keyword.len()..];
            let table: String = after
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if !table.is_empty() && !tables.contains(&table) {
                tables.push(table);
            }
        }
    }

    tables
}

/// Converts JSON parameters to query values.
fn convert_params(params: &[serde_json::Value]) -> Vec<Value> {
    params
        .iter()
        .map(|p| match p {
            serde_json::Value::Null => Value::Null,
            serde_json::Value::Bool(b) => Value::Boolean(*b),
            serde_json::Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    Value::BigInt(i)
                } else {
                    Value::Text(n.to_string())
                }
            }
            serde_json::Value::String(s) => Value::Text(s.clone()),
            _ => Value::Text(p.to_string()),
        })
        .collect()
}

/// Applies scope transformations to query results.
fn apply_scope_transformations(
    result: &craton_query::QueryResult,
    scope: &ExportScope,
) -> (Vec<Vec<serde_json::Value>>, Vec<String>) {
    let mut transformations = Vec::new();

    let rows: Vec<Vec<serde_json::Value>> = result
        .rows
        .iter()
        .map(|row| {
            row.iter()
                .enumerate()
                .map(|(i, value)| {
                    let column = result.columns.get(i).map_or("", craton::ColumnName::as_str);

                    // Check if this field should be transformed
                    if let Some(rule) = scope
                        .transformations
                        .iter()
                        .find(|r| r.matches_field(column))
                    {
                        let transform_name =
                            format!("{}:{}", transformation_name(&rule.transformation), column);
                        if !transformations.contains(&transform_name) {
                            transformations.push(transform_name);
                        }
                        apply_transform(value, &rule.transformation)
                    } else {
                        value_to_json(value)
                    }
                })
                .collect()
        })
        .collect();

    (rows, transformations)
}

/// Returns a human-readable name for a transformation type.
fn transformation_name(t: &TransformationType) -> &'static str {
    match t {
        TransformationType::Redact => "redact",
        TransformationType::Mask => "mask",
        TransformationType::PartialMask { .. } => "partial_mask",
        TransformationType::Pseudonymize => "pseudonymize",
        TransformationType::Generalize { .. } => "generalize",
        TransformationType::Hash => "hash",
        TransformationType::Encrypt => "encrypt",
    }
}

/// Applies a single transformation to a value.
fn apply_transform(value: &Value, transform: &TransformationType) -> serde_json::Value {
    match transform {
        TransformationType::Redact => serde_json::Value::Null,
        TransformationType::Mask => {
            let s = value_to_string(value);
            serde_json::Value::String("*".repeat(s.len()))
        }
        TransformationType::PartialMask {
            show_start,
            show_end,
        } => {
            let s = value_to_string(value);
            let len = s.len();
            if len <= show_start + show_end {
                serde_json::Value::String("*".repeat(len))
            } else {
                let first: String = s.chars().take(*show_start).collect();
                let last: String = s.chars().skip(len - show_end).collect();
                let middle = "*".repeat(len - show_start - show_end);
                serde_json::Value::String(format!("{first}{middle}{last}"))
            }
        }
        TransformationType::Pseudonymize => {
            use std::fmt::Write;
            let s = value_to_string(value);
            let (_alg, hash) = hash_with_purpose(HashPurpose::Internal, s.as_bytes());
            let mut hex = String::with_capacity(16);
            for b in hash.iter().take(8) {
                let _ = write!(hex, "{b:02x}");
            }
            serde_json::Value::String(format!("PSEUDO-{hex}"))
        }
        TransformationType::Generalize { bucket_size } => {
            if let Value::BigInt(n) = value {
                let size = bucket_size.unwrap_or(10) as i64;
                let bucket = (*n / size) * size;
                let next = bucket + size;
                serde_json::Value::String(format!("{bucket}-{}", next - 1))
            } else {
                value_to_json(value)
            }
        }
        TransformationType::Hash => {
            let s = value_to_string(value);
            let (_alg, hash) = hash_with_purpose(HashPurpose::Compliance, s.as_bytes());
            serde_json::Value::String(hex_encode(&hash))
        }
        TransformationType::Encrypt => {
            // Encryption placeholder - would need key management
            serde_json::Value::String("[ENCRYPTED]".to_string())
        }
    }
}

/// Converts a Value to a JSON value.
fn value_to_json(value: &Value) -> serde_json::Value {
    match value {
        Value::Null => serde_json::Value::Null,
        Value::BigInt(n) => serde_json::Value::Number((*n).into()),
        Value::Text(s) => serde_json::Value::String(s.clone()),
        Value::Boolean(b) => serde_json::Value::Bool(*b),
        Value::Timestamp(t) => {
            // Convert timestamp nanoseconds to ISO 8601 format
            let secs = t.as_nanos() / 1_000_000_000;
            let nanos = t.as_nanos() % 1_000_000_000;
            serde_json::Value::String(format!("{secs}.{nanos:09}"))
        }
        Value::Bytes(b) => {
            use base64::Engine;
            let encoded = base64::engine::general_purpose::STANDARD.encode(b);
            serde_json::Value::String(encoded)
        }
    }
}

/// Converts a Value to a string for transformation.
fn value_to_string(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::BigInt(n) => n.to_string(),
        Value::Text(s) => s.clone(),
        Value::Boolean(b) => b.to_string(),
        Value::Timestamp(t) => {
            // Convert timestamp nanoseconds to string
            t.as_nanos().to_string()
        }
        Value::Bytes(b) => {
            use base64::Engine;
            base64::engine::general_purpose::STANDARD.encode(b)
        }
    }
}

/// Generates a unique export ID.
fn generate_export_id() -> String {
    use rand::Rng;
    use std::fmt::Write;
    let mut rng = rand::thread_rng();
    let bytes: [u8; 16] = rng.r#gen();
    let mut hex = String::with_capacity(32);
    for b in bytes {
        let _ = write!(hex, "{b:02x}");
    }
    format!("export-{hex}")
}

/// Hex encodes bytes.
fn hex_encode(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut hex = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(hex, "{b:02x}");
    }
    hex
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::RwLock;
    use tempfile::TempDir;

    fn setup() -> (Arc<Craton>, Arc<RwLock<TokenStore>>, Arc<AuditLog>) {
        let temp_dir = TempDir::new().unwrap();
        let db = Arc::new(Craton::open(temp_dir.path()).unwrap());
        let tokens = Arc::new(RwLock::new(TokenStore::new()));
        let audit = Arc::new(AuditLog::new());
        (db, tokens, audit)
    }

    #[test]
    fn test_query_safety_blocks_dangerous_patterns() {
        let (db, tokens, audit) = setup();
        let handler = ToolHandler::new(db, tokens, audit);

        assert!(handler.validate_query_safety("SELECT * FROM users").is_ok());
        assert!(handler.validate_query_safety("DROP TABLE users").is_err());
        assert!(handler.validate_query_safety("DELETE FROM users").is_err());
        assert!(
            handler
                .validate_query_safety("SELECT * FROM users; DROP TABLE users")
                .is_err()
        );
    }

    #[test]
    fn test_extract_tables() {
        let tables = extract_tables_from_query("SELECT * FROM users WHERE id = 1");
        assert_eq!(tables, vec!["users"]);

        let tables = extract_tables_from_query(
            "SELECT * FROM users JOIN orders ON users.id = orders.user_id",
        );
        assert_eq!(tables, vec!["users", "orders"]);
    }

    #[test]
    fn test_parse_token_id() {
        let hex = "0123456789abcdef0123456789abcdef";
        let token_id = parse_token_id(hex).unwrap();
        assert_eq!(token_id.to_hex(), hex);
    }

    #[test]
    fn test_parse_token_id_invalid() {
        assert!(parse_token_id("short").is_err());
        assert!(parse_token_id("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz").is_err());
    }
}
