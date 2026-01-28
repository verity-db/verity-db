//! MCP tool definitions.
//!
//! Defines the tools available to MCP clients for interacting with `Craton`.

use serde::{Deserialize, Serialize};

/// Tool definitions exposed by the MCP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    /// Unique tool name.
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// JSON Schema for the tool's input parameters.
    pub input_schema: serde_json::Value,
}

/// All available tools.
pub fn available_tools() -> Vec<ToolDefinition> {
    vec![
        query_tool(),
        export_tool(),
        verify_tool(),
        list_tables_tool(),
    ]
}

/// Query tool - execute SQL queries with automatic access control.
pub fn query_tool() -> ToolDefinition {
    ToolDefinition {
        name: "craton_query".to_string(),
        description: "Execute a SQL query against Craton with automatic access control \
                      enforcement. Results are automatically filtered and transformed \
                      based on your access token's scope."
            .to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "sql": {
                    "type": "string",
                    "description": "The SQL query to execute. Supports SELECT with WHERE, ORDER BY, and LIMIT."
                },
                "token": {
                    "type": "string",
                    "description": "Access token for authorization."
                },
                "params": {
                    "type": "array",
                    "description": "Optional query parameters for parameterized queries.",
                    "items": {
                        "type": ["string", "number", "boolean", "null"]
                    }
                }
            },
            "required": ["sql", "token"]
        }),
    }
}

/// Export tool - bulk data export with transformations.
pub fn export_tool() -> ToolDefinition {
    ToolDefinition {
        name: "craton_export".to_string(),
        description: "Export data from Craton tables with automatic anonymization and \
                      transformation based on your access token's scope. Returns data in \
                      the requested format with a content hash for verification."
            .to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "tables": {
                    "type": "array",
                    "description": "List of table names to export.",
                    "items": {"type": "string"}
                },
                "format": {
                    "type": "string",
                    "description": "Output format.",
                    "enum": ["json", "csv"]
                },
                "token": {
                    "type": "string",
                    "description": "Access token for authorization."
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of rows to export per table.",
                    "minimum": 1,
                    "maximum": 10000
                }
            },
            "required": ["tables", "format", "token"]
        }),
    }
}

/// Verify tool - verify integrity of previously exported data.
pub fn verify_tool() -> ToolDefinition {
    ToolDefinition {
        name: "craton_verify".to_string(),
        description: "Verify the integrity of previously exported data by checking its \
                      content hash against the audit log. Returns verification status \
                      and the original export metadata."
            .to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "export_id": {
                    "type": "string",
                    "description": "The export ID returned from a previous craton_export call."
                },
                "content_hash": {
                    "type": "string",
                    "description": "The content hash to verify (hex-encoded SHA-256)."
                },
                "token": {
                    "type": "string",
                    "description": "Access token for authorization."
                }
            },
            "required": ["export_id", "content_hash", "token"]
        }),
    }
}

/// List tables tool - discover available tables.
pub fn list_tables_tool() -> ToolDefinition {
    ToolDefinition {
        name: "craton_list_tables".to_string(),
        description: "List the tables available to query based on your access token's scope. \
                      Returns table names and their accessible columns."
            .to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "token": {
                    "type": "string",
                    "description": "Access token for authorization."
                }
            },
            "required": ["token"]
        }),
    }
}

/// Input for the query tool.
#[derive(Debug, Clone, Deserialize)]
pub struct QueryInput {
    /// SQL query to execute.
    pub sql: String,
    /// Access token.
    pub token: String,
    /// Optional query parameters.
    #[serde(default)]
    pub params: Vec<serde_json::Value>,
}

/// Input for the export tool.
#[derive(Debug, Clone, Deserialize)]
pub struct ExportInput {
    /// Tables to export.
    pub tables: Vec<String>,
    /// Output format.
    pub format: ExportFormat,
    /// Access token.
    pub token: String,
    /// Maximum rows per table.
    pub limit: Option<u32>,
}

/// Export output format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ExportFormat {
    /// JSON format.
    Json,
    /// CSV format.
    Csv,
}

/// Input for the verify tool.
#[derive(Debug, Clone, Deserialize)]
pub struct VerifyInput {
    /// Export ID to verify.
    pub export_id: String,
    /// Content hash to check.
    pub content_hash: String,
    /// Access token.
    pub token: String,
}

/// Input for the list tables tool.
#[derive(Debug, Clone, Deserialize)]
pub struct ListTablesInput {
    /// Access token.
    pub token: String,
}

/// Output from the query tool.
#[derive(Debug, Clone, Serialize)]
pub struct QueryOutput {
    /// Column names.
    pub columns: Vec<String>,
    /// Row data.
    pub rows: Vec<Vec<serde_json::Value>>,
    /// Number of rows returned.
    pub row_count: usize,
    /// Whether results were truncated due to limits.
    pub truncated: bool,
    /// Transformations that were applied.
    pub transformations_applied: Vec<String>,
}

/// Output from the export tool.
#[derive(Debug, Clone, Serialize)]
pub struct ExportOutput {
    /// Unique export ID for verification.
    pub export_id: String,
    /// Content hash (SHA-256, hex-encoded).
    pub content_hash: String,
    /// Exported data.
    pub data: serde_json::Value,
    /// Export metadata.
    pub metadata: ExportMetadata,
}

/// Metadata about an export.
#[derive(Debug, Clone, Serialize)]
pub struct ExportMetadata {
    /// Tables exported.
    pub tables: Vec<String>,
    /// Total row count.
    pub total_rows: usize,
    /// Export timestamp (ISO 8601).
    pub exported_at: String,
    /// Transformations applied.
    pub transformations: Vec<String>,
}

/// Output from the verify tool.
#[derive(Debug, Clone, Serialize)]
pub struct VerifyOutput {
    /// Whether the hash matches.
    pub verified: bool,
    /// Original export metadata (if found).
    pub export_metadata: Option<ExportMetadata>,
    /// Verification message.
    pub message: String,
}

/// Output from the list tables tool.
#[derive(Debug, Clone, Serialize)]
pub struct ListTablesOutput {
    /// Available tables.
    pub tables: Vec<TableInfo>,
}

/// Information about an accessible table.
#[derive(Debug, Clone, Serialize)]
pub struct TableInfo {
    /// Table name.
    pub name: String,
    /// Accessible columns.
    pub columns: Vec<String>,
    /// Whether any columns are transformed.
    pub has_transformations: bool,
}
