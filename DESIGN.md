# Claude RAG Implementation Plan

## Project Overview

Develop a Rust tool to build a **complete time-aware RAG knowledge base** for projects, including:
- **Session Records**: Claude Code interaction history (user input + AI output)
- **Source Files**: Project code with hierarchical indexing (file-level + function/class-level)
- **Code Symbols**: ⭐ Symbol-level indexing with branch awareness (functions, classes, methods)
- **Documentation**: README, design docs, comments, etc.
- **Other Files**: Configs, test cases, etc.
- **Git History**: Commit history and diffs with temporal context ⭐

## Tech Stack

| Component | Selection |
|-----------|-----------|
| Language | Rust 2024 Edition |
| Vector Database | **sled (embedded KV)** + **HNSW algorithm** (self-implemented) |
| Embedding Model | Zhipu AI embedding-3 API |
| Data Source | Claude Code session JSONL files |
| HTTP Client | reqwest |
| JSON Parsing | serde_json |
| Hook System | session-start + daemon file monitoring |
| Git Integration | **git2** for commit/diff history ⭐ |

---

## Architecture Design

```
┌─────────────────────────────────────────────────────────────────────────┐
│                           Claude RAG                                    │
├─────────────────────────────────────────────────────────────────────────┤
│                                                                          │
│  ┌─────────────┐    ┌─────────────┐    ┌─────────────┐                 │
│  │   Config    │    │   Parser    │    │  Embedding  │                 │
│  │   Module    │───▶│   Module    │───▶│   Client    │                 │
│  └─────────────┘    └─────────────┘    └─────────────┘                 │
│         │                  │                  │                         │
│         ▼                  ▼                  ▼                         │
│  ┌─────────────┐    ┌─────────────┐    ┌─────────────┐                 │
│  │   Storage   │◀───│   Vector    │◀───│  ZhipuAI    │                 │
│  │   Manager   │    │   Builder   │    │    API      │                 │
│  └─────────────┘    └─────────────┘    └─────────────┘                 │
│         │                  │                                          │
│         ▼                  ▼                                          │
│  ┌─────────────────────────────────────┐    ┌──────────────────┐      │
│  │      sled + HNSW (local embedded)    │    │   Hook/Daemon    │      │
│  │  ┌─────────────────────────────┐    │    │                  │      │
│  │  │  sled (KV storage)          │    │    │ session-start    │      │
│  │  │  - sessions (sessions)      │    │    │   ↓              │      │
│  │  │  - messages (messages)      │    │    │ start file watch │      │
│  │  │  - files (source/docs)      │    │    │   ↓              │      │
│  │  │  - symbols (functions/classes)│   │    │ fsnotify watch   │      │
│  │  │  - index_state              │    │    │ session/file changes│     │
│  │  └─────────────────────────────┘    │    │   ↓              │      │
│  │  ┌─────────────────────────────┐    │    │ incremental      │      │
│  │  │  HNSW (vector index)        │    │    │ parse+vectorize  │      │
│  │  │  - unified index all content│    │    └──────────────────┘      │
│  │  │  - filter by type query     │    │                                │
│  │  └─────────────────────────────┘    │    ┌──────────────────┐      │
│  │                                      │    │  File Scanner    │      │
│  │                                      │    │  - source files  │      │
│  │                                      │    │  - doc files     │      │
│  │                                      │    │  - .gitignore    │      │
│  │                                      │    └──────────────────┘      │
│  └─────────────────────────────────────┘                                │
│                                                                          │
└─────────────────────────────────────────────────────────────────────────┘
```

**Data Sources:**
| Source | Parse Method | Index Granularity |
|--------|-------------|-------------------|
| **Session JSONL** | Parser Module | Per message |
| **Source Files** | FileScanner + AST | File-level + function/class-level |
| **Doc Files** | FileScanner | File-level + paragraph-level |
| **Other Files** | FileScanner | File-level |

**Data Storage Design:**
- **Storage Location**: `.rag/` folder in each project directory
  ```
  {projectPath}/.rag/
  ├── config.json          # Project-level config (optional, overrides global)
  ├── db/                  # sled database files
  │   ├── sled-data
  │   └── ...
  ├── hnsw.bin             # HNSW index snapshot
  └── git_sync_cache.json  # ⭐ Git sync persistent cache (two-tier caching)
  ```

- **Global Config** (`~/.claude/rag/config.toml`):
  ```toml
  # Zhipu AI Embedding API Configuration
  [embedding]
  api_token = "your-zhipu-api-token"           # Required: Zhipu AI API token
  api_url = "https://open.bigmodel.cn/api/paas/v4/embeddings"  # Optional: API endpoint
  dimensions = 1024                             # Vector dimensions
  batch_size = 8                                # Batch request size
  timeout_ms = 30000                            # Request timeout

  # HNSW Index Parameters
  [hnsw]
  m = 16                # Max connections per layer
  ef_construction = 200 # Search width during build
  ef_search = 50        # Search width during query

  # Indexing Options
  [index]
  index_source = true   # Whether to index source files
  index_docs = true     # Whether to index documentation
  index_other = false   # Whether to index other files

  # Daemon Options
  [daemon]
  session_timeout_seconds = 60  # Session timeout detection
  persist_interval_seconds = 300 # HNSW persist interval
  ```

- **Project Config** (`{projectPath}/.rag/config.json`):
  ```json
  {
    "enabled": true,
    "embedding": {
      "dimensions": 1024,
      "batchSize": 8
    },
    "hnsw": {
      "m": 16,
      "efConstruction": 200,
      "efSearch": 50
    }
  }
  ```

- **sled**: Embedded KV database for structured data
  - Key: `session:{id}` → Value: Session JSON
  - Key: `message:{id}` → Value: Message JSON
  - Key: `commit:{id}` → Value: Commit JSON ⭐
  - Key: `diff:{id}` → Value: GitDiff JSON ⭐
  - Key: `index_state:{id}` → Value: Index state

- **HNSW**: In-memory vector index for fast approximate search
  - Nodes: Vector representation of each message
  - Edges: Graph connections based on similarity
  - Supports persistence to disk, loaded on startup

### Hook/Daemon Workflow

```
Claude Code session starts
        │
        ▼
┌─────────────────────────────────────┐
│   session-start Hook triggered      │
│   ├─ Receive env variables          │
│   │   - CLAUDE_SESSION_ID           │
│   │   - CLAUDE_PROJECT_PATH         │
│   └─ Notify daemon: start watching  │
└─────────────────────────────────────┘
                │
                ▼
┌─────────────────────────────────────┐
│   Daemon                            │
│   ├─ Continuous run, manage multiple│
│   │   session watches               │
│   ├─ Use fsnotify to watch .jsonl   │
│   └─ On file change detected:       │
│       ├─ Read new JSONL lines       │
│       ├─ Check if message indexed   │
│       ├─ Call embedding API         │
│       ├─ Write to sled + HNSW       │
│       └─ Update index_state         │
└─────────────────────────────────────┘
                │
                ▼
┌─────────────────────────────────────┐
│   Session end detection            │
│   ├─ File unchanged for N seconds  │
│   ├─ Or session close signal       │
│   └─ Clean up session watch        │
└─────────────────────────────────────┘
```

**Key Changes:**
- ✅ Use `session-start` hook (confirmed available)
- ✅ Daemon runs continuously, manages file watches for multiple sessions
- ✅ Detect session end via file timeout or signal (replacing unreliable session-end)
- ✅ Single daemon process, avoiding multi-process overhead

---

## Core Modules

### 1. Config Module (`src/config.rs`)
- Read global config `~/.claude/rag/config.toml`
  - API token and endpoint for Zhipu AI embedding
  - HNSW parameters (m, efConstruction, efSearch)
  - Embedding parameters (dimensions, batchSize, timeout)
  - Index scope options (source/docs/other)
  - Daemon options (session timeout, persist interval)
- Read project config `{projectPath}/.rag/config.json` (optional)
  - Override global HNSW parameters
  - Override embedding parameters
  - Project-specific enable/disable
- Config priority: project config > global config > defaults
- Config format: TOML (global), JSON (project)

### 2. Parser Module (`src/parser.rs`)
- Scan `~/.claude/projects/` directory
- Read `sessions-index.json` for session metadata
- Parse `.jsonl` session files
- Extract: user messages, AI replies, code snippets, tool call results
- **Incremental parsing**: Only process new/changed messages

### 3. FileScanner Module (`src/scanner.rs`)
- **Scan project files**:
  - Respect `.gitignore` rules
  - Classify by file type (source/doc/other)
  - Detect file changes (mtime + hash)
- **Source parsing**:
  - Use Tree-sitter to parse code AST
  - Extract functions, classes, structs and other symbols
  - Extract doc comments
- **Doc parsing**:
  - Markdown segmentation (by header/paragraph)
  - Code block extraction
- **Incremental updates**: Only index changed files

### 4. Embedding Client (`src/embedding.rs`)
- Wrap Zhipu AI embedding-3 API calls
- Batch request support (max 8 items)
- Error retry mechanism
- Vector dimensions: 1024 (confirmed)

### 5. Vector Builder (`src/vector.rs`)
- Text chunking strategy
  - Sessions: by message
  - Source: by symbol + file summary
  - Docs: by paragraph + file summary
- Metadata association (type, path, timestamp)
- Vector normalization

### 6. Storage Manager (`src/storage.rs`)
- **sled client wrapper**: Open/create project-level database
- **HNSW index management**: Vector insertion, search, persistence
- **Transaction support**: Ensure data consistency
- **Type filtering**: Query by content type

### 7. Hook Module (`src/hook.rs`)
- Hook script generation (install to `~/.claude/hooks/`)
- `session-start.sh`: Notify daemon to start watching that session
- **Daemon service**:
  - Manage file watches for multiple projects
  - Incremental JSONL change parsing (sessions)
  - Incremental project file scanning (source/docs)
  - Session end detection (file timeout)
  - Update corresponding project's `.rag/` database

### 8. CLI (`src/main.rs`)
- `init`: Initialize knowledge base
- `index`: Index existing sessions and project files
- `daemon`: Start background monitoring service
- `query`: Semantic query (can filter by type)
- `status`: Check knowledge base status
- `mcp-server`: Start MCP server
- `install-skills`: Install Skills to `~/.claude/skills/`

### 9. Skills Module (`src/skills.rs`)
- Skills script generator
- Generate `rag-query.sh`, `rag-code.sh`, `rag-docs.sh`, etc.
- Install to `~/.claude/skills/`

### 10. MCP Server (`src/mcp.rs`)
- MCP protocol implementation
- Provide tools: rag_query, rag_search_code, rag_search_docs, rag_search_session
- Communicate with Claude Code via stdio

### 11. Git Collector (`src/git_collector.rs`) ⭐
- Collect Git commit history using git2
- Parse commit metadata (hash, author, message, files changed)
- Extract file diffs with line-level changes
- Support Conventional Commits parsing

### 12. Confidence Engine (`src/confidence.rs`) ⭐
- Calculate confidence levels based on content type and timestamp
- Temporal weight calculation with decay
- Git state synchronization (check if code matches HEAD)
- **GitSync module with two-tier persistent caching** ⭐
  - L1: Memory LRU cache for hot data (default: 1000 entries)
  - L2: Disk persistent cache (default: 10000 entries)
  - HEAD change detection and automatic cache invalidation
  - File content hashing (SHA-256) for modification detection
  - Background persistence task (5-minute interval)
  - Graceful degradation for persistence failures
  - Cache file: `{projectPath}/.rag/git_sync_cache.json`

### 13. Time Decay Calculator (`src/time_decay.rs`) ⭐
- Exponential decay based on age
- Configurable decay rates per content type
- Combine with semantic similarity for final scoring

### 14. Timeline Builder (`src/timeline.rs`) ⭐
- Build feature evolution timelines
- Cluster related content by feature/topic
- Identify current state vs historical changes
- Generate chronological event sequences

### 15. Results Enhancer (`src/results.rs`) ⭐
- Enhanced result structure with Git metadata
- Superseded/desprecated detection
- Current state identification
- Timeline event formatting

### 16. Result Formatter (`src/formatter.rs`) ⭐
- Markdown output with confidence badges
- Timeline visualization
- Git context formatting
- Color-coded age indicators

---

## Data Structure Design

### sled KV Storage

**Content Type Enum:**
```rust
enum ContentType {
    Session,    // Session
    Message,    // Message
    File,       // File
    Symbol,     // Function/class symbols
    Commit,     // ⭐ Git commit
    GitDiff,    // ⭐ Git diff
}
```

**Session Data:**
```
Key:   session:{sessionId}
Value: {
  "sessionId": "...",
  "projectPath": "...",
  "summary": "...",
  "messageCount": 123,
  "gitBranch": "master",
  "createdAt": 1234567890,
  "updatedAt": 1234567890,
  "summaryVector": [0.1, 0.2, ...]  // 1024 dimensions
}
```

**Message Data:**
```
Key:   message:{messageId}
Value: {
  "messageId": "...",
  "sessionId": "...",
  "role": "user|assistant|tool",
  "content": "...",
  "timestamp": 1234567890,
  "vector": [0.1, 0.2, ...]  // 1024 dimensions
}
```

**Index State:**
```
Key:   index_state:{sessionId}
Value: {
  "sessionId": "...",
  "lastMessageId": "...",
  "lastFileMtime": 1234567890,
  "indexedCount": 100,
  "lastUpdateTime": 1234567890
}
```

**Auxiliary Indexes:**
```
Key:   session_messages:{sessionId}
Value: [messageId1, messageId2, ...]  // All message IDs for this session

Key:   project_files:{projectPath}
Value: [fileId1, fileId2, ...]        // All file IDs for this project

Key:   file_symbols:{fileId}
Value: [symbolId1, symbolId2, ...]    // All symbol IDs for this file

Key:   file_commits:{projectPath}:{filePath}  // ⭐
Value: [commitId1, commitId2, ...]    // Commits affecting this file (time-desc)

Key:   commit_sessions:{commitId}     // ⭐
Value: [sessionId1, sessionId2, ...]  // Sessions discussing this commit
```

**File Data:**
```
Key:   file:{fileId}
Value: {
  "fileId": "...",              // hash(filePath)
  "projectPath": "...",
  "filePath": "...",            // relative path: src/main.rs
  "fileType": "source|doc|other",
  "language": "rust|markdown|...",
  "size": 12345,
  "modifiedTime": 1234567890,
  "summaryVector": [0.1, 0.2, ...],  // File summary vector
  "indexedAt": 1234567890
}
```

**Symbol Data (function/class):**
```
Key:   symbol:{symbolId}
Value: {
  "symbolId": "...",            // hash(fileId:symbolName)
  "fileId": "...",
  "symbolType": "function|class|struct|enum|trait|...",
  "symbolName": "main",
  "signature": "fn main() -> Result<()>",
  "startLine": 10,
  "endLine": 25,
  "docComment": "Entry point...",
  "code": "fn main() { ... }",
  "vector": [0.1, 0.2, ...],
  "indexedAt": 1234567890
}
```

**⭐ Commit Data (Git):**
```
Key:   commit:{commitId}
Value: {
  "commitId": "...",            // hash("commit":hash)
  "projectPath": "...",
  "hash": "a1b2c3d4...",        // Full Git commit hash
  "shortHash": "a1b2c3d",
  "author": "Developer Name",
  "committer": "Developer Name",
  "message": "feat: add OAuth2 login\n\nThis implements...",
  "messageSummary": "feat: add OAuth2 login",
  "commitDate": 1234567890,
  "parentHashes": ["parent1", ...],
  "filesChanged": ["src/auth.rs", ...],
  "insertions": 50,
  "deletions": 10,
  "messageVector": [0.1, 0.2, ...],
  "indexedAt": 1234567890
}
```

**⭐ GitDiff Data:**
```
Key:   diff:{diffId}
Value: {
  "diffId": "...",              // hash("diff":commitHash:filePath)
  "commitId": "...",
  "filePath": "src/auth/login.rs",
  "oldOid": "abc123...",
  "newOid": "def456...",
  "diffContent": "@@ -10,5 +10,10 @@ ...",
  "diffSummary": "Add OAuth2 token handling",
  "addedLines": ["let token = ...", ...],
  "removedLines": ["let jwt = ...", ...],
  "beforeContext": "fn login() { ... }",
  "afterContext": "fn login() { ... }",
  "vector": [0.1, 0.2, ...],
  "indexedAt": 1234567890
}
```

**⭐ Confidence Level:**
```rust
enum ConfidenceLevel {
    Highest = 5,  // Current code/docs (matches latest Git state)
    High = 4,     // Git commit/diff (authoritative change records)
    Medium = 3,   // Recent session (≤7 days)
    Low = 2,      // Old session (7-30 days)
    Lowest = 1,   // Stale discussion (>30 days)
}

impl ConfidenceLevel {
    fn base_weight(&self) -> f32 {
        match self {
            Highest => 1.2,
            High => 1.0,
            Medium => 0.85,
            Low => 0.6,
            Lowest => 0.4,
        }
    }
}
```

### HNSW Vector Index

**Memory Structure:**
```
HNSWIndex {
  layers: [
    // Layer 0 (densest)
    Layer0: {
      node_id -> Node {
        vector: [f32; 1024],
        neighbors: [node_id],  // connections based on cosine similarity
        data: message_id
      }
    },
    // Layer 1+ (sparse, for fast skipping)
    Layer1: { ... },
    Layer2: { ... },
  ],
  entry_point: node_id  // search entry point
}
```

**Query Flow:**
```
query_vector → Start greedy search from top layer entry_point
            → Move down layer by layer, find nearest neighbor each layer
            → Exact search ef nearest neighbors at bottom (layer 0)
            → Return top-k results
```

---

## Incremental Indexing & Deduplication

### Scenario 1: Initial Indexing (manual trigger)

```
┌─────────────────────────────────────────────────────────────┐
│  Initial Indexing Flow                                     │
├─────────────────────────────────────────────────────────────┤
│                                                             │
│  1. Scan all projects under ~/.claude/projects/            │
│     ↓                                                       │
│  2. For each project:                                      │
│     ├─ Open {project}/.rag/db (sled)                       │
│     ├─ Read sessions-index.json                            │
│     └─ For each session:                                  │
│         ├─ Query index_state:{sessionId}                   │
│         ├─ If not exists → full index                      │
│         └─ If exists → check fileMtime                     │
│             └─ Unchanged → skip                            │
│             └─ Changed → incremental index                 │
│                                                             │
└─────────────────────────────────────────────────────────────┘
```

**Deduplication Strategy:**

| Condition | Action |
|-----------|--------|
| Project database doesn't exist | Create new database, full index |
| Session not indexed | Full index |
| Session indexed, fileMtime unchanged | Skip |
| Session indexed, fileMtime changed | Incremental index (continue from lastMessageId) |
| Message ID exists | Skip (sled insert overwrites, but check first to avoid duplicate work) |

### Scenario 2: Hook + Daemon Real-time Indexing

```
┌─────────────────────────────────────────────────────────────┐
│  session-start Hook Triggered                              │
├─────────────────────────────────────────────────────────────┤
│  1. Receive env variables:                                 │
│     - CLAUDE_SESSION_ID                                    │
│     - CLAUDE_PROJECT_PATH                                  │
│  2. Notify daemon via Unix socket / signal                │
│  3. Daemon registers watch for that session               │
└─────────────────────────────────────────────────────────────┘
                        │
                        ▼
┌─────────────────────────────────────────────────────────────┐
│  Daemon - Single process managing all projects            │
├─────────────────────────────────────────────────────────────┤
│  1. Use fsnotify to watch ~/.claude/projects/*/*.jsonl    │
│  2. On file WRITE event detected:                         │
│     ├─ Parse new JSONL lines                              │
│     ├─ Extract sessionId + messageId                      │
│     ├─ Check if message:{messageId} exists                │
│     ├─ If not exists →                                    │
│     │   ├─ Call embedding API                             │
│     │   ├─ Insert message:{id} → sled                     │
│     │   ├─ Insert vector to HNSW index                    │
│     │   └─ Update index_state:{sessionId}                 │
│     └─ If exists → skip (idempotent)                      │
│  3. Session end detection:                                │
│     ├─ File unchanged for 60 seconds                      │
│     ├─ Or project directory change detected               │
│     └─ Persist HNSW index to disk                         │
└─────────────────────────────────────────────────────────────┘
```

**Daemon Architecture:**
```
┌─────────────────────────────────────────────────────────┐
│  Daemon (single process)                               │
│  ├─ fsnotify: watch all .jsonl files                   │
│  ├─ SessionManager: manage active session list         │
│  ├─ Indexer: incremental index processor (thread pool) │
│  └─ HNSWIndex: independent memory index per project    │
│      ├─ project1 → HNSWIndex                           │
│      ├─ project2 → HNSWIndex                           │
│      └─ ...                                           │
└─────────────────────────────────────────────────────────┘
```

### Core Code Logic

```rust
// Check if session needs indexing
fn should_index_session(db: &sled::Db, session_id: &str, file_mtime: i64) -> IndexAction {
    let key = format!("index_state:{}", session_id);
    match db.get(&key) {
        Ok(Some(state_bytes)) => {
            let state: IndexState = serde_json::from_slice(&state_bytes)?;
            if state.last_file_mtime == file_mtime {
                return IndexAction::Skip;  // Unchanged, skip
            }
            IndexAction::Incremental(state.last_message_id)
        }
        _ => IndexAction::Full,  // New session, full index
    }
}

// Check if message is already indexed
fn is_message_indexed(db: &sled::Db, message_id: &str) -> bool {
    let key = format!("message:{}", message_id);
    db.get(&key).unwrap().is_some()
}

// Incremental index messages
async fn incremental_index(
    db: &sled::Db,
    hnsw: &mut HNSWIndex,
    session_id: &str,
    from_message_id: Option<&str>,
) -> Result<()> {
    let mut last_id = from_message_id;

    for line in read_jsonl_lines(session_id) {
        let msg = parse_message(&line)?;

        // Skip already indexed messages
        if is_message_indexed(db, &msg.id) {
            continue;
        }

        // Skip messages before from_message_id
        if let Some(ref from_id) = last_id {
            if msg.id != *from_id && !is_after(&msg.id, from_id) {
                continue;
            }
        }

        // Vectorize
        let vector = embed(&msg.content).await?;

        // Insert to sled
        let key = format!("message:{}", msg.id);
        let value = serde_json::to_vec(&msg.with_vector(&vector))?;
        db.insert(&key, value)?;

        // Insert to HNSW
        hnsw.insert(&vector, &msg.id)?;

        // Update index state
        update_index_state(db, session_id, &msg.id)?;
        last_id = Some(&msg.id);
    }
    Ok(())
}

// HNSW index structure (simplified)
struct HNSWIndex {
    layers: Vec<Vec<HNSWNode>>,
    entry_point: Option<usize>,
}

impl HNSWIndex {
    fn insert(&mut self, vector: &[f32], id: &str) -> Result<()> {
        // Implement HNSW insert algorithm
        // 1. Greedy search for nearest neighbors from top layer
        // 2. Update connections at each layer
        // 3. Build more connections at layer 0
    }

    fn search(&self, query: &[f32], k: usize) -> Vec<(String, f32)> {
        // Implement ANN search
        // 1. Search down from entry_point
        // 2. Return top-k nearest neighbors
    }
}
```

### Data Consistency Guarantees

| Mechanism | Description |
|-----------|-------------|
| **Primary Key Constraint** | `messageId` as PK, database-level duplicate prevention |
| **File Timestamp** | `fileMtime` determines if file changed |
| **State Tracking** | `index_state` records indexing progress |
| **Idempotent Operations** | Repeated execution has no side effects |

---

## File Manifest

```
src/
├── main.rs              # CLI entry point
├── lib.rs               # Library exports
├── config.rs            # Configuration management (global + project-level)
├── error.rs             # Centralized error types (thiserror)
│
├── models/              # Data model definitions
│   ├── mod.rs
│   ├── session.rs       # Session struct
│   ├── message.rs       # Message struct
│   ├── file.rs          # File struct
│   ├── symbol.rs        # Symbol struct with branch awareness ⭐
│   ├── commit.rs        # ⭐ Git commit struct
│   └── diff.rs          # ⭐ Git diff struct
│
├── storage/             # Storage layer (sled + HNSW)
│   ├── mod.rs
│   ├── sled.rs          # sled KV database wrapper with branch-aware storage ⭐
│   └── hnsw.rs          # HNSW vector index implementation
│
├── collector/           # Data collectors
│   ├── mod.rs
│   ├── session.rs       # Session collection from JSONL
│   ├── file.rs          # File scanning (source/docs)
│   └── git.rs           # ⭐ Git history collection (git2)
│
├── retrieval/           # ⭐ Time-aware retrieval
│   ├── mod.rs
│   ├── confidence.rs    # Confidence level calculation
│   ├── decay.rs         # Temporal decay calculator
│   ├── git_sync.rs      # ⭐ Git state sync with persistent caching
│   └── timeline.rs      # Timeline builder
│
├── ⭐ Code Symbol Indexing Module
│   ├── code_chunker.rs   # 3-tier code chunking (symbol/block/file) ⭐
│   ├── branch.rs         # Git branch detection and isolation ⭐
│   ├── symbol_cache.rs   # 3-tier symbol caching (L1/L2/L3) ⭐
│   └── indexer.rs        # Extended with symbol indexing ⭐
│
├── embedding.rs         # Zhipu AI embedding-3 API client
├── parser.rs            # Session JSONL parsing
├── scanner.rs           # File scanning with .gitignore support
├── ast.rs               # AST parsing (Tree-sitter wrapper)
├── vector.rs            # Vector building with code formatting ⭐
├── hook.rs              # Hook system (session-start)
├── daemon.rs            # Daemon service (fsnotify + incremental indexing)
├── skills.rs            # Skills generator
├── mcp.rs               # MCP Server with granularity parameter ⭐
├── query.rs             # Query executor with symbol support ⭐
└── formatter.rs         # ⭐ Result formatting (markdown + timeline)


tests/
├── integration/         # Integration tests
│   ├── mod.rs
│   ├── e2e_tests.rs
│   └── git_integration.rs
└── fixtures/            # Test fixtures

benches/
└── hnsw_bench.rs        # HNSW performance benchmarks

Cargo.toml               # Dependency config
hooks/
└── session-start.sh     # Generated hook script
skills/
├── rag-query.sh         # Generated: query all content
├── rag-code.sh          # Generated: search source
├── rag-docs.sh          # Generated: search docs
└── rag-session.sh       # Generated: search sessions
```

**File Descriptions:**

| File | Description |
|------|-------------|
| `src/main.rs` | CLI entry point with clap subcommands |
| `src/lib.rs` | Library exports for reuse |
| `src/config.rs` | Config reading from `~/.claude/rag/config.toml` and project `.rag/config.json` |
| `src/error.rs` | Centralized `RagError` enum with thiserror |
| **`src/models/`** | **Data model definitions (modular)** |
| `src/models/session.rs` | Session struct for Claude Code sessions |
| `src/models/message.rs` | Message struct (user/assistant/tool) |
| `src/models/file.rs` | File struct (source/docs) |
| `src/models/symbol.rs` | Symbol struct (functions/classes) with branch awareness ⭐ |
| `src/models/commit.rs` | ⭐ Git commit metadata |
| `src/models/diff.rs` | ⭐ Git diff content |
| **`src/storage/`** | **Storage layer (modular)** |
| `src/storage/sled.rs` | sled KV database wrapper with branch-aware symbol storage ⭐ |
| `src/storage/hnsw.rs` | HNSW vector index (self-implemented) |
| **`src/collector/`** | **Data collectors (modular)** |
| `src/collector/session.rs` | Session JSONL parsing |
| `src/collector/file.rs` | File scanning with .gitignore |
| `src/collector/git.rs` | ⭐ Git history via git2 |
| **`src/retrieval/`** | **⭐ Time-aware retrieval (modular)** |
| `src/retrieval/confidence.rs` | Confidence level (5 levels) |
| `src/retrieval/decay.rs` | Temporal decay calculator |
| `src/retrieval/git_sync.rs` | ⭐ Git state sync with two-tier persistent caching |
| `src/retrieval/timeline.rs` | Timeline builder |
| `src/embedding.rs` | Zhipu AI embedding-3 API client |
| `src/parser.rs` | Session JSONL parsing |
| `src/scanner.rs` | File scanner (source/docs classification) |
| `src/ast.rs` | Tree-sitter AST parser for code symbols |
| `src/vector.rs` | Vector building (chunking + normalization) |
| `src/hook.rs` | Hook system (session-start script generation) |
| `src/daemon.rs` | Daemon service (fsnotify, session management) |
| `src/skills.rs` | Skills generator (bash scripts) |
| `src/mcp.rs` | MCP Server (stdio communication) |
| `src/formatter.rs` | ⭐ Result formatter (markdown, timeline, Git context) |

---

## Dependency Config (Cargo.toml)

```toml
[dependencies]
# Async runtime
tokio = { version = "1", features = ["full"] }

# HTTP client
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

# Embedded database
sled = "0.34"

# File monitoring
notify = "6"

# Git operations ⭐
git2 = "0.18"

# Path handling
dirs = "5"
walkdir = "2"
ignore = "0.4"

# Code parsing (Tree-sitter)
tree-sitter = "0.22"
tree-sitter-rust = "0.21"
tree-sitter-javascript = "0.21"
tree-sitter-python = "0.21"
tree-sitter-typescript = "0.21"

# Logging
tracing = "0.1"
tracing-subscriber = "0.3"

# Vector computation
# Self-implemented cosine similarity and HNSW
```

---

## Claude Code Integration

### Method 1: Skills (User Manual Invocation)

**Workflow:**
```
User: /rag-query "How to handle login error?"
         │
         ▼
Claude Code calls ~/.claude/skills/rag-query.sh
         │
         ▼
Skill executes: claude-rag query "How to handle login error?"
         │
         ▼
Return search results → Claude displays to user
```

**Skills List:**

| Skill Name | Invocation | Function |
|------------|------------|----------|
| `rag-query` | `/rag-query "question"` | Query all types (sessions+source+docs) |
| `rag-code` | `/rag-code "function name"` | Search source only |
| `rag-docs` | `/rag-docs "topic"` | Search docs only |
| `rag-session` | `/rag-session "keyword"` | Search session history only |

**Skill Script Example (`rag-query.sh`):**
```bash
#!/usr/bin/env bash
# Claude Code Skill: RAG Query

QUERY="$*"

if [ -z "$QUERY" ]; then
  echo "Please enter query content"
  echo "Usage: /rag-query <query content>"
  exit 1
fi

# Get current project path
PROJECT_PATH="$(pwd)"

# Call claude-rag query
claude-rag query --project "$PROJECT_PATH" --type all "$QUERY"
```

---

### Method 2: MCP Server (Claude Auto Invocation)

**Workflow:**
```
User: "How was login handled in this project before?"
         │
         ▼
Claude determines need to query project context
         │
         ▼
Claude proactively calls MCP tool: mcp__rag__query("login handling")
         │
         ▼
MCP Server returns relevant context
         │
         ▼
Claude answers user based on context
```

**MCP Tool Definition:**

```json
{
  "name": "claude-rag-mcp",
  "version": "0.1.0",
  "tools": [
    {
      "name": "rag_query",
      "description": "Semantic query project RAG knowledge base (sessions+source+docs)",
      "inputSchema": {
        "type": "object",
        "properties": {
          "query": {
            "type": "string",
            "description": "Query content"
          },
          "top_k": {
            "type": "integer",
            "description": "Number of results to return, default 5",
            "default": 5
          }
        },
        "required": ["query"]
      }
    },
    {
      "name": "rag_search_code",
      "description": "Search project source code (functions, classes, etc.)",
      "inputSchema": {
        "type": "object",
        "properties": {
          "query": {"type": "string"},
          "language": {"type": "string", "description": "Programming language filter"}
        },
        "required": ["query"]
      }
    },
    {
      "name": "rag_search_docs",
      "description": "Search project documentation",
      "inputSchema": {
        "type": "object",
        "properties": {
          "query": {"type": "string"}
        },
        "required": ["query"]
      }
    },
    {
      "name": "rag_search_session",
      "description": "Search historical session records",
      "inputSchema": {
        "type": "object",
        "properties": {
          "query": {"type": "string"}
        },
        "required": ["query"]
      }
    }
  ]
}
```

**MCP Config (`~/.claude/settings.json`):**
```json
{
  "mcpServers": {
    "claude-rag": {
      "command": "claude-rag",
      "args": ["mcp-server"]
    }
  }
}
```

**MCP Server Return Format:**
```json
{
  "results": [
    {
      "type": "code",
      "content": "fn handle_login() -> Result<()> { ... }",
      "filePath": "src/auth/login.rs",
      "symbolName": "handle_login",
      "startLine": 10,
      "endLine": 25,
      "similarity": 0.92,
      "confidenceLevel": "Highest",
      "timestamp": 1737820800,
      "isCurrent": true
    },
    {
      "type": "commit",
      "content": "feat: add OAuth2 login",
      "commitHash": "a1b2c3d4...",
      "shortHash": "a1b2c3d",
      "author": "Developer",
      "commitDate": 1737724800,
      "filesChanged": ["src/auth/login.rs"],
      "similarity": 0.88,
      "confidenceLevel": "High"
    },
    {
      "type": "message",
      "content": "We discussed login error handling before...",
      "sessionId": "xxx",
      "timestamp": "2025-01-23T06:00:00Z",
      "similarity": 0.75,
      "confidenceLevel": "Medium",
      "ageDescription": "5 days ago"
    }
  ]
}
```

---

## Key File Paths

| Type | Path |
|------|------|
| **Global RAG config** | **`~/.claude/rag/config.toml`** |
| Claude sessions | `~/.claude/projects/*/sessions-index.json` |
| Session files | `~/.claude/projects/*/*.jsonl` |
| **Project RAG config** | **`{projectPath}/.rag/config.json`** |
| **Knowledge base storage** | `{projectPath}/.rag/` |
| sled database | `{projectPath}/.rag/db/` |
| HNSW index | `{projectPath}/.rag/hnsw.bin` |
| Hook scripts | `~/.claude/hooks/session-start.sh` |
| Daemon PID | `~/.claude/rag/daemon.pid` |
| Zhipu API | `https://open.bigmodel.cn/api/paas/v4/embeddings` |

---

## Time-Aware Retrieval with Git History ⭐

### Problem Definition

During project development, features may be modified multiple times, leading to contradictory conclusions in Claude session history. RAG retrieval needs to:

1. **Prioritize current code/docs**: Show what's actually implemented now
2. **Provide change reasons**: Explain WHY changes happened via Git commit messages
3. **Build complete timeline**: Trace feature evolution over time

### Core Design

#### Git History as First-Class Citizen

Git commit history is the authoritative source for understanding "why changes happened":

```
┌─────────────────────────────────────────────────────────────┐
│                     Git Commit History                       │
├─────────────────────────────────────────────────────────────┤
│  Commit 1: "feat: add OAuth2 login"                         │
│  ├─ Files: src/auth/login.rs (新增)                         │
│  ├─ Diff: +50 lines (实现 OAuth2)                           │
│  └─ Date: 2025-01-25                                        │
│                                                             │
│  Commit 2: "fix: resolve token leak issue"                  │
│  ├─ Files: src/auth/login.rs (修改)                         │
│  ├─ Diff: -5 +10 lines (修复 token 泄漏)                    │
│  └─ Date: 2025-01-20                                        │
│                                                             │
│  Commit 3: "refactor: migrate from JWT to OAuth2"           │
│  ├─ Files: src/auth/login.rs (重构)                         │
│  ├─ Diff: -30 +50 lines (从 JWT 迁移到 OAuth2)              │
│  ├─ Message: "JWT 存在安全风险，改用 OAuth2..."             │
│  └─ Date: 2025-01-15                                        │
└─────────────────────────────────────────────────────────────┘
```

#### Confidence Level System

```rust
enum ConfidenceLevel {
    Highest = 5,  // Current code/docs (matches latest Git state)
    High = 4,     // Git commit/diff (authoritative change records)
    Medium = 3,   // Recent session (≤7 days)
    Low = 2,      // Old session (7-30 days)
    Lowest = 1,   // Stale discussion (>30 days)
}

// Confidence-aware scoring
final_score = semantic_similarity * temporal_weight * confidence_weight
```

#### Enhanced Retrieval Results

```rust
struct EnhancedItem {
    // Base info
    pub id: String,
    pub content_type: ContentType,
    pub content: String,

    // Scoring
    pub similarity: f32,
    pub temporal_weight: f32,
    pub final_score: f32,

    // Temporal info
    pub timestamp: i64,
    pub age_description: String,
    pub confidence_level: ConfidenceLevel,

    // ⭐ Git-specific fields
    pub git_info: Option<GitInfo>,

    // Status
    pub is_current: bool,
    pub is_deprecated: bool,
    pub superseded_by: Option<SupersededInfo>,
}

struct GitInfo {
    pub commit_hash: String,
    pub commit_message: String,
    pub commit_date: i64,
    pub author: String,
    pub file_changes: Vec<FileChange>,
}
```

#### Timeline Building

```rust
struct FeatureTimeline {
    pub feature_name: String,
    pub timeline: Vec<TimelineEvent>,  // Chronological events
    pub current_state: CurrentState,   // What's current
    pub related_files: Vec<String>,
}

enum TimelineEventType {
    GitChange,      // Commit/diff changes
    Discussion,     // Session discussions
    Implementation, // Current code
}
```

### Indexing with Git History

```rust
impl Indexer {
    async fn index_project(&self, project_path: &str) -> Result<IndexStats> {
        // 1. Index source files
        stats += self.index_source_files(project_path).await?;

        // 2. Index docs
        stats += self.index_docs(project_path).await?;

        // 3. ⭐ Index Git history
        if let Ok(git_collector) = GitCollector::new(project_path) {
            stats += self.index_git_history(&git_collector).await?;
        }

        // 4. Index sessions
        stats += self.index_sessions(project_path).await?;

        Ok(stats)
    }
}
```

### Query Output Example

```markdown
# 登录认证功能

## 📌 当前状态

**当前实现**: 代码库中的最新版本

- `src/auth/login.rs`
- `src/auth/oauth2.rs`

最新 commit: `a1b2c3d4`

---

## 📜 变更时间线

### 🟢 2025-01-25 (2小时前)

**[Git 变更]** 修改 src/auth/login.rs: fix: resolve token leak issue

修复了 OAuth2 token 泄漏问题，在用户登出时正确清理 token。

---

### 🔵 2025-01-20 (5天前)

**[Git 变更]** 修改 src/auth/login.rs: feat: add OAuth2 login

实现 OAuth2 认证流程，支持 GitHub 和 Google 登录。

---

### 🟡 2025-01-15 (10天前)

**[讨论]** 讨论是否迁移到 OAuth2

> User: JWT token 存在什么安全问题？
>
> AI: JWT token 如果不正确处理，可能导致...
>
> User: 那我们应该用什么方案？
>
> AI: 建议使用 OAuth2，因为...
```

### Implementation Phases

#### Phase 1: Git Collection
- `src/git_collector.rs` - Git history collection
- `src/models.rs` - Add Commit/GitDiff structures
- `Cargo.toml` - Add git2 dependency

#### Phase 2: Indexing & Storage
- `src/indexer.rs` - Integrate Git indexing
- `src/storage.rs` - Add Git data storage

#### Phase 3: Time-Aware Retrieval
- `src/confidence.rs` - Confidence calculation
- `src/time_decay.rs` - Temporal decay
- `src/hnsw.rs` - Integrate temporal weighting

#### Phase 4: Timeline & Formatting
- `src/timeline.rs` - Timeline building
- `src/results.rs` - Enhanced result structures
- `src/formatter.rs` - Result formatting
- `src/mcp.rs` - Extend MCP interface

---

## Git State Synchronization Architecture ⭐

### Overview

Git state synchronization provides accurate tracking of whether indexed content matches the current Git HEAD, enabling proper `is_current` and `is_deprecated` marking in query results.

### Two-Tier Caching System

```
┌─────────────────────────────────────────────────────────────────────────┐
│                        GitSync Cache Architecture                       │
├─────────────────────────────────────────────────────────────────────────┤
│                                                                          │
│  ┌─────────────────────────────────────────────────────────────────┐    │
│  │  L1: Memory Cache (LRU)                                         │    │
│  │  ├─ Fast access to hot data                                    │    │
│  │  ├─ Default: 1000 entries                                      │    │
│  │  └─ In-memory only, lost on restart                            │    │
│  └─────────────────────────────────────────────────────────────────┘    │
│                              │ Promote on L1 miss                      │
│                              ▼                                         │
│  ┌─────────────────────────────────────────────────────────────────┐    │
│  │  L2: Persistent Cache (Disk)                                    │    │
│  │  ├─ Survives program restarts                                  │    │
│  │  ├─ Default: 10000 entries                                     │    │
│  │  ├─ Stored: .rag/git_sync_cache.json                           │    │
│  │  └─ Atomic write (temp file + rename)                          │    │
│  └─────────────────────────────────────────────────────────────────┘    │
│                                                                          │
│  ┌─────────────────────────────────────────────────────────────────┐    │
│  │  Background Persistence Task                                    │    │
│  │  ├─ Runs every 5 minutes                                       │    │
│  │  ├─ Only writes if dirty flag is set                           │    │
│  │  └─ Cleaned up on Drop                                         │    │
│  └─────────────────────────────────────────────────────────────────┘    │
│                                                                          │
└─────────────────────────────────────────────────────────────────────────┘
```

### Cache Entry Structure

```rust
struct GitStatusEntry {
    /// HEAD commit hash when cached
    head_hash: String,
    /// Whether the file is current (matches Git HEAD)
    is_current: bool,
    /// When the cache entry was created
    cached_at: DateTime<Utc>,
    /// File content hash (SHA-256) for detecting modifications
    file_hash: Option<String>,
    /// Original deprecation reason (only set for Deprecated status)
    reason: Option<String>,
}
```

### Persistent Cache Format

```rust
struct GitSyncCache {
    /// Cache version for future migrations
    version: u32,
    /// HEAD commit hash when cache was created
    head_hash: String,
    /// Cache entries (file_path -> entry)
    entries: HashMap<String, GitStatusEntry>,
    /// Maximum number of entries to keep
    max_entries: usize,
    /// When cache was last updated
    updated_at: DateTime<Utc>,
}
```

### Cache Invalidation Strategies

| Strategy | Trigger | Behavior |
|----------|---------|----------|
| **HEAD Change** | Git commit detected | All entries invalidated (head_hash mismatch) |
| **File Modified** | Content hash mismatch | Re-check Git status |
| **TTL Expired** | Entry age > cache_ttl_seconds | Re-check Git status |
| **Manual** | `clear_cache()` or `invalidate_file()` | Explicit invalidation |

### File Hash Optimization

Files are hashed using SHA-256 to detect content modifications without re-querying Git:

```rust
fn compute_file_hash(repo: &Repository, file_path: &str) -> Option<String> {
    // Skip files larger than 10 MB to avoid blocking
    if metadata.len() > MAX_HASH_SIZE {
        return None;
    }

    // Compute SHA-256 hash
    let mut hasher = Sha256::new();
    // Read in chunks and update...
    Some(format!("{:x}", hasher.finalize()))
}
```

**Benefits:**
- Cached `Current` status can be returned if file hash matches
- Avoids expensive `git status` calls for unchanged files
- Safe fallback to Git check if hash computation fails

### Background Persistence Flow

```
┌─────────────────────────────────────────────────────────────┐
│  Background Persistence Task (tokio::spawn)                │
├─────────────────────────────────────────────────────────────┤
│                                                             │
│  1. Spawn on GitSync creation (if tokio runtime available) │
│     ↓                                                       │
│  2. Run every 5 minutes (tokio::interval)                   │
│     ↓                                                       │
│  3. Check dirty flag (AtomicBool)                           │
│     ├─ If NOT dirty → skip                                 │
│     └─ If dirty → continue                                 │
│     ↓                                                       │
│  4. Serialize cache to JSON                                 │
│     ↓                                                       │
│  5. Write to temp file (.rag/git_sync_cache.json.tmp)       │
│     ↓                                                       │
│  6. Atomic rename to final location                         │
│     ↓                                                       │
│  7. Clear dirty flag                                        │
│     ↓                                                       │
│  8. Repeat (go to step 2)                                   │
│                                                             │
└─────────────────────────────────────────────────────────────┘
```

### Error Handling & Graceful Degradation

| Error Scenario | Behavior |
|----------------|----------|
| **Cache file not found** | Start with empty cache (normal first run) |
| **Cache file corrupted** | Log warning, start with empty cache |
| **Unsupported version** | Log warning, start with empty cache |
| **Persistence write fails** | Log warning, continue with memory-only mode |
| **Persistence disabled** | Operate in memory-only mode (no disk I/O) |

### Public API

```rust
impl GitSync {
    /// Create with default settings
    pub fn new(project_path: &Path, cache_ttl_seconds: u64) -> Result<Self>;

    /// Create with custom capacity
    pub fn with_capacity(project_path: &Path, cache_ttl_seconds: u64, cache_capacity: usize) -> Result<Self>;

    /// Create with full options (including persistence toggle)
    pub fn with_options(project_path: &Path, cache_ttl_seconds: u64, cache_capacity: usize, enable_persistence: bool) -> Result<Self>;

    /// Check single file sync status
    pub async fn check_file_sync(&self, file_path: &str) -> Result<GitSyncStatus>;

    /// Batch check multiple files (more efficient)
    pub async fn batch_check_files(&self, file_paths: &[String]) -> Result<HashMap<String, GitSyncStatus>>;

    /// Check symbol sync status (inherits from file)
    pub async fn check_symbol_sync(&self, file_path: &str) -> Result<GitSyncStatus>;

    /// Manual cache flush
    pub async fn flush(&self) -> Result<()>;

    /// Clear all cached entries
    pub async fn clear_cache(&self);

    /// Invalidate specific file
    pub async fn invalidate_file(&self, file_path: &str);
}
```

### GitSyncStatus Enum

```rust
pub enum GitSyncStatus {
    /// Content matches Git HEAD (current)
    Current,
    /// Content differs from Git HEAD (deprecated)
    Deprecated {
        reason: String,  // Human-readable description
    },
    /// Not applicable (non-Git repo or untracked file)
    NotApplicable,
}
```

### Testing Coverage

The implementation includes comprehensive tests:
- `test_persistent_cache_save` - Verifies cache file creation
- `test_persistent_cache_load_on_restart` - Simulates program restart
- `test_cache_invalidation_on_head_change` - HEAD change detection
- `test_persistence_failure_graceful_degradation` - Persistence disabled
- `test_persistence_full_workflow` - Complete workflow simulation
- `test_corrupted_cache_file` - Handles corrupted JSON
- `test_unsupported_cache_version` - Handles version mismatches

### Configuration Options

| Option | Default | Description |
|--------|---------|-------------|
| `cache_ttl_seconds` | 60 | How long cache entries remain valid |
| `memory_cache_capacity` | 1000 | L1 cache size (LRU) |
| `persistent_cache_max_entries` | 10000 | L2 cache size |
| `enable_persistence` | true | Whether to enable disk persistence |
| `persist_interval` | 300 seconds | Background persistence frequency |

### Integration Points

The GitSync module integrates with:
- **Confidence Engine**: Uses `GitSyncStatus::is_current()` for confidence scoring
- **Query Processing**: Filters results based on current/deprecated status
- **Indexing**: Updates cache when new files are indexed
- **Daemon Operations**: Persists cache during incremental indexing

---

## ⭐ Code Symbol Indexing with Branch Awareness

### Overview

Code symbol indexing provides **function/class/method-level semantic search** with Git branch isolation:

- **3-tier chunking strategy**: Symbol → Block → File levels
- **Branch-aware storage**: Separate symbol indices per Git branch
- **Semantic search**: Find functions by meaning, not just keywords
- **MCP granularity**: Support `symbol`, `file`, and `mixed` search modes

### Architecture

\`\`\`
┌─────────────────────────────────────────────────────────────────────────┐
│                    Code Symbol Indexing Pipeline                     │
├─────────────────────────────────────────────────────────────────────────┤
│                                                                          │
│  ┌─────────────┐    ┌─────────────┐    ┌─────────────┐                 │
│  │   File       │    │   Ast       │    │  Code       │                 │
│  │   Scanner    │───▶│   Parser    │───▶│  Chunker    │                 │
│  └─────────────┘    └─────────────┘    └─────────────┘                 │
│         │                  │                  │                         │
│         ▼                  ▼                  ▼                         │
│  ┌─────────────┐    ┌─────────────┐    ┌─────────────┐                 │
│  │  Branch      │    │   Symbol     │    │   Symbol    │                 │
│  │  Manager     │    │   Cache      │    │   Indexer   │                 │
│  └─────────────┘    └─────────────┘    └─────────────┘                 │
│         │                  │                  │                         │
│         ▼                  ▼                  ▼                         │
│  ┌─────────────────────────────────────────────────────────────────┐    │
│  │                    Storage Layer                               │    │
│  └─────────────────────────────────────────────────────────────────┘    │
│                                                                          │
└─────────────────────────────────────────────────────────────────────────┘
\`\`\`

### 3-Tier Chunking Strategy

| Level | Threshold | Description | Example |
|-------|-----------|-------------|---------|
| **Symbol** | < 500 lines | Individual function/class | \`fn authenticate_user()\` |
| **Block** | > 500 lines | Large symbol split into 300-line chunks | \`impl Auth (lines 1-300)\` |
| **File** | Fallback | File summary when AST parsing fails | \`src/auth.rs overview\` |

### Branch-Aware Storage

**Storage Keys:**
\`\`\`
# Individual symbol (branch-aware)
symbol:{branch}:{symbol_id}
Example: symbol:main:symbol:abc123

# Branch symbol index
branch_symbols:{branch}
Example: branch_symbols:main
\`\`\`

### CLI Integration

**New Command:**
\`\`\`bash
claude-rag index-code --branch main    # Index current branch
claude-rag index-code --all              # Index all branches
\`\`\`

### MCP Granularity Parameter

| Mode | Description | Use Case |
|------|-------------|----------|
| \`symbol\` | Search function/class symbols | Find specific implementations |
| \`file\` | Search file-level content | Find relevant files |
| \`mixed\` | Return both symbols and files | Comprehensive search |

### Implementation Status

| Component | Status | Tests |
|-----------|--------|-------|
| CodeChunker | ✅ Complete | 12/12 |
| BranchManager | ✅ Complete | 20/20 |
| Symbol Cache | ✅ Complete | 8/8 |
| Storage Extensions | ✅ Complete | 38/38 |
| Indexer Extensions | ✅ Complete | 22/22 |
| MCP Granularity | ✅ Complete | 6/6 |
| CLI Integration | ✅ Complete | All passing |

**Total: 649 tests passing**

