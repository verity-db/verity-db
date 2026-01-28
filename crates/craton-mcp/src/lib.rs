//! `Craton` MCP Server
//!
//! This crate implements a Model Context Protocol (MCP) server for `Craton`,
//! enabling secure LLM and third-party API access to database operations.
//!
//! # Overview
//!
//! The MCP server exposes the following tools:
//!
//! - **`craton_query`**: Execute SQL queries with automatic access control and transformation
//! - **`craton_export`**: Bulk data export with anonymization
//! - **`craton_verify`**: Verify integrity of previously exported data
//! - **`craton_list_tables`**: Discover available tables based on access scope
//!
//! # Security
//!
//! All tool invocations require a valid access token (from `craton-sharing`). The token
//! determines:
//!
//! - Which tables can be accessed
//! - Which fields are visible
//! - What transformations are applied (redaction, masking, pseudonymization, etc.)
//! - Maximum row limits
//!
//! # Audit Logging
//!
//! Every tool invocation is logged with:
//!
//! - Token ID and tenant
//! - Tool name and parameters
//! - Tables accessed
//! - Transformations applied
//! - Success/failure status
//! - Duration
//!
//! # Example
//!
//! ```ignore
//! use std::sync::Arc;
//! use craton::Craton;
//! use craton_sharing::TokenStore;
//! use craton_mcp::McpServer;
//!
//! // Create the MCP server
//! let db = Arc::new(Craton::open("./data")?);
//! let tokens = Arc::new(TokenStore::new());
//! let server = McpServer::with_db_and_tokens(db, tokens);
//!
//! // Handle a JSON-RPC request
//! let request = r#"{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}"#;
//! let response = server.handle_request(request);
//! ```
//!
//! # Protocol
//!
//! The server implements MCP over JSON-RPC 2.0. Supported methods:
//!
//! - `initialize`: Initialize the MCP session
//! - `tools/list`: List available tools
//! - `tools/call`: Execute a tool

mod audit;
mod error;
mod handler;
mod server;
mod tool;

pub use audit::{AuditLog, AuditRecord};
pub use error::{McpError, McpResult};
pub use handler::ToolHandler;
pub use server::{JsonRpcError, JsonRpcRequest, JsonRpcResponse, McpServer};
pub use tool::{
    ExportFormat, ExportInput, ExportMetadata, ExportOutput, ListTablesInput, ListTablesOutput,
    QueryInput, QueryOutput, TableInfo, ToolDefinition, VerifyInput, VerifyOutput, available_tools,
};

#[cfg(test)]
mod tests;
