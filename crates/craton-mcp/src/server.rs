//! MCP server implementation.
//!
//! Implements the Model Context Protocol (MCP) for LLM tool access.
//! Uses JSON-RPC 2.0 for the transport layer.

use std::sync::{Arc, RwLock};

use serde::{Deserialize, Serialize};
use craton::Craton;
use craton_sharing::TokenStore;

use crate::audit::AuditLog;
use crate::error::{McpError, McpResult};
use crate::handler::ToolHandler;
use crate::tool::{
    ExportInput, ListTablesInput, QueryInput, ToolDefinition, VerifyInput, available_tools,
};

/// JSON-RPC 2.0 request.
#[derive(Debug, Clone, Deserialize)]
pub struct JsonRpcRequest {
    /// JSON-RPC version (must be "2.0").
    pub jsonrpc: String,
    /// Request ID.
    pub id: serde_json::Value,
    /// Method name.
    pub method: String,
    /// Method parameters.
    #[serde(default)]
    pub params: serde_json::Value,
}

/// JSON-RPC 2.0 response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    /// JSON-RPC version.
    pub jsonrpc: String,
    /// Request ID (matches request).
    pub id: serde_json::Value,
    /// Result (present on success).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    /// Error (present on failure).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

/// JSON-RPC 2.0 error.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcError {
    /// Error code.
    pub code: i32,
    /// Error message.
    pub message: String,
    /// Additional data.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

impl JsonRpcResponse {
    /// Creates a success response.
    pub fn success(id: serde_json::Value, result: serde_json::Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: Some(result),
            error: None,
        }
    }

    /// Creates an error response.
    pub fn error(id: serde_json::Value, code: i32, message: String) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: None,
            error: Some(JsonRpcError {
                code,
                message,
                data: None,
            }),
        }
    }

    /// Creates an error response from an `McpError`.
    pub fn from_mcp_error(id: serde_json::Value, error: &McpError) -> Self {
        Self::error(id, error.error_code(), error.to_string())
    }
}

/// MCP server that handles tool invocations.
pub struct McpServer {
    /// Tool handler for executing tools.
    handler: ToolHandler,
}

impl McpServer {
    /// Creates a new MCP server.
    pub fn new(db: Arc<Craton>, tokens: Arc<RwLock<TokenStore>>, audit: Arc<AuditLog>) -> Self {
        Self {
            handler: ToolHandler::new(db, tokens, audit),
        }
    }

    /// Creates an MCP server with a new audit log.
    pub fn with_db_and_tokens(db: Arc<Craton>, tokens: Arc<RwLock<TokenStore>>) -> Self {
        Self::new(db, tokens, Arc::new(AuditLog::new()))
    }

    /// Handles a JSON-RPC request and returns a response.
    pub fn handle_request(&self, request: &str) -> String {
        // Parse the request
        let parsed: Result<JsonRpcRequest, _> = serde_json::from_str(request);

        let response = match parsed {
            Ok(req) => self.dispatch(req),
            Err(e) => JsonRpcResponse::error(
                serde_json::Value::Null,
                -32700, // Parse error
                format!("parse error: {e}"),
            ),
        };

        serde_json::to_string(&response).unwrap_or_else(|_| {
            r#"{"jsonrpc":"2.0","id":null,"error":{"code":-32603,"message":"serialization failed"}}"#.to_string()
        })
    }

    /// Dispatches a request to the appropriate handler.
    fn dispatch(&self, request: JsonRpcRequest) -> JsonRpcResponse {
        // Validate JSON-RPC version
        if request.jsonrpc != "2.0" {
            return JsonRpcResponse::error(
                request.id,
                -32600, // Invalid request
                "invalid JSON-RPC version".to_string(),
            );
        }

        match request.method.as_str() {
            // MCP initialization
            "initialize" => self.handle_initialize(request.id),

            // List available tools
            "tools/list" => self.handle_list_tools(request.id),

            // Execute a tool
            "tools/call" => self.handle_tool_call(request.id, &request.params),

            // Unknown method
            _ => JsonRpcResponse::error(
                request.id,
                -32601, // Method not found
                format!("unknown method: {}", request.method),
            ),
        }
    }

    /// Handles the initialize request.
    #[allow(clippy::unused_self)]
    fn handle_initialize(&self, id: serde_json::Value) -> JsonRpcResponse {
        let result = serde_json::json!({
            "protocolVersion": "2024-11-05",
            "serverInfo": {
                "name": "craton-mcp",
                "version": env!("CARGO_PKG_VERSION")
            },
            "capabilities": {
                "tools": {}
            }
        });

        JsonRpcResponse::success(id, result)
    }

    /// Handles the tools/list request.
    #[allow(clippy::unused_self)]
    fn handle_list_tools(&self, id: serde_json::Value) -> JsonRpcResponse {
        let tools: Vec<serde_json::Value> = available_tools()
            .into_iter()
            .map(|t| tool_to_json(&t))
            .collect();

        let result = serde_json::json!({
            "tools": tools
        });

        JsonRpcResponse::success(id, result)
    }

    /// Handles the tools/call request.
    fn handle_tool_call(
        &self,
        id: serde_json::Value,
        params: &serde_json::Value,
    ) -> JsonRpcResponse {
        // Extract tool name and arguments
        let tool_name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");

        let arguments = params
            .get("arguments")
            .cloned()
            .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));

        // Execute the tool
        let result = self.execute_tool(tool_name, arguments);

        match result {
            Ok(output) => {
                let result = serde_json::json!({
                    "content": [{
                        "type": "text",
                        "text": serde_json::to_string_pretty(&output).unwrap_or_default()
                    }]
                });
                JsonRpcResponse::success(id, result)
            }
            Err(e) => JsonRpcResponse::from_mcp_error(id, &e),
        }
    }

    /// Executes a tool by name.
    fn execute_tool(
        &self,
        name: &str,
        arguments: serde_json::Value,
    ) -> McpResult<serde_json::Value> {
        match name {
            "craton_query" => {
                let input: QueryInput = serde_json::from_value(arguments)?;
                let output = self.handler.execute_query(&input)?;
                Ok(serde_json::to_value(output)?)
            }
            "craton_export" => {
                let input: ExportInput = serde_json::from_value(arguments)?;
                let output = self.handler.execute_export(&input)?;
                Ok(serde_json::to_value(output)?)
            }
            "craton_verify" => {
                let input: VerifyInput = serde_json::from_value(arguments)?;
                let output = self.handler.execute_verify(&input)?;
                Ok(serde_json::to_value(output)?)
            }
            "craton_list_tables" => {
                let input: ListTablesInput = serde_json::from_value(arguments)?;
                let output = self.handler.execute_list_tables(&input)?;
                Ok(serde_json::to_value(output)?)
            }
            _ => Err(McpError::UnknownTool(name.to_string())),
        }
    }
}

/// Converts a tool definition to JSON for the MCP protocol.
fn tool_to_json(tool: &ToolDefinition) -> serde_json::Value {
    serde_json::json!({
        "name": tool.name,
        "description": tool.description,
        "inputSchema": tool.input_schema
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_test_server() -> (McpServer, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let db = Arc::new(Craton::open(temp_dir.path()).unwrap());
        let tokens = Arc::new(RwLock::new(TokenStore::new()));
        (McpServer::with_db_and_tokens(db, tokens), temp_dir)
    }

    #[test]
    fn test_initialize() {
        let (server, _temp) = create_test_server();

        let request = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#;
        let response = server.handle_request(request);

        let parsed: JsonRpcResponse = serde_json::from_str(&response).unwrap();
        assert!(parsed.error.is_none());
        assert!(parsed.result.is_some());

        let result = parsed.result.unwrap();
        assert_eq!(result["protocolVersion"], "2024-11-05");
        assert_eq!(result["serverInfo"]["name"], "craton-mcp");
    }

    #[test]
    fn test_list_tools() {
        let (server, _temp) = create_test_server();

        let request = r#"{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}"#;
        let response = server.handle_request(request);

        let parsed: JsonRpcResponse = serde_json::from_str(&response).unwrap();
        assert!(parsed.error.is_none());

        let result = parsed.result.unwrap();
        let tools = result["tools"].as_array().unwrap();
        assert!(!tools.is_empty());

        // Check that craton_query tool is present
        let has_query = tools.iter().any(|t| t["name"] == "craton_query");
        assert!(has_query);
    }

    #[test]
    fn test_unknown_method() {
        let (server, _temp) = create_test_server();

        let request = r#"{"jsonrpc":"2.0","id":3,"method":"unknown/method","params":{}}"#;
        let response = server.handle_request(request);

        let parsed: JsonRpcResponse = serde_json::from_str(&response).unwrap();
        assert!(parsed.error.is_some());
        assert_eq!(parsed.error.unwrap().code, -32601);
    }

    #[test]
    fn test_invalid_json() {
        let (server, _temp) = create_test_server();

        let request = "not valid json";
        let response = server.handle_request(request);

        let parsed: JsonRpcResponse = serde_json::from_str(&response).unwrap();
        assert!(parsed.error.is_some());
        assert_eq!(parsed.error.unwrap().code, -32700);
    }

    #[test]
    fn test_tool_call_without_token() {
        let (server, _temp) = create_test_server();

        let request = r#"{
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": {
                "name": "craton_query",
                "arguments": {
                    "sql": "SELECT * FROM users"
                }
            }
        }"#;
        let response = server.handle_request(request);

        let parsed: JsonRpcResponse = serde_json::from_str(&response).unwrap();
        // Should fail due to missing token
        assert!(parsed.error.is_some());
    }
}
