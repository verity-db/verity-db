//! Integration tests for the MCP server.

use std::sync::{Arc, RwLock};

use chrono::TimeDelta;
use tempfile::TempDir;
use craton::Craton;
use craton_sharing::{AccessToken, ExportScope, TokenStore};
use craton_types::TenantId;

use crate::{AuditLog, McpServer, ToolHandler};

/// Creates a test environment with a database and token store.
fn setup() -> (Arc<Craton>, Arc<RwLock<TokenStore>>, Arc<AuditLog>, TempDir) {
    let temp_dir = TempDir::new().unwrap();
    let db = Arc::new(Craton::open(temp_dir.path()).unwrap());
    let tokens = Arc::new(RwLock::new(TokenStore::new()));
    let audit = Arc::new(AuditLog::new());
    (db, tokens, audit, temp_dir)
}

#[test]
fn test_mcp_server_creation() {
    let (db, tokens, audit, _temp) = setup();
    let _server = McpServer::new(db, tokens, audit);
}

#[test]
fn test_tool_handler_creation() {
    let (db, tokens, audit, _temp) = setup();
    let _handler = ToolHandler::new(db, tokens, audit);
}

#[test]
fn test_mcp_initialize_flow() {
    let (db, tokens, _, _temp) = setup();
    let server = McpServer::with_db_and_tokens(db, tokens);

    // Initialize
    let init_request = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#;
    let response = server.handle_request(init_request);
    assert!(response.contains("protocolVersion"));
    assert!(response.contains("craton-mcp"));

    // List tools
    let list_request = r#"{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}"#;
    let response = server.handle_request(list_request);
    assert!(response.contains("craton_query"));
    assert!(response.contains("craton_export"));
    assert!(response.contains("craton_verify"));
    assert!(response.contains("craton_list_tables"));
}

#[test]
fn test_token_validation() {
    let (db, tokens, audit, _temp) = setup();
    let server = McpServer::new(db, tokens.clone(), audit);

    // Create a valid token
    let tenant_id = TenantId::new(1);
    let scope = ExportScope::tables(["users"]);
    let token = AccessToken::new(tenant_id, "test token", scope, TimeDelta::seconds(3600));
    let token_id = tokens.write().unwrap().issue(token.clone());
    let token_str = token_id.to_hex();

    // Try to list tables with the token
    let request = format!(
        r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"name":"craton_list_tables","arguments":{{"token":"{}"}}}}}}"#,
        token_str
    );
    let response = server.handle_request(&request);

    // Should succeed (empty tables since we haven't created any)
    assert!(response.contains("tables"));
}

#[test]
fn test_query_safety_validation() {
    let (db, tokens, audit, _temp) = setup();
    let server = McpServer::new(db, tokens.clone(), audit);

    // Create a token
    let tenant_id = TenantId::new(1);
    let scope = ExportScope::tables(["users"]);
    let token = AccessToken::new(tenant_id, "test token", scope, TimeDelta::seconds(3600));
    let token_id = tokens.write().unwrap().issue(token.clone());
    let token_str = token_id.to_hex();

    // Try a dangerous query
    let request = format!(
        r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"name":"craton_query","arguments":{{"sql":"DROP TABLE users","token":"{}"}}}}}}"#,
        token_str
    );
    let response = server.handle_request(&request);

    // Should be rejected
    assert!(response.contains("error"));
    assert!(response.contains("blocked pattern") || response.contains("SELECT"));
}

#[test]
fn test_audit_logging() {
    let (db, tokens, audit, _temp) = setup();
    let server = McpServer::new(db, tokens.clone(), audit.clone());

    // Create a token
    let tenant_id = TenantId::new(1);
    let scope = ExportScope::tables(["users"]);
    let token = AccessToken::new(tenant_id, "test token", scope, TimeDelta::seconds(3600));
    let token_id = tokens.write().unwrap().issue(token.clone());
    let token_str = token_id.to_hex();

    // Make a request
    let request = format!(
        r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"name":"craton_list_tables","arguments":{{"token":"{}"}}}}}}"#,
        token_str
    );
    let _ = server.handle_request(&request);

    // Check audit log
    let records = audit.records_for_tenant(tenant_id);
    assert!(!records.is_empty());
    assert_eq!(records[0].tool_name, "craton_list_tables");
}

#[test]
fn test_export_format_options() {
    // Test that both JSON and CSV formats are supported
    use crate::tool::ExportFormat;

    let json_format: ExportFormat = serde_json::from_str(r#""json""#).unwrap();
    let csv_format: ExportFormat = serde_json::from_str(r#""csv""#).unwrap();

    assert_eq!(json_format, ExportFormat::Json);
    assert_eq!(csv_format, ExportFormat::Csv);
}
