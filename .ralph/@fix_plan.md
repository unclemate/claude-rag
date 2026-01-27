# Claude RAG - Ralph Development Plan

> 本计划基于 README.md、DESIGN.md、PLAN.md、DEVELOPMENT.md 综合制定

---

## Phase 1: Basic Framework

### 1.1 Project Structure Setup
- [ ] Set up `src/` directory structure per DEVELOPMENT.md
- [ ] Create module scaffolding:
  ```
  src/
  ├── main.rs              # CLI entry point
  ├── lib.rs               # Library exports
  ├── config.rs            # Configuration management
  ├── error.rs             # Centralized error types
  ├── models/              # Data models
  │   ├── mod.rs
  │   ├── session.rs
  │   ├── message.rs
  │   ├── file.rs
  │   ├── symbol.rs
  │   ├── commit.rs       # ⭐ Git
  │   └── diff.rs         # ⭐ Git
  ├── storage/             # Storage layer
  │   ├── mod.rs
  │   ├── sled.rs         # KV database wrapper
  │   └── hnsw.rs         # Vector index implementation
  ├── collector/           # Data collectors
  │   ├── mod.rs
  │   ├── session.rs      # Session collection
  │   ├── file.rs         # File scanning
  │   └── git.rs          # ⭐ Git collector
  ├── retrieval/           # ⭐ Time-aware retrieval
  │   ├── mod.rs
  │   ├── confidence.rs   # Confidence level calculation
  │   ├── decay.rs        # Temporal decay calculator
  │   └── timeline.rs     # Timeline builder
  ├── embedding.rs         # Zhipu AI API client
  ├── parser.rs            # Session JSONL parsing
  ├── scanner.rs           # File scanning with .gitignore
  ├── ast.rs               # AST parsing (Tree-sitter)
  ├── vector.rs            # Vector building (chunking)
  ├── hook.rs              # Hook system
  ├── daemon.rs            # Daemon service
  ├── skills.rs            # Skills generator
  ├── mcp.rs               # MCP Server
  └── formatter.rs         # ⭐ Result formatting

  tests/
  ├── integration/
  │   ├── mod.rs
  │   ├── e2e_tests.rs
  │   └── git_integration.rs
  └── fixtures/

  benches/
  └── hnsw_bench.rs        # HNSW benchmarks
  ```
- [ ] Set up `tests/` directory structure
- [ ] Set up `benches/` directory

### 1.2 Config Module (`src/config.rs`)
- [ ] Define `Config` struct with all settings
- [ ] Implement global config reading from `~/.claude/rag/config.toml`
- [ ] Implement project config reading from `{projectPath}/.rag/config.json`
- [ ] Config priority: project > global > defaults
- [ ] Add validation for required fields (API token)
- [ ] Add tests for config parsing

### 1.3 Error Types (`src/error.rs`)
- [ ] Define `RagError` enum with thiserror
- [ ] Include variants: Io, Sled, Git, Embedding, Config, NotFound
- [ ] Define `Result<T>` type alias
- [ ] Add error conversion implementations

### 1.4 CLI Framework (`src/main.rs`)
- [ ] Set up clap with derive API
- [ ] Define subcommands:
  - `init` - Initialize knowledge base
  - `index` - Index existing sessions/files
  - `daemon` - Start monitoring service
  - `query` - Semantic query
  - `status` - Check status
  - `mcp-server` - Start MCP server
  - `install-skills` - Install Skills
- [ ] Add global options: `--verbose`, `--config`
- [ ] Implement basic command routing

---

## Phase 2: Data Models & Parsing

### 2.1 Core Models (`src/models/`)
- [ ] Define `ContentType` enum (Session, Message, File, Symbol, Commit, GitDiff)
- [ ] Define `Session` struct
- [ ] Define `Message` struct
- [ ] Define `File` struct
- [ ] Define `Symbol` struct
- [ ] Add Serialize/Serialize derives
- [ ] Add unit tests

### 2.2 Git Models (`src/models/commit.rs`, `src/models/diff.rs`) ⭐
- [ ] Define `Commit` struct with:
  - hash, shortHash, author, committer
  - message, messageSummary
  - commitDate, parentHashes
  - filesChanged, insertions, deletions
- [ ] Define `GitDiff` struct with:
  - filePath, oldOid, newOid
  - diffContent, diffSummary
  - addedLines, removedLines
  - beforeContext, afterContext
- [ ] Add Conventional Commits parsing support
- [ ] Add BREAKING CHANGE extraction

### 2.3 Parser Module (`src/parser.rs`)
- [ ] Implement `scan_claude_projects()` - Find all projects
- [ ] Implement `parse_sessions_index()` - Read metadata
- [ ] Implement `parse_jsonl_file()` - Parse session files
- [ ] Implement incremental parsing support
- [ ] Add error handling for malformed JSONL
- [ ] Add tests with sample data

---

## Phase 3: Embedding Service

### 3.1 Embedding Client (`src/embedding.rs`)
- [ ] Define `EmbeddingClient` struct
- [ ] Implement Zhipu AI API client with reqwest
- [ ] Implement batch embedding (max 8 items)
- [ ] Add retry mechanism with exponential backoff
- [ ] Add timeout handling (30s default)
- [ ] Implement error handling for API failures
- [ ] Add tests with mock API

### 3.2 Vector Builder (`src/vector.rs`)
- [ ] Define text chunking strategies:
  - Sessions: per message
  - Source: per symbol + file summary
  - Docs: per paragraph + file summary
- [ ] Implement metadata association
- [ ] Implement vector normalization (L2)
- [ ] Add chunking tests

---

## Phase 4: Storage Layer (Core)

### 4.1 Sled Wrapper (`src/storage/sled.rs`)
- [ ] Define `StorageManager` struct
- [ ] Implement `open_project_db()` - Create/open project database
- [ ] Implement KV operations:
  - `session:{id}` → Session data
  - `message:{id}` → Message data
  - `file:{id}` → File data
  - `symbol:{id}` → Symbol data
  - `commit:{id}` → Commit data ⭐
  - `diff:{id}` → GitDiff data ⭐
  - `index_state:{id}` → Index progress
- [ ] Implement auxiliary indexes:
  - `session_messages:{sessionId}` → [messageIds]
  - `project_files:{projectPath}` → [fileIds]
  - `file_symbols:{fileId}` → [symbolIds]
  - `file_commits:{projectPath}:{filePath}` → [commitIds] ⭐
- [ ] Add transaction support
- [ ] Add tests with temp directories

### 4.2 HNSW Index (`src/storage/hnsw.rs`)
- [ ] Define `HNSWNode` struct with vector and neighbors
- [ ] Define `HNSWIndex` struct with layers
- [ ] Implement `new()` with configurable parameters (m, efConstruction)
- [ ] Implement `insert()` algorithm:
  - Greedy search for nearest neighbors
  - Update connections at each layer
  - Build more connections at layer 0
- [ ] Implement `search()` algorithm:
  - Search down from entry_point
  - Return top-k nearest neighbors
- [ ] Implement `save()` - Persist to disk
- [ ] Implement `load()` - Load from disk
- [ ] Add type filtering support
- [ ] Add tests with known vectors

---

## Phase 5: File Scanning & Parsing

### 5.1 File Scanner (`src/scanner.rs`)
- [ ] Implement `.gitignore` parsing (use ignore crate)
- [ ] Implement file classification (source/doc/other)
- [ ] Implement change detection (mtime + hash)
- [ ] Implement incremental scanning
- [ ] Add tests

### 5.2 AST Parser (`src/ast.rs`)
- [ ] Set up Tree-sitter for multiple languages
- [ ] Implement symbol extraction:
  - Functions
  - Classes/Structs
  - Methods
  - Traits
- [ ] Extract doc comments
- [ ] Map line numbers
- [ ] Add language support: Rust, Python, TypeScript, JavaScript
- [ ] Add tests

### 5.3 Document Parser (`src/parser.rs` - extension)
- [ ] Implement Markdown segmentation
- [ ] Extract code blocks
- [ ] Segment by headers
- [ ] Add tests

---

## Phase 5a: Git History Collection ⭐

### 5a.1 Git Collector (`src/collector/git.rs`)
- [ ] Define `GitCollector` struct using git2
- [ ] Implement `new(project_path)` - Open repository
- [ ] Implement `collect_all_commits()` - Get commit history
- [ ] Implement `get_file_diff()` - Extract diffs
- [ ] Implement `parse_conventional_commits()` - Parse commit messages
- [ ] Implement `extract_breaking_changes()` - Find BREAKING CHANGE
- [ ] Add error handling for non-Git projects
- [ ] Add tests with test repositories

### 5a.2 Git Integration with Storage
- [ ] Extend `StorageManager` with Git operations
- [ ] Implement commit storage
- [ ] Implement diff storage
- [ ] Update auxiliary indexes for Git data
- [ ] Add integration tests

---

## Phase 6: Hook + Daemon

### 6.1 Hook System (`src/hook.rs`)
- [ ] Implement `generate_session_start_hook()` script
- [ ] Implement `install_hooks()` - Copy to `~/.claude/hooks/`
- [ ] Handle environment variables (CLAUDE_SESSION_ID, CLAUDE_PROJECT_PATH)
- [ ] Add daemon notification mechanism

### 6.2 Daemon Service (`src/daemon.rs`)
- [ ] Implement `Daemon` struct
- [ ] Implement Unix socket/server for communication
- [ ] Implement `SessionManager` for active sessions
- [ ] Implement fsnotify-based file watching
- [ ] Implement incremental indexing on file changes
- [ ] Implement session timeout detection (60s)
- [ ] Implement HNSW persistence interval (300s)
- [ ] Add PID file management
- [ ] Add signal handling (SIGTERM, SIGINT)
- [ ] Add tests

### 6.3 Daemon Commands
- [ ] `daemon start` - Start background service
- [ ] `daemon stop` - Stop service
- [ ] `daemon status` - Check status
- [ ] `daemon restart` - Restart service

---

## Phase 7: Time-Aware Retrieval ⭐

### 7.1 Confidence Engine (`src/retrieval/confidence.rs`)
- [ ] Define `ConfidenceLevel` enum (Highest=5, High=4, Medium=3, Low=2, Lowest=1)
- [ ] Implement `base_weight()` method
- [ ] Implement `from_content_type()` - Calculate from content type and timestamp
- [ ] Implement Git state synchronization check
- [ ] Add tests

### 7.2 Time Decay Calculator (`src/retrieval/decay.rs`)
- [ ] Implement exponential decay function
- [ ] Add configurable decay rates per content type
- [ ] Combine with semantic similarity
- [ ] Add tests

### 7.3 Timeline Builder (`src/retrieval/timeline.rs`)
- [ ] Define `FeatureTimeline` struct
- [ ] Define `TimelineEvent` enum (GitChange, Discussion, Implementation)
- [ ] Implement timeline building from indexed content
- [ ] Cluster events by feature/topic
- [ ] Identify current state vs historical
- [ ] Sort chronologically
- [ ] Add tests

### 7.4 Enhanced Results (`src/results.rs`)
- [ ] Define `EnhancedItem` struct with:
  - similarity, temporal_weight, final_score
  - confidence_level, age_description
  - git_info (optional)
  - is_current, is_deprecated, superseded_by
- [ ] Define `GitInfo` struct
- [ ] Define `SupersededInfo` struct
- [ ] Add constructors and helper methods

### 7.5 Result Formatter (`src/formatter.rs`)
- [ ] Implement markdown output with confidence badges
- [ ] Implement timeline visualization
- [ ] Implement Git context formatting
- [ ] Implement color-coded age indicators
- [ ] Add format options (json, markdown, text)

---

## Phase 8: Claude Code Integration

### 8.1 Skills Generator (`src/skills.rs`)
- [ ] Implement `rag-query.sh` template
- [ ] Implement `rag-code.sh` template
- [ ] Implement `rag-docs.sh` template
- [ ] Implement `rag-session.sh` template
- [ ] Implement `install_skills()` - Install to `~/.claude/skills/`
- [ ] Add `install-skills` CLI command
- [ ] Add tests

### 8.2 MCP Server (`src/mcp.rs`)
- [ ] Implement MCP protocol (stdio communication)
- [ ] Define tools:
  - `rag_query` - Query all content
  - `rag_search_code` - Search source
  - `rag_search_docs` - Search docs
  - `rag_search_session` - Search sessions
  - `rag_timeline` - ⭐ Build timeline
- [ ] Implement request/response handling
- [ ] Add error responses
- [ ] Add `mcp-server` CLI command
- [ ] Add integration tests

### 8.3 MCP Configuration
- [ ] Implement MCP config generator
- [ ] Support Claude Code settings.json format
- [ ] Add `setup-mcp` CLI command

---

## Phase 9: Feature Refinement

### 9.1 Error Handling
- [ ] Add comprehensive error context with anyhow
- [ ] Improve error messages
- [ ] Add error recovery strategies
- [ ] Add logging with tracing

### 9.2 Logging
- [ ] Set up tracing-subscriber
- [ ] Add log levels (error, warn, info, debug, trace)
- [ ] Add structured logging
- [ ] Add log file support

### 9.3 Progress Display
- [ ] Add progress bars for indexing
- [ ] Add statistics display
- [ ] Add ETA calculation
- [ ] Add verbose mode

### 9.4 Testing
- [ ] Add unit tests for all modules (target: 85% coverage) ⭐
- [ ] Add integration tests
- [ ] Add E2E tests
- [ ] Add property-based tests (proptest)
- [ ] Set up cargo-tarpaulin for CI

### 9.5 Documentation
- [ ] Add rustdoc comments to all public APIs
- [ ] Add module-level documentation
- [ ] Add examples in docs
- [ ] Generate and host docs

### 9.6 Performance
- [ ] Add benchmarks for HNSW operations
- [ ] Add benchmarks for embedding API
- [ ] Optimize hot paths
- [ ] Profile with flamegraph

---

## CLI Commands Checklist

### `claude-rag init`
- [ ] Create `.rag/` directory
- [ ] Initialize sled database
- [ ] Initialize HNSW index
- [ ] Create default config

### `claude-rag index`
- [ ] `--all` - Index all projects
- [ ] `--project <path>` - Index specific project
- [ ] `--force` - Force re-index
- [ ] `--type <type>` - Index specific type (sessions, source, docs, git)
- [ ] Progress display

### `claude-rag daemon`
- [ ] `start` - Start daemon
- [ ] `stop` - Stop daemon
- [ ] `status` - Check status
- [ ] `restart` - Restart daemon

### `claude-rag query`
- [ ] Basic query
- [ ] `--type <type>` - Filter by content type
- [ ] `--top-k <n>` - Number of results
- [ ] `--timeline` - ⭐ Show timeline
- [ ] `--file <path>` - ⭐ Query specific file
- [ ] `--show-diffs` - ⭐ Show git diffs
- [ ] `--format <format>` - Output format

### `claude-rag status`
- [ ] Show database stats
- [ ] Show index status
- [ ] Show daemon status

### `claude-rag mcp-server`
- [ ] Start MCP server via stdio
- [ ] Handle MCP protocol
- [ ] Expose tools

### `claude-rag install-skills`
- [ ] Generate skill scripts
- [ ] Install to `~/.claude/skills/`
- [ ] Verify installation

---

## Dependencies (Cargo.toml)

```toml
[dependencies]
# Async
tokio = { version = "1", features = ["full"] }

# HTTP
reqwest = { version = "0.12", features = ["json"] }

# Serialization
serde = { version = "1", features = ["derive"] }
serde_json = "1"
toml = "0.8"

# CLI
clap = { version = "4", features = ["derive"] }

# Error handling
anyhow = "1"
thiserror = "1"

# Database
sled = "0.34"

# File monitoring
notify = "6"

# Git ⭐
git2 = "0.18"

# Paths
dirs = "5"
walkdir = "2"
ignore = "0.4"

# Tree-sitter
tree-sitter = "0.22"
tree-sitter-rust = "0.21"
tree-sitter-python = "0.21"
tree-sitter-javascript = "0.21"
tree-sitter-typescript = "0.21"

# Logging
tracing = "0.1"
tracing-subscriber = "0.3"

[dev-dependencies]
tempfile = "3"
criterion = "0.5"
proptest = "1"
cargo-tarpaulin = "0.30"
```

---

## Completed

- [x] Ralph project initialized
- [x] Development plan created from documentation

---

## Notes

**Priority Guidelines:**
1. **Focus on MVP**: Complete Phase 1-4 before Phase 5+
2. **Git Features (⭐)**: Can be implemented after basic retrieval works
3. **Testing**: Maintain 85% coverage requirement
4. **One Task Per Loop**: Follow Ralph's philosophy

**Key Principles:**
- KISS: Keep implementation simple
- DRY: Avoid code duplication
- SOLID: Follow single responsibility principle
- Test-driven: Write tests alongside code
