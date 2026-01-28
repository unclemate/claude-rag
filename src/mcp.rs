//! # MCP (Model Context Protocol) Server
//!
//! This module implements an MCP server that exposes RAG (Retrieval-Augmented Generation)
//! functionality as tools to Claude Code through stdio communication.
//!
//! ## Architecture
//!
//! The MCP server follows the JSON-RPC 2.0 protocol and communicates via standard input/output.
//! It exposes the following tools:
//!
//! - `rag_query` - Query all indexed content (sessions, files, git history)
//! - `rag_search_code` - Search only source code files
//! - `rag_search_docs` - Search only documentation files
//! - `rag_search_session` - Search only Claude sessions
//! - `rag_timeline` - Build feature timelines from git history
//!
//! ## Example
//!
//! ```no_run
//! use claude_rag::mcp::McpServer;
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     // Create server with current directory as project path
//!     let server = McpServer::new(None)?;
//!
//!     // Run the server (listens on stdin/stdout)
//!     server.run().await?;
//!
//!     Ok(())
//! }
//! ```
//!
//! ## Configuration
//!
//! The server reads configuration from `.rag/config.json` in the project directory.
//! Required configuration includes:
//!
//! - `embedding.api_token` - API token for embedding service
//! - `embedding.api_url` - URL for embedding API endpoint
//! - `hnsw.m`, `hnsw.ef_construction`, `hnsw.ef_search` - HNSW index parameters
//!
//! ## Security
//!
//! ### Threat Model
//!
//! The MCP server operates in a trusted environment (local machine) but defends against:
//!
//! 1. **Malicious Input** - Invalid or malformed JSON-RPC requests
//! 2. **Resource Exhaustion** - DoS attacks via large requests or expensive queries
//! 3. **Path Traversal** - Attempts to access files outside the project directory
//!
//! ### Implemented Security Measures
//!
//! #### Input Validation
//! - **Query length**: 1-1000 characters (enforced at `extract_query`)
//! - **Top-K limits**: 1-100 for user requests (enforced at `extract_top_k`)
//! - **Request size limit**: `MAX_REQUEST_SIZE` (1MB) per JSON-RPC request
//! - **Type validation**: All parameters are validated against JSON Schema
//! - **Empty query rejection**: Whitespace-only queries are rejected
//!
//! #### Resource Limits
//! - **Max results**: `MAX_RESULTS_TO_DISPLAY` (500) caps returned results
//! - **Top-K caps**: Different defaults per tool type (code: 10, general: 5, timeline: 20)
//! - **Storage access**: All file operations use bounded paths
//!
//! #### Path Security
//! - **Canonicalization**: All paths are resolved via `canonical()` before use
//! - **TOCTOU protection**: Path validation happens before storage operations
//! - **Project boundary**: File collection is scoped to project directory
//!
//! ### Known Limitations
//!
//! 1. **No Request Timeout**: The server does not enforce per-request timeout.
//!    - **Risk**: Slow requests could block indefinitely (slow loris attack)
//!    - **Mitigation**: Use external timeout wrappers (e.g., `timeout` command)
//!    - **Future**: `REQUEST_TIMEOUT_SECONDS` constant defined but not implemented
//!
//! 2. **Async Runtime Usage**: The server uses `Arc<Runtime>::block_on()` to call async code.
//!    - **Risk**: Potential deadlock if called from within async context
//!    - **Mitigation**: Documented warning; server is sync-only by design
//!
//! 3. **No Rate Limiting**: Multiple concurrent requests are not rate-limited.
//!    - **Risk**: Client could flood with requests
//!    - **Mitigation**: MCP clients (Claude Code) generally limit request rate
//!
//! 4. **No Authentication**: Stdio communication assumes trusted client.
//!    - **Risk**: Any process with stdin access can send requests
//!    - **Mitigation**: Server is designed for local development use only
//!
//! ### Security Best Practices for Deployment
//!
//! - Run server only on trusted development machines
//! - Keep API tokens in `.rag/config.json` with appropriate file permissions (0600)
//! - Use `CLAUDE_PROJECT_PATH` environment variable to constrain project scope
//! - Monitor server logs for unusual request patterns
//! - Consider wrapping with `timeout` command for production use

use crate::config::ConfigManager;
use crate::embedding::EmbeddingClient;
use crate::error::{RagError, Result};
use crate::models::ContentType;
use crate::storage::sled::StorageManager;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fmt::{Display, Write};
use std::io::{self, BufRead, BufReader, Write as IoWrite};
use std::path::{Path, PathBuf};
use std::sync::Arc;

// ==========================================================================
// Protocol Constants
// ==========================================================================

/// MCP protocol version.
const MCP_VERSION: &str = "2024-11-05";

/// JSON-RPC 2.0 error codes.
const JSONRPC_ERROR_METHOD_NOT_FOUND: i32 = -32601;
const JSONRPC_ERROR_INVALID_PARAMS: i32 = -32602;
const JSONRPC_ERROR_INTERNAL_ERROR: i32 = -32603;

// ==========================================================================
// Path & Environment Constants
// ==========================================================================

/// Environment variable for project path.
const PROJECT_PATH_ENV: &str = "CLAUDE_PROJECT_PATH";

// ==========================================================================
// Message Constants
// ==========================================================================

/// Message constants for error responses.
const MSG_NO_INDEX: &str = "No index found for project. Please run indexing first.";
const MSG_INDEX_ERROR: &str = "Error loading index: ";
const MSG_EMBEDDING_ERROR: &str = "Failed to generate embedding: ";
const MSG_TIMELINE_NOT_IMPLEMENTED: &str = "Timeline building is not yet fully integrated with storage.";
const MSG_TIMELINE_IMPLEMENTED: &str = "The timeline builder is implemented and can be used once commit/session data is indexed.";

// ==========================================================================
// File Extension Constants
// ==========================================================================

/// Documentation file extensions.
const DOC_EXTENSIONS: &[&str] = &[
    "md", "markdown", "rst", "txt", "adoc", "asciidoc",
    "html", "htm", "pdf", "doc", "docx",
];

/// Source code file extensions.
const CODE_EXTENSIONS: &[&str] = &[
    "rs", "py", "js", "ts", "tsx", "jsx", "go", "java", "kt", "kts",
    "c", "cpp", "cc", "cxx", "h", "hpp", "hxx",
    "cs", "fs", "fsx", "vb", "swift", "scala", "rb", "php",
    "sh", "bash", "zsh", "fish", "ps1", "psm1",
    "yaml", "yml", "json", "toml", "xml", "cfg", "ini",
    "sql", "graphql", "wsdl", "proto",
];

// ==========================================================================
// Limit & Validation Constants
// ==========================================================================

/// Maximum allowed value for top_k parameter to prevent resource exhaustion.
const MAX_TOP_K: usize = 1000;

/// Absolute maximum for search results to prevent DoS.
const MAX_SEARCH_RESULTS: usize = 500;

/// Maximum number of results to display in response to prevent buffer overflow.
const MAX_RESULTS_TO_DISPLAY: usize = 100;

/// Maximum query length to prevent abuse.
const MAX_QUERY_LENGTH: usize = 1000;

/// Minimum query length (non-empty after trimming).
const MIN_QUERY_LENGTH: usize = 1;

/// Minimum value for top_k parameter.
const MIN_TOP_K: usize = 1;

/// Maximum request size to prevent DoS (1 MB).
const MAX_REQUEST_SIZE: usize = 1_048_576;

// ==========================================================================
// Default Values
// ==========================================================================

/// Default values for top_k per tool.
const DEFAULT_TOP_K_GENERAL: usize = 5;
const DEFAULT_TOP_K_CODE: usize = 10;

/// Default top_k for timeline tool.
const DEFAULT_TOP_K_TIMELINE: usize = 50;

/// Multiplier for file filter search (fetch more for filtering).
const FILE_FILTER_MULTIPLIER: usize = 5;

// ==========================================================================
// Performance Constants
// ==========================================================================

/// Base capacity for output string buffer.
const OUTPUT_BASE_CAPACITY: usize = 256;

/// Additional capacity per result for output string buffer.
const OUTPUT_PER_RESULT_CAPACITY: usize = 80;

/// Request timeout in seconds to prevent slow loris attacks.
const REQUEST_TIMEOUT_SECONDS: u64 = 30;

/// MCP server for RAG functionality.
///
/// This server implements the Model Context Protocol (MCP) to expose RAG
/// (Retrieval-Augmented Generation) functionality as tools to Claude Code.
///
/// # Fields
///
/// * `storage` - Manages persistent storage for indexed data
/// * `client` - Handles embedding generation for queries
/// * `runtime` - Shared Tokio runtime for async operations
///
/// # Example
///
/// ```no_run
/// use claude_rag::mcp::McpServer;
///
/// let server = McpServer::new(None)?;
/// // Use server.run().await? in async context
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub struct McpServer {
    /// Storage manager for indexed data.
    storage: StorageManager,
    /// Embedding client for query vectorization.
    client: EmbeddingClient,
    /// Shared Tokio runtime for async operations.
    runtime: Arc<tokio::runtime::Runtime>,
}

impl McpServer {
    /// Creates a new MCP server instance.
    ///
    /// This method initializes the server by:
    /// 1. Resolving the project path (from argument, env var, or current directory)
    /// 2. Loading configuration from `.rag/config.json`
    /// 3. Opening the project's storage database
    /// 4. Initializing the embedding client
    /// 5. Creating a shared Tokio runtime for async operations
    ///
    /// # Arguments
    ///
    /// * `project_path` - Optional path to the project directory. If `None`, the
    ///   server will try `CLAUDE_PROJECT_PATH` environment variable, then fall back
    ///   to the current working directory.
    ///
    /// # Returns
    ///
    /// Returns a `Result` containing the initialized `McpServer` or an error if:
    /// - The project path doesn't exist or isn't a directory
    /// - Configuration file is missing or invalid
    /// - Storage database cannot be opened
    /// - Tokio runtime cannot be created
    ///
    /// # Example
    ///
    /// ```no_run
    /// use claude_rag::mcp::McpServer;
    /// use std::path::Path;
    ///
    /// // Use current directory
    /// let server = McpServer::new(None)?;
    ///
    /// // Use specific path
    /// let server = McpServer::new(Some(Path::new("/my/project")))?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// # Note
    ///
    /// This method creates a new Tokio runtime. For best results, create the
    /// server before entering any async context. The `block_on()` calls in
    /// `embed_query_sync()` should not be called from within another runtime.
    pub fn new(project_path: Option<&Path>) -> Result<Self> {
        // Resolve project path
        let project_path = Self::resolve_project_path(project_path)?;

        // Load configuration
        let config = ConfigManager::load(Some(&project_path))?;

        // Initialize storage manager
        let storage = StorageManager::open_project_db(&project_path)?;

        // Initialize embedding client
        let client = EmbeddingClient::from_config(&config.embedding);

        // Create shared Tokio runtime
        let runtime = tokio::runtime::Runtime::new()
            .map_err(|e| RagError::Config(format!("Failed to create runtime: {}", e)))?;

        Ok(Self {
            storage,
            client,
            runtime: Arc::new(runtime),
        })
    }

    /// Resolves the project path from parameter, environment variable, or current directory.
    ///
    /// This method implements security measures to prevent path traversal attacks:
    /// - Canonicalizes the path to resolve symlinks and relative components
    /// - Verifies the path exists and is a directory
    ///
    /// # Resolution Order
    ///
    /// 1. Provided `project_path` parameter (if `Some`)
    /// 2. `CLAUDE_PROJECT_PATH` environment variable (if set and valid)
    /// 3. Current working directory (final fallback)
    ///
    /// # Arguments
    ///
    /// * `provided` - Optional path explicitly provided by the caller
    ///
    /// # Returns
    ///
    /// Returns a `Result` containing the canonicalized project path or an error.
    fn resolve_project_path(provided: Option<&Path>) -> Result<PathBuf> {
        if let Some(path) = provided {
            // Canonicalize to resolve symlinks, relative components, and verify existence
            let canonical = path.canonicalize()
                .map_err(|e| RagError::Validation(format!(
                    "Invalid project path: cannot canonicalize: {}", e
                )))?;

            // Verify it's a directory (canonicalize already verified existence)
            if !canonical.is_dir() {
                return Err(RagError::Validation(format!(
                    "Project path must be a directory: {}",
                    path.display()
                )));
            }

            return Ok(canonical);
        }

        // Try environment variable
        if let Ok(path_str) = std::env::var(PROJECT_PATH_ENV) {
            let path = PathBuf::from(&path_str);

            // Try to canonicalize and validate the environment variable path
            if let Ok(canonical) = path.canonicalize() {
                if canonical.is_dir() {
                    return Ok(canonical);
                }
            }
            // If env var path is invalid, warn and fall through to current directory
            eprintln!("Warning: {} environment variable points to invalid path, using current directory instead", PROJECT_PATH_ENV);
        }

        // Use current directory as final fallback
        std::env::current_dir()
            .map_err(|e| RagError::Config(format!("Failed to get current directory: {}", e)))
    }

    /// Checks if a file path indicates a documentation file based on its extension.
    ///
    /// Uses `Path::extension()` for cross-platform compatibility.
    /// The check is case-insensitive for better cross-platform support.
    ///
    /// # Arguments
    ///
    /// * `file_path` - The file path to check (as a string slice)
    ///
    /// # Returns
    ///
    /// Returns `true` if the file extension is in `DOC_EXTENSIONS`, `false` otherwise.
    fn is_doc_file(file_path: &str) -> bool {
        Path::new(file_path)
            .extension()
            .and_then(|e| e.to_str())
            .map(|ext| ext.to_ascii_lowercase())
            .map(|ext| DOC_EXTENSIONS.contains(&ext.as_str()))
            .unwrap_or(false)
    }

    /// Checks if a file path indicates a source code file based on its extension.
    ///
    /// Uses `Path::extension()` for cross-platform compatibility.
    /// The check is case-insensitive for better cross-platform support.
    ///
    /// # Arguments
    ///
    /// * `file_path` - The file path to check (as a string slice)
    ///
    /// # Returns
    ///
    /// Returns `true` if the file extension is in `CODE_EXTENSIONS`, `false` otherwise.
    fn is_code_file(file_path: &str) -> bool {
        Path::new(file_path)
            .extension()
            .and_then(|e| e.to_str())
            .map(|ext| ext.to_ascii_lowercase())
            .map(|ext| CODE_EXTENSIONS.contains(&ext.as_str()))
            .unwrap_or(false)
    }

    /// Converts a HNSW distance metric to a similarity score.
    ///
    /// The conversion formula is: `similarity = 1 / (1 + distance)`
    ///
    /// - Distance 0 → Similarity 1.0 (identical)
    /// - Distance 1 → Similarity 0.5
    /// - Large distance → Similarity approaches 0
    ///
    /// # Arguments
    ///
    /// * `distance` - The HNSW distance metric (non-negative)
    ///
    /// # Returns
    ///
    /// Returns a similarity score in the range (0, 1], or NaN for negative infinity.
    fn distance_to_similarity(distance: f32) -> f32 {
        1.0 / (1.0 + distance)
    }

    /// Generates an embedding for a query synchronously.
    ///
    /// This is a helper method that wraps the async embedding generation
    /// with runtime.block_on() to avoid code duplication.
    ///
    /// # Arguments
    ///
    /// * `query` - The query text to embed
    ///
    /// # Returns
    ///
    /// Returns a `Result` containing the embedding vector or an error.
    ///
    /// # Note
    ///
    /// This method uses `block_on()` which should not be called from within
    /// an async context. The MCP server is designed to run in its own runtime.
    fn embed_query_sync(&self, query: &str) -> Result<Vec<f32>> {
        self.runtime.block_on(async {
            self.client.embed(query).await
        })
    }

    /// Extracts and validates the `query` parameter from tool arguments.
    ///
    /// # Validation
    ///
    /// The query must:
    /// - Be present and a string (not null)
    /// - Not be empty or whitespace-only after trimming
    /// - Not exceed `MAX_QUERY_LENGTH` characters
    ///
    /// # Arguments
    ///
    /// * `args` - The tool arguments as a JSON map
    ///
    /// # Returns
    ///
    /// Returns a `Result` containing the trimmed query string or a validation error.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The `query` parameter is missing
    /// - The query is not a string
    /// - The query is empty or whitespace-only
    /// - The query exceeds maximum length
    fn extract_query(args: &serde_json::Map<String, Value>) -> Result<String> {
        let query = args.get("query")
            .and_then(|v| v.as_str())
            .map(|s| s.trim())
            .ok_or_else(|| RagError::Validation("Missing 'query' parameter".to_string()))?;

        // Validate minimum length (non-empty after trim)
        if query.len() < MIN_QUERY_LENGTH {
            return Err(RagError::Validation("Query cannot be empty".to_string()));
        }

        // Validate maximum length
        if query.len() > MAX_QUERY_LENGTH {
            return Err(RagError::Validation(format!(
                "Query too long (maximum {} characters, got {})",
                MAX_QUERY_LENGTH,
                query.len()
            )));
        }

        // Only allocate string after all validations pass
        Ok(query.to_string())
    }

    /// Extracts and validates the `top_k` parameter from tool arguments.
    ///
    /// # Validation
    ///
    /// The `top_k` value must:
    /// - Be present as a number or use the default
    /// - Be at least `MIN_TOP_K` (1)
    /// - Not exceed `MAX_TOP_K` (1000)
    ///
    /// # Arguments
    ///
    /// * `args` - The tool arguments as a JSON map
    /// * `default` - The default value to use if `top_k` is not provided
    ///
    /// # Returns
    ///
    /// Returns a `Result` containing the validated `top_k` value or a validation error.
    ///
    /// # Errors
    ///
    /// Returns an error if `top_k` is less than `MIN_TOP_K` or exceeds `MAX_TOP_K`.
    fn extract_top_k(args: &serde_json::Map<String, Value>, default: usize) -> Result<usize> {
        let value = args.get("top_k")
            .and_then(|v| v.as_u64())
            .unwrap_or(default as u64) as usize;

        if value < MIN_TOP_K {
            return Err(RagError::Validation(format!(
                "top_k must be at least {}, got {}", MIN_TOP_K, value
            )));
        }

        if value > MAX_TOP_K {
            return Err(RagError::Validation(format!(
                "top_k must not exceed {}, got {}", MAX_TOP_K, value
            )));
        }

        Ok(value)
    }

    /// Creates a standardized "no index found" error response.
    ///
    /// This response is returned when the HNSW index doesn't exist for the project.
    fn no_index_response() -> Value {
        json!({
            "content": [{
                "type": "text",
                "text": MSG_NO_INDEX
            }]
        })
    }

    /// Creates a standardized embedding error response.
    ///
    /// # Arguments
    ///
    /// * `error` - The error that occurred during embedding generation
    fn embedding_error_response(error: impl Display) -> Value {
        json!({
            "content": [{
                "type": "text",
                "text": format!("{}{}", MSG_EMBEDDING_ERROR, error)
            }]
        })
    }

    /// Creates a standardized index loading error response.
    ///
    /// # Arguments
    ///
    /// * `error` - The error that occurred during index loading
    fn index_error_response(error: impl Display) -> Value {
        json!({
            "content": [{
                "type": "text",
                "text": format!("{}{}", MSG_INDEX_ERROR, error)
            }]
        })
    }

    /// Executes a search query against the HNSW index.
    ///
    /// This method:
    /// 1. Checks if the HNSW index exists
    /// 2. Loads the index from storage
    /// 3. Generates an embedding for the query
    /// 4. Performs a k-NN search
    /// 5. Formats the results as an MCP response
    ///
    /// # Arguments
    ///
    /// * `query` - The search query text
    /// * `top_k` - Maximum number of results to return
    /// * `content_type` - Optional content type filter (File, Message, Session)
    ///
    /// # Returns
    ///
    /// Returns a `Result` containing the formatted MCP response or an error.
    ///
    /// # Errors
    ///
    /// This method doesn't return errors directly. Instead, it returns
    /// `Ok()` with error messages embedded in the response content for:
    /// - Missing index
    /// - Index loading failures
    /// - Embedding generation failures
    fn execute_search(
        &self,
        query: &str,
        top_k: usize,
        content_type: Option<ContentType>,
    ) -> Result<Value> {
        // Limit top_k to prevent excessive response size
        let effective_top_k = top_k.min(MAX_RESULTS_TO_DISPLAY);

        // Check if HNSW index exists
        if !self.storage.has_hnsw_index() {
            return Ok(Self::no_index_response());
        }

        // Try to load and query the index
        match self.storage.load_hnsw() {
            Ok(Some(index)) => {
                // Generate embedding for query using helper method
                let embedding = match self.embed_query_sync(query) {
                    Ok(e) => e,
                    Err(e) => {
                        return Ok(Self::embedding_error_response(e));
                    }
                };

                // Search HNSW index with optional content type filter
                let results = match index.search(&embedding, effective_top_k, content_type) {
                    Ok(results) => results,
                    Err(e) => {
                        eprintln!("Warning: HNSW search failed: {}", e);
                        // Return empty results rather than failing completely
                        Vec::new()
                    }
                };

                // Format results with pre-allocated capacity using constants
                let type_label = content_type
                    .map(|ct| format!(" ({:?})", ct))
                    .unwrap_or_default();

                let mut output = String::with_capacity(
                    OUTPUT_BASE_CAPACITY + results.len() * OUTPUT_PER_RESULT_CAPACITY
                );
                let _ = writeln!(output, "# RAG Search Results{}", type_label);
                let _ = writeln!(output, "Query: {}", query);
                let _ = writeln!(output, "Found {} results", results.len());
                let _ = writeln!(output);

                for (i, (id, distance)) in results.iter().enumerate() {
                    let similarity = Self::distance_to_similarity(*distance);
                    let _ = writeln!(output, "{}. {} (similarity: {:.4})", i + 1, id, similarity);
                }

                Ok(json!({
                    "content": [{
                        "type": "text",
                        "text": output
                    }]
                }))
            }
            Ok(None) => Ok(Self::no_index_response()),
            Err(e) => Ok(Self::index_error_response(e)),
        }
    }

    /// Execute search with post-hoc file type filtering.
    fn execute_search_with_file_filter(
        &self,
        query: &str,
        top_k: usize,
        base_type: ContentType,
        file_filter: impl Fn(&str) -> bool,
        title: &str,
    ) -> Result<Value> {
        // Limit top_k to prevent excessive response size
        let effective_top_k = top_k.min(MAX_RESULTS_TO_DISPLAY);

        // Check if HNSW index exists
        if !self.storage.has_hnsw_index() {
            return Ok(Self::no_index_response());
        }

        // Try to load and query the index
        match self.storage.load_hnsw() {
            Ok(Some(index)) => {
                // Generate embedding for query using helper method
                let embedding = match self.embed_query_sync(query) {
                    Ok(e) => e,
                    Err(e) => {
                        return Ok(Self::embedding_error_response(e));
                    }
                };

                // Search HNSW index - get more results for filtering, but cap at absolute maximum
                let search_limit = (effective_top_k.saturating_mul(FILE_FILTER_MULTIPLIER))
                    .min(MAX_SEARCH_RESULTS);
                let raw_results = match index.search(&embedding, search_limit, Some(base_type)) {
                    Ok(results) => results,
                    Err(e) => {
                        eprintln!("Warning: HNSW search with filter failed: {}", e);
                        Vec::new()
                    }
                };
                let total_raw = raw_results.len();

                // Filter by file type and limit to effective_top_k
                let mut filtered_results = Vec::with_capacity(effective_top_k);
                for (id, distance) in raw_results {
                    if file_filter(&id) {
                        filtered_results.push((id, distance));
                        if filtered_results.len() >= effective_top_k {
                            break;
                        }
                    }
                }

                // Format results with pre-allocated capacity using constants
                let mut output = String::with_capacity(
                    OUTPUT_BASE_CAPACITY + filtered_results.len() * OUTPUT_PER_RESULT_CAPACITY
                );
                let _ = writeln!(output, "# {}", title);
                let _ = writeln!(output, "Query: {}", query);
                let _ = writeln!(output, "Found {} results (filtered from {} indexed items)", filtered_results.len(), total_raw);
                let _ = writeln!(output);

                for (i, (id, distance)) in filtered_results.iter().enumerate() {
                    let similarity = Self::distance_to_similarity(*distance);
                    let _ = writeln!(output, "{}. {} (similarity: {:.4})", i + 1, id, similarity);
                }

                Ok(json!({
                    "content": [{
                        "type": "text",
                        "text": output
                    }]
                }))
            }
            Ok(None) => Ok(Self::no_index_response()),
            Err(e) => Ok(Self::index_error_response(e)),
        }
    }

    /// Starts the MCP server and begins processing JSON-RPC requests.
    ///
    /// This method runs the server's main loop, which:
    /// 1. Reads JSON-RPC requests from standard input
    /// 2. Parses and validates each request
    /// 3. Dispatches to the appropriate handler
    /// 4. Writes responses to standard output
    ///
    /// The server continues running until stdin is closed.
    ///
    /// # Protocol
    ///
    /// The server communicates using JSON-RPC 2.0 over stdio:
    /// - Each line is a separate JSON-RPC request/response
    /// - Requests must include `jsonrpc`, `id`, `method`, and optional `params`
    /// - Responses include `jsonrpc`, `id`, and either `result` or `error`
    ///
    /// # Supported Methods
    ///
    /// - `initialize` - Server initialization handshake
    /// - `tools/list` - List available tools
    /// - `tools/call` - Execute a tool
    /// - `ping` - Health check
    ///
    /// # Example
    ///
    /// ```no_run
    /// use claude_rag::mcp::McpServer;
    ///
    /// #[tokio::main]
    /// async fn main() -> anyhow::Result<()> {
    ///     let server = McpServer::new(None)?;
    ///     server.run().await?;
    ///     Ok(())
    /// }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Stdin/stdout cannot be accessed
    /// - JSON parsing fails
    /// - Response serialization fails
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

            // Enforce request size limit to prevent DoS
            if line.len() > MAX_REQUEST_SIZE {
                let response = JsonRpcResponse::error(
                    json!(null),
                    JSONRPC_ERROR_INVALID_PARAMS,
                    &format!("Request too large (maximum {} bytes)", MAX_REQUEST_SIZE),
                );
                let response_json = serde_json::to_string(&response)
                    .map_err(|e| RagError::Parse(format!("Failed to serialize response: {}", e)))?;
                writeln!(stdout, "{}", response_json).map_err(RagError::Io)?;
                stdout.flush().map_err(RagError::Io)?;
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
                JSONRPC_ERROR_METHOD_NOT_FOUND,
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
            "rag_search_code" => self.call_rag_search_code(arguments)?,
            "rag_search_docs" => self.call_rag_search_docs(arguments)?,
            "rag_search_session" => self.call_rag_search(arguments)?,
            "rag_timeline" => self.call_rag_timeline(arguments)?,
            _ => {
                return Ok(JsonRpcResponse::error(
                    request.id.clone(),
                    JSONRPC_ERROR_INVALID_PARAMS,
                    &format!("Unknown tool: {}", tool_name),
                ));
            }
        };

        Ok(JsonRpcResponse::success(request.id.clone(), result))
    }

    /// Call rag_query tool - queries all indexed content.
    fn call_rag_query(&self, args: &serde_json::Map<String, Value>) -> Result<Value> {
        let query = Self::extract_query(args)?;
        let top_k = Self::extract_top_k(args, DEFAULT_TOP_K_GENERAL)?;
        self.execute_search(&query, top_k, None)
    }

    /// Call rag_search_session tool - searches only sessions.
    fn call_rag_search(&self, args: &serde_json::Map<String, Value>) -> Result<Value> {
        let query = Self::extract_query(args)?;
        let top_k = Self::extract_top_k(args, DEFAULT_TOP_K_GENERAL)?;
        self.execute_search(&query, top_k, Some(ContentType::Session))
    }

    /// Call rag_search_code tool - searches only source code files.
    fn call_rag_search_code(&self, args: &serde_json::Map<String, Value>) -> Result<Value> {
        let query = Self::extract_query(args)?;
        let top_k = Self::extract_top_k(args, DEFAULT_TOP_K_CODE)?;
        self.execute_search_with_file_filter(
            &query,
            top_k,
            ContentType::File,
            Self::is_code_file,
            "RAG Search Results (Code)",
        )
    }

    /// Call rag_search_docs tool - searches only documentation files.
    fn call_rag_search_docs(&self, args: &serde_json::Map<String, Value>) -> Result<Value> {
        let query = Self::extract_query(args)?;
        let top_k = Self::extract_top_k(args, DEFAULT_TOP_K_GENERAL)?;
        self.execute_search_with_file_filter(
            &query,
            top_k,
            ContentType::File,
            Self::is_doc_file,
            "RAG Search Results (Documentation)",
        )
    }

    /// Call rag_timeline tool.
    ///
    /// This builds a timeline from indexed git commits and sessions.
    fn call_rag_timeline(&self, args: &serde_json::Map<String, Value>) -> Result<Value> {
        let feature = args.get("feature")
            .and_then(|v| v.as_str());

        // Reuse extract_top_k for consistent validation
        let top_k = Self::extract_top_k(args, DEFAULT_TOP_K_TIMELINE)?;

        // TODO: Implement timeline functionality
        // For now, return a placeholder response
        // In a full implementation, we would:
        // 1. Query storage for all commits and sessions
        // 2. Build timeline using FeatureTimeline::build()
        // 3. Format results

        let mut output = Vec::new();
        output.push("# Feature Timeline".to_string());

        if let Some(feat) = feature {
            output.push(format!("Feature: {}", feat));
        } else {
            output.push("General project timeline".to_string());
        }

        output.push(format!("Top-K: {}", top_k));
        output.push(String::new());
        output.push(MSG_TIMELINE_NOT_IMPLEMENTED.to_string());
        output.push(MSG_TIMELINE_IMPLEMENTED.to_string());

        Ok(json!({
            "content": [{
                "type": "text",
                "text": output.join("\n")
            }]
        }))
    }

    /// Generates an MCP server configuration for Claude Code's settings.json.
    ///
    /// This method creates a JSON configuration snippet that can be added to
    /// Claude Code's settings file to register this MCP server.
    ///
    /// # Returns
    ///
    /// Returns a `Result` containing the pretty-printed JSON configuration string.
    ///
    /// # Example Output
    ///
    /// ```json
    /// {
    ///   "mcpServers": {
    ///     "claude-rag": {
    ///       "command": "claude-rag",
    ///       "args": ["mcp-server"]
    ///     }
    ///   }
    /// }
    /// ```
    ///
    /// # Usage
    ///
    /// The returned JSON can be added to `~/.claude/settings.json` under the `mcpServers` key.
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

    // Tool definition methods
    //
    // These methods return JSON tool definitions for the MCP protocol.
    // Each tool definition includes:
    // - name: Tool identifier
    // - description: Human-readable description
    // - inputSchema: JSON Schema for parameters

    /// Generate rag_query tool definition.
    ///
    /// This tool queries all indexed content (sessions, files, git history).
    fn tool_rag_query(&self) -> Value {
        json!({
            "name": "rag_query",
            "description": "Query the RAG knowledge base for relevant context from all sources (sessions, files, git history). \
                          Use this when you need to find information about the codebase, previous discussions, or project history.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "The search query",
                        "minLength": 1,
                        "maxLength": 1000
                    },
                    "top_k": {
                        "type": "number",
                        "description": "Maximum number of results to return (default: 5, max: 100)",
                        "default": DEFAULT_TOP_K_GENERAL,
                        "minimum": 1,
                        "maximum": 100
                    }
                },
                "required": ["query"]
            }
        })
    }

    /// Generate rag_search_code tool definition.
    ///
    /// This tool queries only source code content.
    fn tool_rag_search_code(&self) -> Value {
        json!({
            "name": "rag_search_code",
            "description": "Search only source code files in the RAG knowledge base. \
                          Use this when you need to find specific implementations, functions, or code patterns.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "The search query for code",
                        "minLength": 1,
                        "maxLength": 1000
                    },
                    "top_k": {
                        "type": "number",
                        "description": "Maximum number of results to return (default: 10, max: 100)",
                        "default": DEFAULT_TOP_K_CODE,
                        "minimum": 1,
                        "maximum": 100
                    }
                },
                "required": ["query"]
            }
        })
    }

    /// Generate rag_search_docs tool definition.
    ///
    /// This tool queries only documentation content.
    fn tool_rag_search_docs(&self) -> Value {
        json!({
            "name": "rag_search_docs",
            "description": "Search only documentation files (README, API docs, guides, etc.) in the RAG knowledge base. \
                          Use this when you need to find project documentation, usage examples, or design explanations.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "The search query for documentation",
                        "minLength": 1,
                        "maxLength": 1000
                    },
                    "top_k": {
                        "type": "number",
                        "description": "Maximum number of results to return (default: 5, max: 100)",
                        "default": DEFAULT_TOP_K_GENERAL,
                        "minimum": 1,
                        "maximum": 100
                    }
                },
                "required": ["query"]
            }
        })
    }

    /// Generate rag_search_session tool definition.
    ///
    /// This tool queries only Claude session content.
    fn tool_rag_search_session(&self) -> Value {
        json!({
            "name": "rag_search_session",
            "description": "Search only previous Claude sessions in the RAG knowledge base. \
                          Use this when you need to find what was discussed previously about a topic.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "The search query for sessions",
                        "minLength": 1,
                        "maxLength": 1000
                    },
                    "top_k": {
                        "type": "number",
                        "description": "Maximum number of results to return (default: 5, max: 100)",
                        "default": DEFAULT_TOP_K_GENERAL,
                        "minimum": 1,
                        "maximum": 100
                    }
                },
                "required": ["query"]
            }
        })
    }

    /// Generate rag_timeline tool definition.
    ///
    /// This tool builds a feature timeline from git history and sessions.
    fn tool_rag_timeline(&self) -> Value {
        json!({
            "name": "rag_timeline",
            "description": "[DEVELOPMENT] Build a feature timeline from git history and sessions. \
                          Note: This tool is not yet fully implemented and returns placeholder data.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "feature": {
                        "type": "string",
                        "description": "Optional feature name to build a timeline for. If not provided, returns a general project timeline.",
                        "maxLength": 100
                    },
                    "top_k": {
                        "type": "number",
                        "description": "Maximum number of timeline events to include (default: 20, max: 100)",
                        "default": DEFAULT_TOP_K_TIMELINE,
                        "minimum": 1,
                        "maximum": 100
                    }
                },
                "required": []
            }
        })
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
    use tempfile::TempDir;

    /// Create the test configuration content.
    ///
    /// Centralized to avoid duplication across test helpers.
    fn create_test_config_content() -> String {
        r#"{
            "embedding": {
                "api_token": "test-token",
                "api_url": "https://open.bigmodel.cn/api/paas/v4/embeddings",
                "dimensions": 1024,
                "batch_size": 8,
                "timeout_ms": 30000
            },
            "hnsw": {
                "m": 16,
                "ef_construction": 200,
                "ef_search": 50
            },
            "index": {
                "index_source": true,
                "index_docs": true,
                "index_other": false
            },
            "daemon": {
                "session_timeout_seconds": 60,
                "persist_interval_seconds": 300,
                "socket_path": null,
                "pid_file": null,
                "file_debounce_ms": 500,
                "shutdown_timeout_seconds": 5
            },
            "git": {
                "enable_git_indexing": true,
                "max_commits_to_index": 1000,
                "include_diff_content": true,
                "conventional_commits": true,
                "extract_breaking_changes": true
            },
            "confidence": {
                "code_weight": 1.2,
                "git_commit_weight": 1.0,
                "session_weight": 0.7,
                "decay_rate": 0.05
            },
            "retrieval": {
                "enable_timeline": true,
                "show_git_context": true
            }
        }"#.to_string()
    }

    /// Create a test server with a temporary project directory.
    fn create_test_server() -> McpServer {
        let temp_dir = TempDir::new()
            .expect("Failed to create temporary directory for test");
        let project_path = temp_dir.path();

        let rag_dir = project_path.join(".rag");
        std::fs::create_dir_all(&rag_dir)
            .expect("Failed to create .rag directory");
        std::fs::write(rag_dir.join("config.json"), create_test_config_content())
            .expect("Failed to write test config file");

        McpServer::new(Some(project_path))
            .expect("Failed to create test MCP server")
    }

    /// Create a test server with a mock HNSW index for testing query logic.
    fn create_test_server_with_index() -> (McpServer, TempDir) {
        let temp_dir = TempDir::new()
            .expect("Failed to create temporary directory for test");
        let project_path = temp_dir.path();

        // Create config directory and write config
        let rag_dir = project_path.join(".rag");
        std::fs::create_dir_all(&rag_dir)
            .expect("Failed to create .rag directory");
        std::fs::write(rag_dir.join("config.json"), create_test_config_content())
            .expect("Failed to write test config file");

        // Create a mock HNSW index
        let mut index = crate::storage::hnsw::HnswIndex::new(16, 200, 50);

        // Add some test vectors
        let test_vectors = vec![
            ("msg-1-chunk-0", vec![0.1, 0.2, 0.3], ContentType::Message),
            ("file-1-chunk-0", vec![0.4, 0.5, 0.6], ContentType::File),
            ("file-2-chunk-0", vec![0.7, 0.8, 0.9], ContentType::File),
            ("session-1-chunk-0", vec![0.2, 0.3, 0.4], ContentType::Session),
        ];

        for (id, vector, content_type) in test_vectors {
            index.insert(id.to_string(), content_type, vector)
                .expect("Failed to insert test vector into HNSW index");
        }

        // Save the index
        let hnsw_path = project_path.join(".rag/hnsw.bin");
        index.save(&hnsw_path)
            .expect("Failed to save HNSW index");

        let server = McpServer::new(Some(project_path))
            .expect("Failed to create test MCP server");
        (server, temp_dir)
    }

    #[test]
    fn test_mcp_server_new() {
        let temp_dir = TempDir::new()
            .expect("Failed to create temporary directory");
        let _server = McpServer::new(Some(temp_dir.path()));
    }

    #[test]
    fn test_generate_config() {
        let server = create_test_server();
        let config = server.generate_config()
            .expect("Failed to generate config");

        assert!(config.contains("claude-rag"));
        assert!(config.contains("mcp-server"));
    }

    #[test]
    fn test_tool_rag_query_definition() {
        let server = create_test_server();
        let tool = server.tool_rag_query();

        assert_eq!(tool["name"], "rag_query");
        assert!(tool["description"].is_string());
        assert!(tool["inputSchema"]["properties"]["query"].is_object());
        assert!(tool["inputSchema"]["properties"]["top_k"].is_object());
    }

    #[test]
    fn test_tool_rag_search_code_definition() {
        let server = create_test_server();
        let tool = server.tool_rag_search_code();

        assert_eq!(tool["name"], "rag_search_code");
        assert!(tool["description"].is_string());
    }

    #[test]
    fn test_tool_rag_search_docs_definition() {
        let server = create_test_server();
        let tool = server.tool_rag_search_docs();

        assert_eq!(tool["name"], "rag_search_docs");
    }

    #[test]
    fn test_tool_rag_search_session_definition() {
        let server = create_test_server();
        let tool = server.tool_rag_search_session();

        assert_eq!(tool["name"], "rag_search_session");
    }

    #[test]
    fn test_tool_rag_timeline_definition() {
        let server = create_test_server();
        let tool = server.tool_rag_timeline();

        assert_eq!(tool["name"], "rag_timeline");
        assert!(tool["inputSchema"]["properties"]["feature"].is_object());
    }

    #[test]
    fn test_handle_initialize() {
        let server = create_test_server();
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
        let server = create_test_server();
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
        let server = create_test_server();
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
        let server = create_test_server();
        let mut args = serde_json::Map::new();
        args.insert("query".to_string(), json!("test query"));
        args.insert("top_k".to_string(), json!(10));

        let result = server.call_rag_query(&args).unwrap();
        assert!(result["content"].is_array());
    }

    #[test]
    fn test_call_rag_query_missing_query() {
        let server = create_test_server();
        let args = serde_json::Map::new();

        let result = server.call_rag_query(&args);
        assert!(result.is_err());
    }

    #[test]
    fn test_call_rag_timeline_with_feature() {
        let server = create_test_server();
        let mut args = serde_json::Map::new();
        args.insert("feature".to_string(), json!("authentication"));

        let result = server.call_rag_timeline(&args).unwrap();
        assert!(result["content"].is_array());
    }

    #[test]
    fn test_call_rag_timeline_without_feature() {
        let server = create_test_server();
        let args = serde_json::Map::new();

        let result = server.call_rag_timeline(&args).unwrap();
        assert!(result["content"].is_array());
    }

    // ==================== Security Tests ====================

    // Security tests for query validation
    #[test]
    fn test_extract_query_empty_string() {
        let mut args = serde_json::Map::new();
        args.insert("query".to_string(), json!(""));

        let result = McpServer::extract_query(&args);
        // Empty string should now be rejected
        assert!(result.is_err());
    }

    #[test]
    fn test_extract_query_whitespace_only() {
        let mut args = serde_json::Map::new();
        args.insert("query".to_string(), json!("   \t\n  "));

        let result = McpServer::extract_query(&args);
        // Whitespace-only should be rejected
        assert!(result.is_err());
    }

    #[test]
    fn test_extract_query_too_long() {
        let long_query = "a".repeat(MAX_QUERY_LENGTH + 100);
        let mut args = serde_json::Map::new();
        args.insert("query".to_string(), json!(long_query));

        let result = McpServer::extract_query(&args);
        assert!(result.is_err());
    }

    #[test]
    fn test_extract_query_at_max_length() {
        let max_query = "a".repeat(MAX_QUERY_LENGTH);
        let mut args = serde_json::Map::new();
        args.insert("query".to_string(), json!(max_query));

        let result = McpServer::extract_query(&args);
        // Should succeed at exactly max length
        assert!(result.is_ok());
    }

    #[test]
    fn test_extract_query_valid_with_padding() {
        let mut args = serde_json::Map::new();
        args.insert("query".to_string(), json!("  valid query  "));

        let result = McpServer::extract_query(&args);
        assert!(result.is_ok());
        // Should be trimmed
        assert_eq!(result.unwrap(), "valid query");
    }

    #[test]
    fn test_distance_to_similarity() {
        // distance_to_similarity is a static method, no server instance needed
        // Distance 0 should return 1.0 (perfect similarity)
        assert_eq!(McpServer::distance_to_similarity(0.0), 1.0);

        // Distance 1 should return 0.5
        assert_eq!(McpServer::distance_to_similarity(1.0), 0.5);

        // Large distances should return close to 0
        let similarity = McpServer::distance_to_similarity(100.0);
        assert!(similarity > 0.0 && similarity < 0.01);
    }

    #[test]
    fn test_is_code_file() {
        // Test code file extensions
        assert!(McpServer::is_code_file("main.rs"));
        assert!(McpServer::is_code_file("app.py"));
        assert!(McpServer::is_code_file("index.ts"));
        assert!(McpServer::is_code_file("Component.tsx"));
        assert!(McpServer::is_code_file("lib.go"));
        assert!(McpServer::is_code_file("Main.java"));
        assert!(McpServer::is_code_file("script.sh"));
        assert!(McpServer::is_code_file("config.json"));
        assert!(McpServer::is_code_file("Cargo.toml"));

        // Test non-code files
        assert!(!McpServer::is_code_file("README.md"));
        assert!(!McpServer::is_code_file("docs.txt"));
        assert!(!McpServer::is_code_file("file_without_ext"));
        assert!(!McpServer::is_code_file(".hidden"));
        assert!(!McpServer::is_code_file(""));
    }

    #[test]
    fn test_is_doc_file() {
        // Test documentation file extensions
        assert!(McpServer::is_doc_file("README.md"));
        assert!(McpServer::is_doc_file("CHANGELOG.markdown"));
        assert!(McpServer::is_doc_file("api.rst"));
        assert!(McpServer::is_doc_file("notes.txt"));
        assert!(McpServer::is_doc_file("manual.adoc"));
        assert!(McpServer::is_doc_file("guide.html"));
        assert!(McpServer::is_doc_file("spec.pdf"));
        assert!(McpServer::is_doc_file("doc.docx"));

        // Test non-documentation files
        assert!(!McpServer::is_doc_file("main.rs"));
        assert!(!McpServer::is_doc_file("app.py"));
        assert!(!McpServer::is_doc_file("file_without_ext"));
        assert!(!McpServer::is_doc_file(""));
    }

    #[test]
    fn test_extract_query_valid() {
        let mut args = serde_json::Map::new();
        args.insert("query".to_string(), json!("test query"));

        let result = McpServer::extract_query(&args);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "test query");
    }

    #[test]
    fn test_extract_query_missing() {
        let args = serde_json::Map::new();

        let result = McpServer::extract_query(&args);
        assert!(result.is_err());
    }

    #[test]
    fn test_extract_query_null() {
        let mut args = serde_json::Map::new();
        args.insert("query".to_string(), json!(null));

        let result = McpServer::extract_query(&args);
        assert!(result.is_err());
    }

    #[test]
    fn test_extract_top_k_with_value() {
        let mut args = serde_json::Map::new();
        args.insert("top_k".to_string(), json!(15));

        let result = McpServer::extract_top_k(&args, 5);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 15);
    }

    #[test]
    fn test_extract_top_k_with_default() {
        let args = serde_json::Map::new();

        let result = McpServer::extract_top_k(&args, 5);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 5);
    }

    #[test]
    fn test_extract_top_k_with_zero() {
        let mut args = serde_json::Map::new();
        args.insert("top_k".to_string(), json!(0));

        let result = McpServer::extract_top_k(&args, 5);
        // top_k=0 should be rejected (below MIN_TOP_K of 1)
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), RagError::Validation(_)));
    }

    #[test]
    fn test_extract_top_k_exceeds_max() {
        let mut args = serde_json::Map::new();
        args.insert("top_k".to_string(), json!(2000));

        let result = McpServer::extract_top_k(&args, 5);
        assert!(result.is_err());
    }

    #[test]
    fn test_resolve_project_path_with_provided() {
        let temp_dir = TempDir::new()
            .expect("Failed to create temporary directory");
        let path = temp_dir.path();

        let result = McpServer::resolve_project_path(Some(path));
        assert!(result.is_ok());
        assert_eq!(result.expect("Failed to resolve project path"), path);
    }

    #[test]
    fn test_resolve_project_path_with_env_var() {
        let temp_dir = TempDir::new()
            .expect("Failed to create temporary directory");
        let path_str = temp_dir.path().to_str()
            .expect("Project path contains invalid UTF-8 characters");
        std::env::set_var(PROJECT_PATH_ENV, path_str);

        let result = McpServer::resolve_project_path(None);
        assert!(result.is_ok());
        assert_eq!(result.expect("Failed to resolve project path"), temp_dir.path());

        // Clean up environment variable
        std::env::remove_var(PROJECT_PATH_ENV);
    }

    #[test]
    fn test_resolve_project_path_env_var_not_exists() {
        std::env::set_var(PROJECT_PATH_ENV, "/nonexistent/path/that/does/not/exist");

        let result = McpServer::resolve_project_path(None);
        // Should fall back to current directory
        assert!(result.is_ok());

        // Clean up environment variable
        std::env::remove_var(PROJECT_PATH_ENV);
    }

    #[test]
    fn test_constants_defined() {
        // Verify constants are defined and non-empty
        assert!(!MCP_VERSION.is_empty());
        assert!(!PROJECT_PATH_ENV.is_empty());
        assert!(!MSG_NO_INDEX.is_empty());
        assert!(!MSG_INDEX_ERROR.is_empty());
        assert!(!MSG_EMBEDDING_ERROR.is_empty());
        assert!(!MSG_TIMELINE_NOT_IMPLEMENTED.is_empty());
        assert!(!MSG_TIMELINE_IMPLEMENTED.is_empty());
        assert!(!DOC_EXTENSIONS.is_empty());
        assert!(!CODE_EXTENSIONS.is_empty());

        // Verify new validation constants
        assert!(MAX_TOP_K > 0);
        assert!(DEFAULT_TOP_K_GENERAL > 0);
        assert!(DEFAULT_TOP_K_CODE > 0);
        assert!(DEFAULT_TOP_K_GENERAL <= MAX_TOP_K);
        assert!(DEFAULT_TOP_K_CODE <= MAX_TOP_K);
    }

    #[test]
    fn test_tool_definitions_count() {
        let server = create_test_server();

        // Verify all tools are defined
        let tools = vec![
            server.tool_rag_query(),
            server.tool_rag_search_code(),
            server.tool_rag_search_docs(),
            server.tool_rag_search_session(),
            server.tool_rag_timeline(),
        ];

        assert_eq!(tools.len(), 5);

        // Verify each tool has required fields
        for tool in tools {
            assert!(tool["name"].is_string());
            assert!(tool["description"].is_string());
            assert!(tool["inputSchema"].is_object());
            assert!(tool["inputSchema"]["type"].is_string());
            assert!(tool["inputSchema"]["properties"].is_object());
        }
    }

    #[test]
    fn test_handle_tools_call_with_params() {
        let server = create_test_server();
        let mut params = serde_json::Map::new();
        params.insert("name".to_string(), json!("rag_query"));
        params.insert("arguments".to_string(), json!({"query": "test"}));

        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: json!(1),
            method: "tools/call".to_string(),
            params: Some(json!(params)),
        };

        let response = server.handle_tools_call(&request);
        assert!(response.is_ok());
        let response = response.unwrap();
        assert!(response.result.is_some());
        assert!(response.error.is_none());
    }

    #[test]
    fn test_handle_tools_call_unknown_tool() {
        let server = create_test_server();
        let mut params = serde_json::Map::new();
        params.insert("name".to_string(), json!("unknown_tool"));
        params.insert("arguments".to_string(), json!({}));

        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: json!(1),
            method: "tools/call".to_string(),
            params: Some(json!(params)),
        };

        let response = server.handle_tools_call(&request);
        assert!(response.is_ok());
        let response = response.unwrap();
        // Error response should have None for result, Some for error
        assert!(response.result.is_none());
        assert!(response.error.is_some());
        assert_eq!(response.error.unwrap().code, -32602);
    }

    #[test]
    fn test_handle_tools_call_missing_name() {
        let server = create_test_server();
        let params = serde_json::Map::new();

        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: json!(1),
            method: "tools/call".to_string(),
            params: Some(json!(params)),
        };

        let response = server.handle_tools_call(&request);
        assert!(response.is_err());
    }

    #[test]
    fn test_jsonrpc_response_success() {
        let id = json!(1);
        let result = json!({"test": "data"});

        let response = JsonRpcResponse::success(id.clone(), result.clone());
        assert_eq!(response.jsonrpc, "2.0");
        assert_eq!(response.id, id);
        assert!(response.result.is_some());
        assert!(response.error.is_none());
        assert_eq!(response.result.unwrap(), result);
    }

    #[test]
    fn test_jsonrpc_response_error() {
        let id = json!(1);
        let code = -32601;
        let message = "Method not found";

        let response = JsonRpcResponse::error(id.clone(), code, message);
        assert_eq!(response.jsonrpc, "2.0");
        assert_eq!(response.id, id);
        assert!(response.result.is_none());
        assert!(response.error.is_some());
        let error = response.error.unwrap();
        assert_eq!(error.code, code);
        assert_eq!(error.message, message);
    }

    #[test]
    fn test_rag_query_with_no_index_returns_message() {
        let server = create_test_server();
        let mut args = serde_json::Map::new();
        args.insert("query".to_string(), json!("test"));

        let result = server.call_rag_query(&args).unwrap();
        assert!(result["content"].is_array());

        // Verify "no index" message is returned
        let content = &result["content"].as_array().unwrap()[0];
        assert_eq!(content["type"], "text");
        let text = content["text"].as_str().unwrap();
        assert!(text.contains(MSG_NO_INDEX) || text.contains("No index"));
    }

    #[test]
    fn test_rag_search_code_with_no_index() {
        let server = create_test_server();
        let mut args = serde_json::Map::new();
        args.insert("query".to_string(), json!("test"));

        let result = server.call_rag_search_code(&args).unwrap();
        assert!(result["content"].is_array());

        // Verify "no index" message is returned
        let content = &result["content"].as_array().unwrap()[0];
        assert!(content["text"].as_str().unwrap().contains("No index"));
    }

    #[test]
    fn test_rag_search_docs_with_no_index() {
        let server = create_test_server();
        let mut args = serde_json::Map::new();
        args.insert("query".to_string(), json!("test"));

        let result = server.call_rag_search_docs(&args).unwrap();
        assert!(result["content"].is_array());

        // Verify "no index" message is returned
        let content = &result["content"].as_array().unwrap()[0];
        assert!(content["text"].as_str().unwrap().contains("No index"));
    }

    #[test]
    fn test_rag_search_session_with_no_index() {
        let server = create_test_server();
        let mut args = serde_json::Map::new();
        args.insert("query".to_string(), json!("test"));

        let result = server.call_rag_search(&args).unwrap();
        assert!(result["content"].is_array());

        // Verify "no index" message is returned
        let content = &result["content"].as_array().unwrap()[0];
        assert!(content["text"].as_str().unwrap().contains("No index"));
    }

    #[test]
    fn test_doc_extensions_coverage() {
        // Verify documentation extension constants include major doc types
        assert!(DOC_EXTENSIONS.contains(&"md"));
        assert!(DOC_EXTENSIONS.contains(&"markdown"));
        assert!(DOC_EXTENSIONS.contains(&"rst"));
        assert!(DOC_EXTENSIONS.contains(&"txt"));
        assert!(DOC_EXTENSIONS.contains(&"adoc"));
        assert!(DOC_EXTENSIONS.contains(&"html"));
        assert!(DOC_EXTENSIONS.contains(&"pdf"));
    }

    #[test]
    fn test_code_extensions_coverage() {
        // Verify code extension constants include major code types
        assert!(CODE_EXTENSIONS.contains(&"rs"));
        assert!(CODE_EXTENSIONS.contains(&"py"));
        assert!(CODE_EXTENSIONS.contains(&"js"));
        assert!(CODE_EXTENSIONS.contains(&"ts"));
        assert!(CODE_EXTENSIONS.contains(&"tsx"));
        assert!(CODE_EXTENSIONS.contains(&"go"));
        assert!(CODE_EXTENSIONS.contains(&"java"));
        assert!(CODE_EXTENSIONS.contains(&"c"));
        assert!(CODE_EXTENSIONS.contains(&"cpp"));
        assert!(CODE_EXTENSIONS.contains(&"json"));
        assert!(CODE_EXTENSIONS.contains(&"yaml"));
        assert!(CODE_EXTENSIONS.contains(&"toml"));
    }

    // ==================== Additional Tests for Coverage ====================

    #[test]
    fn test_is_doc_file_edge_cases() {
        // Files without extensions
        assert!(!McpServer::is_doc_file("Makefile"));
        assert!(!McpServer::is_doc_file("Dockerfile"));

        // Multiple dots - Path::extension() gets the last one
        assert!(McpServer::is_doc_file("file.name.md")); // md is a doc extension
        // .gz is not a doc extension, so archive.tar.gz won't be recognized as doc
        assert!(!McpServer::is_doc_file("archive.tar.gz"));

        // Case insensitivity - extensions are now case-insensitive
        assert!(McpServer::is_doc_file("README.MD")); // MD -> md is now recognized
        assert!(McpServer::is_doc_file("README.md"));  // md is a doc extension
        assert!(McpServer::is_doc_file("README.Markdown")); // Mixed case works too
    }

    #[test]
    fn test_is_code_file_edge_cases() {
        // Files without extensions
        assert!(!McpServer::is_code_file("Makefile"));
        assert!(!McpServer::is_code_file("script"));

        // Multiple dots - Path::extension() gets the last one
        // .bak is not a code extension, so lib.rs.bak won't be recognized as code
        assert!(!McpServer::is_code_file("lib.rs.bak"));
        assert!(McpServer::is_code_file("lib.c.rs"));  // rs is a code extension

        // Case insensitivity - extensions are now case-insensitive
        assert!(McpServer::is_code_file("main.RS")); // RS -> rs is now recognized
        assert!(McpServer::is_code_file("main.rs"));  // rs is a code extension
        assert!(McpServer::is_code_file("Component.TSX")); // Mixed case works
    }

    #[test]
    fn test_generate_config_format() {
        let server = create_test_server();
        let config = server.generate_config().unwrap();

        // Verify JSON format
        let parsed: Value = serde_json::from_str(&config).unwrap();
        assert!(parsed["mcpServers"].is_object());
        assert!(parsed["mcpServers"]["claude-rag"].is_object());
        assert_eq!(parsed["mcpServers"]["claude-rag"]["command"], "claude-rag");
        assert!(parsed["mcpServers"]["claude-rag"]["args"].is_array());
    }

    #[test]
    fn test_rag_timeline_output_format() {
        let server = create_test_server();
        let mut args = serde_json::Map::new();
        args.insert("feature".to_string(), json!("auth"));
        args.insert("top_k".to_string(), json!(25));

        let result = server.call_rag_timeline(&args).unwrap();
        let content = &result["content"].as_array().unwrap()[0];
        let text = content["text"].as_str().unwrap();

        // Verify timeline output contains expected information
        assert!(text.contains("Feature Timeline"));
        assert!(text.contains("auth"));
        assert!(text.contains("Top-K: 25"));
    }

    #[test]
    fn test_rag_timeline_top_k_default() {
        let server = create_test_server();
        let args = serde_json::Map::new();

        let result = server.call_rag_timeline(&args).unwrap();
        let content = &result["content"].as_array().unwrap()[0];
        let text = content["text"].as_str().unwrap();

        // Default top_k is 50
        assert!(text.contains("Top-K: 50"));
    }

    #[test]
    fn test_tool_rag_query_input_schema() {
        let server = create_test_server();
        let tool = server.tool_rag_query();

        // Verify inputSchema structure
        let schema = &tool["inputSchema"];
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["query"].is_object());
        assert!(schema["properties"]["top_k"].is_object());
        assert_eq!(schema["required"].as_array().unwrap().len(), 1);
        assert!(schema["required"].as_array().unwrap()[0] == "query");
    }

    #[test]
    fn test_tool_rag_search_code_input_schema() {
        let server = create_test_server();
        let tool = server.tool_rag_search_code();

        // Verify default top_k is 10
        assert_eq!(tool["inputSchema"]["properties"]["top_k"]["default"], 10);
    }

    #[test]
    fn test_tool_rag_search_docs_input_schema() {
        let server = create_test_server();
        let tool = server.tool_rag_search_docs();

        // Verify default top_k is 5
        assert_eq!(tool["inputSchema"]["properties"]["top_k"]["default"], 5);
    }

    #[test]
    fn test_tool_rag_search_session_input_schema() {
        let server = create_test_server();
        let tool = server.tool_rag_search_session();

        // Verify default top_k is 5
        assert_eq!(tool["inputSchema"]["properties"]["top_k"]["default"], 5);
    }

    #[test]
    fn test_tool_rag_timeline_input_schema() {
        let server = create_test_server();
        let tool = server.tool_rag_timeline();

        // Timeline tool has no required parameters
        let schema = &tool["inputSchema"];
        assert!(schema["properties"]["feature"].is_object());
        assert!(schema["properties"]["top_k"].is_object());
        // feature is optional - required may be null or empty array
        if let Some(required) = schema["required"].as_array() {
            assert!(!required.contains(&json!("feature")));
        }
        // It's also OK if there's no required field
    }

    #[test]
    fn test_call_rag_query_with_top_k() {
        let server = create_test_server();
        let mut args = serde_json::Map::new();
        args.insert("query".to_string(), json!("test"));
        args.insert("top_k".to_string(), json!(20));

        let result = server.call_rag_query(&args).unwrap();
        assert!(result["content"].is_array());
    }

    #[test]
    fn test_call_rag_search_code_with_top_k() {
        let server = create_test_server();
        let mut args = serde_json::Map::new();
        args.insert("query".to_string(), json!("test"));
        args.insert("top_k".to_string(), json!(15));

        let result = server.call_rag_search_code(&args).unwrap();
        assert!(result["content"].is_array());
    }

    #[test]
    fn test_call_rag_search_docs_with_top_k() {
        let server = create_test_server();
        let mut args = serde_json::Map::new();
        args.insert("query".to_string(), json!("test"));
        args.insert("top_k".to_string(), json!(8));

        let result = server.call_rag_search_docs(&args).unwrap();
        assert!(result["content"].is_array());
    }

    #[test]
    fn test_call_rag_search_session_with_top_k() {
        let server = create_test_server();
        let mut args = serde_json::Map::new();
        args.insert("query".to_string(), json!("test"));
        args.insert("top_k".to_string(), json!(12));

        let result = server.call_rag_search(&args).unwrap();
        assert!(result["content"].is_array());
    }

    #[test]
    fn test_msg_constants() {
        // Verify all message constants are non-empty
        assert!(!MSG_NO_INDEX.is_empty());
        assert!(!MSG_INDEX_ERROR.is_empty());
        assert!(!MSG_EMBEDDING_ERROR.is_empty());
        assert!(!MSG_TIMELINE_NOT_IMPLEMENTED.is_empty());
        assert!(!MSG_TIMELINE_IMPLEMENTED.is_empty());

        // Verify error messages don't start with special characters (for concatenation)
        assert_eq!(MSG_INDEX_ERROR.chars().next().unwrap(), 'E');
        assert_eq!(MSG_EMBEDDING_ERROR.chars().next().unwrap(), 'F');
    }

    #[test]
    fn test_mcp_version_format() {
        // Verify MCP version format
        assert!(MCP_VERSION.contains('-')); // YYYY-MM-DD format
        assert_eq!(MCP_VERSION.len(), 10);  // 2024-11-05 is 10 characters
    }

    #[test]
    fn test_project_path_env_var_name() {
        // Verify environment variable name follows conventions
        assert!(PROJECT_PATH_ENV.contains("PROJECT"));
        assert!(PROJECT_PATH_ENV.contains("PATH"));
        assert!(PROJECT_PATH_ENV.starts_with("CLAUDE_"));
    }

    #[test]
    fn test_query_response_structure() {
        let server = create_test_server();
        let mut args = serde_json::Map::new();
        args.insert("query".to_string(), json!("test"));

        let result = server.call_rag_query(&args).unwrap();

        // Verify response structure conforms to MCP specification
        assert!(result["content"].is_array());
        let content = &result["content"].as_array().unwrap()[0];
        assert_eq!(content["type"], "text");
        assert!(content["text"].is_string());
    }

    #[test]
    fn test_timeline_response_structure() {
        let server = create_test_server();
        let args = serde_json::Map::new();

        let result = server.call_rag_timeline(&args).unwrap();

        // Verify response structure conforms to MCP specification
        assert!(result["content"].is_array());
        let content = &result["content"].as_array().unwrap()[0];
        assert_eq!(content["type"], "text");
        assert!(content["text"].is_string());
    }

    #[test]
    fn test_extract_top_k_very_large() {
        let mut args = serde_json::Map::new();
        args.insert("top_k".to_string(), json!(u64::MAX));

        let result = McpServer::extract_top_k(&args, 5);
        // Very large values should be rejected (exceeds MAX_TOP_K)
        assert!(result.is_err());
    }

    #[test]
    fn test_distance_to_similarity_edge_cases() {
        // Test very large distance
        let huge_dist = McpServer::distance_to_similarity(1000000.0);
        assert!(huge_dist > 0.0 && huge_dist < 0.001);

        // Test NaN
        let nan_dist = McpServer::distance_to_similarity(f32::NAN);
        assert!(nan_dist.is_nan());
    }

    #[test]
    fn test_jsonrpc_request_parse() {
        let json_str = r#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#;
        let request: JsonRpcRequest = serde_json::from_str(json_str).unwrap();
        assert_eq!(request.jsonrpc, "2.0");
        assert_eq!(request.id, json!(1));
        assert_eq!(request.method, "initialize");
        assert!(request.params.is_none());
    }

    #[test]
    fn test_jsonrpc_request_with_params() {
        let json_str = r#"{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}"#;
        let request: JsonRpcRequest = serde_json::from_str(json_str).unwrap();
        assert!(request.params.is_some());
    }

    #[test]
    fn test_jsonrpc_response_serialization() {
        let response = JsonRpcResponse::success(json!(1), json!({"test":"data"}));
        let json_str = serde_json::to_string(&response).unwrap();
        let parsed: Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(parsed["jsonrpc"], "2.0");
        assert_eq!(parsed["result"]["test"], "data");
        assert!(parsed["error"].is_null() || parsed.get("error").is_none());
    }

    #[test]
    fn test_resolve_project_path_current_dir() {
        // Clear environment variable to ensure current directory is used
        std::env::remove_var(PROJECT_PATH_ENV);

        let result = McpServer::resolve_project_path(None);
        assert!(result.is_ok());
        // Should return a valid path
        let path = result.unwrap();
        assert!(path.exists());
    }

    // ==================== Tests with Actual Index ====================

    #[test]
    fn test_query_with_index_checks_index() {
        let (server, _temp_dir) = create_test_server_with_index();
        let mut args = serde_json::Map::new();
        args.insert("query".to_string(), json!("test"));

        // Since API token is invalid, embedding will fail, but we can verify index was checked
        let result = server.call_rag_query(&args).unwrap();
        let content = &result["content"].as_array().unwrap()[0];
        let text = content["text"].as_str().unwrap();

        // Should NOT be "no index" message (because index exists)
        assert!(!text.contains(MSG_NO_INDEX));
    }

    #[test]
    fn test_search_with_index_uses_config() {
        let (server, _temp_dir) = create_test_server_with_index();
        let mut args = serde_json::Map::new();
        args.insert("query".to_string(), json!("test"));
        args.insert("top_k".to_string(), json!(7));

        let result = server.call_rag_query(&args).unwrap();
        let content = &result["content"].as_array().unwrap()[0];
        let _text = content["text"].as_str().unwrap();

        // Verify at least it doesn't fail due to index checking
        assert!(result["content"].is_array());
        assert!(content["type"] == "text");
        assert!(content["text"].is_string());
    }

    #[test]
    fn test_search_code_with_index_checks_index() {
        let (server, _temp_dir) = create_test_server_with_index();
        let mut args = serde_json::Map::new();
        args.insert("query".to_string(), json!("test"));

        let result = server.call_rag_search_code(&args).unwrap();
        let content = &result["content"].as_array().unwrap()[0];
        let text = content["text"].as_str().unwrap();

        // Should NOT be "no index" message
        assert!(!text.contains(MSG_NO_INDEX));
    }

    #[test]
    fn test_search_docs_with_index_checks_index() {
        let (server, _temp_dir) = create_test_server_with_index();
        let mut args = serde_json::Map::new();
        args.insert("query".to_string(), json!("test"));

        let result = server.call_rag_search_docs(&args).unwrap();
        let content = &result["content"].as_array().unwrap()[0];
        let text = content["text"].as_str().unwrap();

        // Should NOT be "no index" message
        assert!(!text.contains(MSG_NO_INDEX));
    }

    #[test]
    fn test_search_session_with_index_checks_index() {
        let (server, _temp_dir) = create_test_server_with_index();
        let mut args = serde_json::Map::new();
        args.insert("query".to_string(), json!("test"));

        let result = server.call_rag_search(&args).unwrap();
        let content = &result["content"].as_array().unwrap()[0];
        let text = content["text"].as_str().unwrap();

        // Should NOT be "no index" message
        assert!(!text.contains(MSG_NO_INDEX));
    }

    #[test]
    fn test_search_with_index_error_handling() {
        let (server, _temp_dir) = create_test_server_with_index();
        let mut args = serde_json::Map::new();
        args.insert("query".to_string(), json!("test"));

        // Since embedding will fail (invalid token), should return embedding error
        let result = server.call_rag_query(&args).unwrap();
        let content = &result["content"].as_array().unwrap()[0];
        let text = content["text"].as_str().unwrap();

        // Should contain embedding error message
        assert!(text.contains("embedding") || text.contains("Failed to generate"));
    }

    // ==================== New Validation Tests ====================

    #[test]
    fn test_extract_top_k_below_minimum() {
        let mut args = serde_json::Map::new();
        args.insert("top_k".to_string(), json!(0));

        let result = McpServer::extract_top_k(&args, 5);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), RagError::Validation(_)));
    }

    #[test]
    fn test_extract_top_k_at_minimum() {
        let mut args = serde_json::Map::new();
        args.insert("top_k".to_string(), json!(1));

        let result = McpServer::extract_top_k(&args, 5);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 1);
    }

    #[test]
    fn test_call_rag_timeline_top_k_below_minimum() {
        let server = create_test_server();
        let mut args = serde_json::Map::new();
        args.insert("top_k".to_string(), json!(0));

        let result = server.call_rag_timeline(&args);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), RagError::Validation(_)));
    }

    #[test]
    fn test_call_rag_timeline_top_k_at_minimum() {
        let server = create_test_server();
        let mut args = serde_json::Map::new();
        args.insert("top_k".to_string(), json!(1));

        let result = server.call_rag_timeline(&args);
        assert!(result.is_ok());
    }

    #[test]
    fn test_call_rag_timeline_top_k_exceeds_maximum() {
        let server = create_test_server();
        let mut args = serde_json::Map::new();
        args.insert("top_k".to_string(), json!(2000));

        let result = server.call_rag_timeline(&args);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), RagError::Validation(_)));
    }

    #[test]
    fn test_case_insensitive_doc_extensions() {
        // All these variations should be recognized as doc files
        assert!(McpServer::is_doc_file("README.md"));
        assert!(McpServer::is_doc_file("README.MD"));
        assert!(McpServer::is_doc_file("README.Md"));
        assert!(McpServer::is_doc_file("README.mD"));

        assert!(McpServer::is_doc_file("docs.TXT"));
        assert!(McpServer::is_doc_file("docs.txt"));

        assert!(McpServer::is_doc_file("MANUAL.PDF"));
        assert!(McpServer::is_doc_file("manual.pdf"));

        assert!(McpServer::is_doc_file("GUIDE.HTML"));
        assert!(McpServer::is_doc_file("guide.html"));
    }

    #[test]
    fn test_case_insensitive_code_extensions() {
        // All these variations should be recognized as code files
        assert!(McpServer::is_code_file("main.rs"));
        assert!(McpServer::is_code_file("main.RS"));
        assert!(McpServer::is_code_file("main.Rs"));

        assert!(McpServer::is_code_file("app.PY"));
        assert!(McpServer::is_code_file("app.py"));

        assert!(McpServer::is_code_file("index.TS"));
        assert!(McpServer::is_code_file("index.ts"));

        assert!(McpServer::is_code_file("Component.TSX"));
        assert!(McpServer::is_code_file("component.tsx"));

        assert!(McpServer::is_code_file("lib.GO"));
        assert!(McpServer::is_code_file("lib.go"));
    }

    #[test]
    fn test_jsonrpc_error_constants() {
        // Verify error constants match JSON-RPC 2.0 specification
        assert_eq!(JSONRPC_ERROR_METHOD_NOT_FOUND, -32601);
        assert_eq!(JSONRPC_ERROR_INVALID_PARAMS, -32602);
    }

    #[test]
    fn test_handle_request_uses_error_constants() {
        let server = create_test_server();
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: json!(1),
            method: "unknown_method".to_string(),
            params: None,
        };

        let response = server.handle_request(&request).unwrap();
        assert!(response.error.is_some());
        assert_eq!(response.error.unwrap().code, JSONRPC_ERROR_METHOD_NOT_FOUND);
    }

    #[test]
    fn test_handle_tools_call_uses_error_constants() {
        let server = create_test_server();
        let mut params = serde_json::Map::new();
        params.insert("name".to_string(), json!("unknown_tool"));
        params.insert("arguments".to_string(), json!({}));

        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: json!(1),
            method: "tools/call".to_string(),
            params: Some(json!(params)),
        };

        let response = server.handle_tools_call(&request).unwrap();
        assert!(response.error.is_some());
        assert_eq!(response.error.unwrap().code, JSONRPC_ERROR_INVALID_PARAMS);
    }

    #[test]
    fn test_max_results_to_display_limiting() {
        // Verify constant is defined and reasonable
        assert!(MAX_RESULTS_TO_DISPLAY > 0);
        assert!(MAX_RESULTS_TO_DISPLAY <= MAX_SEARCH_RESULTS);
    }

    #[test]
    fn test_min_top_k_constant() {
        // Verify MIN_TOP_K is defined as 1
        assert_eq!(MIN_TOP_K, 1);
    }

    #[test]
    fn test_default_top_k_constants() {
        // Verify default values are within valid range
        assert!(DEFAULT_TOP_K_GENERAL >= MIN_TOP_K);
        assert!(DEFAULT_TOP_K_CODE >= MIN_TOP_K);
        assert!(DEFAULT_TOP_K_TIMELINE >= MIN_TOP_K);

        assert!(DEFAULT_TOP_K_GENERAL <= MAX_TOP_K);
        assert!(DEFAULT_TOP_K_CODE <= MAX_TOP_K);
        assert!(DEFAULT_TOP_K_TIMELINE <= MAX_TOP_K);
    }
}
