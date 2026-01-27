//! MCP (Model Context Protocol) server.
//!
//! This module implements an MCP server that exposes RAG functionality
//! as tools to Claude Code through stdio communication.

use crate::error::{RagError, Result};
use crate::models::ContentType;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::{self, BufRead, BufReader, Write};

/// MCP protocol version.
const MCP_VERSION: &str = "2024-11-05";

/// MCP server for RAG functionality.
///
/// Exposes the following tools:
/// - `rag_query`: Query all indexed content
/// - `rag_search_code`: Search source code only
/// - `rag_search_docs`: Search documentation only
/// - `rag_search_session`: Search sessions only
/// - `rag_timeline`: Build feature timeline
pub struct McpServer;

impl McpServer {
    /// Create a new MCP server.
    pub fn new() -> Self {
        Self
    }

    /// Start the MCP server (stdio communication).
    ///
    /// Reads JSON-RPC requests from stdin and writes responses to stdout.
    pub async fn run(&self) -> Result<()> {
        let stdin = io::stdin();
        let stdout = io::stdout();
        let mut stdout = stdout.lock();
        let reader = BufReader::new(stdin);

        for line in reader.lines() {
            let line = line.map_err(RagError::Io)?;

            if line.trim().is_empty() {
                continue;
            }

            // Parse JSON-RPC request
            let request: JsonRpcRequest = serde_json::from_str(&line)
                .map_err(|e| RagError::Parse(format!("Invalid JSON-RPC: {}", e)))?;

            // Handle request
            let response = self.handle_request(&request)?;

            // Write response
            let response_json = serde_json::to_string(&response)
                .map_err(|e| RagError::Parse(format!("Failed to serialize response: {}", e)))?;

            writeln!(stdout, "{}", response_json)
                .map_err(RagError::Io)?;
            stdout.flush()
                .map_err(RagError::Io)?;
        }

        Ok(())
    }

    /// Handle an incoming JSON-RPC request.
    fn handle_request(&self, request: &JsonRpcRequest) -> Result<JsonRpcResponse> {
        match request.method.as_str() {
            "initialize" => self.handle_initialize(request),
            "tools/list" => self.handle_tools_list(request),
            "tools/call" => self.handle_tools_call(request),
            "ping" => Ok(JsonRpcResponse::success(request.id.clone(), json!({}))),
            _ => Ok(JsonRpcResponse::error(
                request.id.clone(),
                -32601,
                &format!("Method not found: {}", request.method),
            )),
        }
    }

    /// Handle initialize request.
    fn handle_initialize(&self, request: &JsonRpcRequest) -> Result<JsonRpcResponse> {
        let result = json!({
            "protocolVersion": MCP_VERSION,
            "capabilities": {
                "tools": {}
            },
            "serverInfo": {
                "name": "claude-rag",
                "version": env!("CARGO_PKG_VERSION")
            }
        });

        Ok(JsonRpcResponse::success(request.id.clone(), result))
    }

    /// Handle tools/list request.
    fn handle_tools_list(&self, request: &JsonRpcRequest) -> Result<JsonRpcResponse> {
        let tools = vec![
            self.tool_rag_query(),
            self.tool_rag_search_code(),
            self.tool_rag_search_docs(),
            self.tool_rag_search_session(),
            self.tool_rag_timeline(),
        ];

        let result = json!({ "tools": tools });
        Ok(JsonRpcResponse::success(request.id.clone(), result))
    }

    /// Handle tools/call request.
    fn handle_tools_call(&self, request: &JsonRpcRequest) -> Result<JsonRpcResponse> {
        let params = request.params.as_ref()
            .ok_or_else(|| RagError::Validation("Missing params".to_string()))?;

        let tool_name = params.get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| RagError::Validation("Missing tool name".to_string()))?;

        let arguments = params.get("arguments")
            .and_then(|v| v.as_object())
            .ok_or_else(|| RagError::Validation("Missing arguments".to_string()))?;

        let result = match tool_name {
            "rag_query" => self.call_rag_query(arguments)?,
            "rag_search_code" => self.call_rag_search(arguments, ContentType::File)?,
            "rag_search_docs" => self.call_rag_search(arguments, ContentType::File)?,
            "rag_search_session" => self.call_rag_search(arguments, ContentType::Session)?,
            "rag_timeline" => self.call_rag_timeline(arguments)?,
            _ => {
                return Ok(JsonRpcResponse::error(
                    request.id.clone(),
                    -32602,
                    &format!("Unknown tool: {}", tool_name),
                ));
            }
        };

        Ok(JsonRpcResponse::success(request.id.clone(), result))
    }

    /// Call rag_query tool.
    fn call_rag_query(&self, args: &serde_json::Map<String, Value>) -> Result<Value> {
        let query = args.get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| RagError::Validation("Missing 'query' parameter".to_string()))?;

        let top_k = args.get("top_k")
            .and_then(|v| v.as_u64())
            .unwrap_or(5) as usize;

        // TODO: Implement actual query
        let results = format!("Query: '{}', Top-K: {} (NOT YET IMPLEMENTED)", query, top_k);

        Ok(json!({
            "content": [
                {
                    "type": "text",
                    "text": results
                }
            ]
        }))
    }

    /// Call rag_search_* tools.
    fn call_rag_search(&self, args: &serde_json::Map<String, Value>, _content_type: ContentType) -> Result<Value> {
        let query = args.get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| RagError::Validation("Missing 'query' parameter".to_string()))?;

        let top_k = args.get("top_k")
            .and_then(|v| v.as_u64())
            .unwrap_or(10) as usize;

        // TODO: Implement actual filtered query
        let results = format!("Search: '{}', Top-K: {} (NOT YET IMPLEMENTED)", query, top_k);

        Ok(json!({
            "content": [
                {
                    "type": "text",
                    "text": results
                }
            ]
        }))
    }

    /// Call rag_timeline tool.
    fn call_rag_timeline(&self, args: &serde_json::Map<String, Value>) -> Result<Value> {
        let feature = args.get("feature")
            .and_then(|v| v.as_str());

        let top_k = args.get("top_k")
            .and_then(|v| v.as_u64())
            .unwrap_or(50) as usize;

        // TODO: Implement actual timeline building
        let timeline = if let Some(feat) = feature {
            format!("Timeline for feature: '{}', Top-K: {} (NOT YET IMPLEMENTED)", feat, top_k)
        } else {
            format!("General project timeline, Top-K: {} (NOT YET IMPLEMENTED)", top_k)
        };

        Ok(json!({
            "content": [
                {
                    "type": "text",
                    "text": timeline
                }
            ]
        }))
    }

    /// Generate MCP configuration for Claude Code settings.json.
    pub fn generate_config(&self) -> Result<String> {
        let config = json!({
            "mcpServers": {
                "claude-rag": {
                    "command": "claude-rag",
                    "args": ["mcp-server"]
                }
            }
        });

        serde_json::to_string_pretty(&config)
            .map_err(|e| RagError::Parse(format!("Failed to generate config: {}", e)))
    }

    // Tool definitions

    fn tool_rag_query(&self) -> Value {
        json!({
            "name": "rag_query",
            "description": "Query the RAG knowledge base for relevant context from all indexed content (sessions, files, git history)",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "The search query"
                    },
                    "top_k": {
                        "type": "number",
                        "description": "Number of results to return (default: 5)",
                        "default": 5
                    }
                },
                "required": ["query"]
            }
        })
    }

    fn tool_rag_search_code(&self) -> Value {
        json!({
            "name": "rag_search_code",
            "description": "Search only source code in the RAG knowledge base",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "The search query for source code"
                    },
                    "top_k": {
                        "type": "number",
                        "description": "Number of results to return (default: 10)",
                        "default": 10
                    }
                },
                "required": ["query"]
            }
        })
    }

    fn tool_rag_search_docs(&self) -> Value {
        json!({
            "name": "rag_search_docs",
            "description": "Search only documentation in the RAG knowledge base",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "The search query for documentation"
                    },
                    "top_k": {
                        "type": "number",
                        "description": "Number of results to return (default: 5)",
                        "default": 5
                    }
                },
                "required": ["query"]
            }
        })
    }

    fn tool_rag_search_session(&self) -> Value {
        json!({
            "name": "rag_search_session",
            "description": "Search only previous Claude sessions in the RAG knowledge base",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "The search query for sessions"
                    },
                    "top_k": {
                        "type": "number",
                        "description": "Number of results to return (default: 5)",
                        "default": 5
                    }
                },
                "required": ["query"]
            }
        })
    }

    fn tool_rag_timeline(&self) -> Value {
        json!({
            "name": "rag_timeline",
            "description": "Build a feature timeline from git history and sessions",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "feature": {
                        "type": "string",
                        "description": "Optional feature name to filter timeline (default: general project timeline)"
                    },
                    "top_k": {
                        "type": "number",
                        "description": "Number of events to include (default: 50)",
                        "default": 50
                    }
                }
            }
        })
    }
}

impl Default for McpServer {
    fn default() -> Self {
        Self::new()
    }
}

/// JSON-RPC request.
#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    #[allow(dead_code)]
    jsonrpc: String,
    id: Value,
    method: String,
    #[serde(default)]
    params: Option<Value>,
}

/// JSON-RPC response.
#[derive(Debug, Serialize)]
struct JsonRpcResponse {
    jsonrpc: String,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

impl JsonRpcResponse {
    fn success(id: Value, result: Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: Some(result),
            error: None,
        }
    }

    fn error(id: Value, code: i32, message: &str) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: None,
            error: Some(JsonRpcError {
                code,
                message: message.to_string(),
            }),
        }
    }
}

/// JSON-RPC error.
#[derive(Debug, Serialize)]
struct JsonRpcError {
    code: i32,
    message: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mcp_server_new() {
        let _server = McpServer::new();
    }

    #[test]
    fn test_mcp_server_default() {
        let _server = McpServer::default();
    }

    #[test]
    fn test_generate_config() {
        let server = McpServer::new();
        let config = server.generate_config().unwrap();

        assert!(config.contains("claude-rag"));
        assert!(config.contains("mcp-server"));
    }

    #[test]
    fn test_tool_rag_query_definition() {
        let server = McpServer::new();
        let tool = server.tool_rag_query();

        assert_eq!(tool["name"], "rag_query");
        assert!(tool["description"].is_string());
        assert!(tool["inputSchema"]["properties"]["query"].is_object());
        assert!(tool["inputSchema"]["properties"]["top_k"].is_object());
    }

    #[test]
    fn test_tool_rag_search_code_definition() {
        let server = McpServer::new();
        let tool = server.tool_rag_search_code();

        assert_eq!(tool["name"], "rag_search_code");
        assert!(tool["description"].is_string());
    }

    #[test]
    fn test_tool_rag_search_docs_definition() {
        let server = McpServer::new();
        let tool = server.tool_rag_search_docs();

        assert_eq!(tool["name"], "rag_search_docs");
    }

    #[test]
    fn test_tool_rag_search_session_definition() {
        let server = McpServer::new();
        let tool = server.tool_rag_search_session();

        assert_eq!(tool["name"], "rag_search_session");
    }

    #[test]
    fn test_tool_rag_timeline_definition() {
        let server = McpServer::new();
        let tool = server.tool_rag_timeline();

        assert_eq!(tool["name"], "rag_timeline");
        assert!(tool["inputSchema"]["properties"]["feature"].is_object());
    }

    #[test]
    fn test_handle_initialize() {
        let server = McpServer::new();
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: json!(1),
            method: "initialize".to_string(),
            params: None,
        };

        let response = server.handle_initialize(&request).unwrap();
        assert!(response.result.is_some());
        assert!(response.error.is_none());
        assert_eq!(response.result.unwrap()["protocolVersion"], MCP_VERSION);
    }

    #[test]
    fn test_handle_tools_list() {
        let server = McpServer::new();
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: json!(2),
            method: "tools/list".to_string(),
            params: None,
        };

        let response = server.handle_tools_list(&request).unwrap();
        assert!(response.result.is_some());
        let result = response.result.unwrap();
        let tools = result["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 5);
    }

    #[test]
    fn test_handle_unknown_method() {
        let server = McpServer::new();
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: json!(3),
            method: "unknown_method".to_string(),
            params: None,
        };

        let response = server.handle_request(&request).unwrap();
        assert!(response.error.is_some());
        assert_eq!(response.error.unwrap().code, -32601);
    }

    #[test]
    fn test_call_rag_query() {
        let server = McpServer::new();
        let mut args = serde_json::Map::new();
        args.insert("query".to_string(), json!("test query"));
        args.insert("top_k".to_string(), json!(10));

        let result = server.call_rag_query(&args).unwrap();
        assert!(result["content"].is_array());
    }

    #[test]
    fn test_call_rag_query_missing_query() {
        let server = McpServer::new();
        let args = serde_json::Map::new();

        let result = server.call_rag_query(&args);
        assert!(result.is_err());
    }

    #[test]
    fn test_call_rag_timeline_with_feature() {
        let server = McpServer::new();
        let mut args = serde_json::Map::new();
        args.insert("feature".to_string(), json!("authentication"));

        let result = server.call_rag_timeline(&args).unwrap();
        assert!(result["content"].is_array());
    }

    #[test]
    fn test_call_rag_timeline_without_feature() {
        let server = McpServer::new();
        let args = serde_json::Map::new();

        let result = server.call_rag_timeline(&args).unwrap();
        assert!(result["content"].is_array());
    }
}
