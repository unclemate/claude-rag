# Claude RAG

> Claude Code Interaction History & Knowledge Base Tool

[![Rust](https://img.shields.io/badge/rust-2024-edition.svg)](https://www.rust-lang.org)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

## Overview

Claude RAG is a Rust tool that builds a **complete time-aware RAG knowledge base** for your projects. It captures:

- **Session Records**: Claude Code interaction history (user input + AI output)
- **Source Files**: Project code with hierarchical indexing (file-level + function/class-level)
- **Code Symbols**: Function/class/method-level indexing with branch awareness ⭐
- **Documentation**: README, design docs, comments, etc.
- **Other Files**: Configs, test cases, etc.
- **Git History**: Commit history and diffs with temporal context ⭐

### Key Features

**🕐 Time-Aware Retrieval**: Prioritizes current code over historical discussions with temporal confidence scoring

**📜 Change Timeline**: Track feature evolution through Git commits and session history

**🎯 Smart Context**: Automatically identifies what's current vs. deprecated

**⭐ Symbol-Level Search**: Find specific functions, classes, and methods with semantic understanding

**🔀 Branch-Aware Indexing**: Separate symbol indices for different Git branches

Data is stored locally in each project's `.rag/` directory, ensuring privacy and enabling semantic retrieval of project context.

## Tech Stack

| Component | Selection |
|-----------|-----------|
| Language | Rust 2024 Edition |
| Vector Database | **sled (embedded KV)** + **HNSW algorithm** (self-implemented) |
| Embedding Model | Zhipu AI embedding-3 API |
| Data Source | Claude Code session JSONL files |
| Hook System | session-start + daemon file monitoring |
| Git Integration | **git2** for commit/diff history |

## Architecture

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
│  │  ┌─────────────────────────────┐    │    │   File Watcher   │      │
│  │  │  HNSW (vector index)        │    │    │   Incremental    │      │
│  │  │  - Unified index all content│    │    │   Indexer        │      │
│  │  │  - Filter by type query     │    │    └──────────────────┘      │
│  │  └─────────────────────────────┘    │                                │
│  └─────────────────────────────────────┘                                │
│                                                                          │
└─────────────────────────────────────────────────────────────────────────┘
```

### Data Sources

| Source | Parse Method | Index Granularity |
|--------|-------------|-------------------|
| **Session JSONL** | Parser Module | Per message |
| **Source Files** | FileScanner + AST | File-level + paragraph-level |
| **Code Symbols** ⭐ | CodeChunker + AST | Symbol/Block/File-level (3-tier chunking) |
| **Doc Files** | FileScanner | File-level + paragraph-level |
| **Other Files** | FileScanner | File-level |
| **Git History** | GitCollector (git2) | Per commit + per file diff (optional, requires Git repo) |

### Storage Design

Each project has its own `.rag/` directory:

```
{projectPath}/.rag/
├── config.json          # Project-level config (optional)
├── db/                  # sled database files
│   ├── sled-data
│   └── ...
└── hnsw.bin             # HNSW index snapshot
```

**Data Isolation**: Each project maintains its own knowledge base, ensuring complete separation of context.

## Quick Start

### 1. Installation

```bash
git clone https://github.com/yourusername/claude-rag.git
cd claude-rag
cargo install --path .
```

### 2. Start Background Monitoring

```bash
# Start background monitoring service
claude-rag daemon start

# Check service status
claude-rag daemon status

# Stop service
claude-rag daemon stop
```

### 3. Index Existing Sessions

```bash
# Index historical sessions for all projects
claude-rag index --all

# Index specific project
claude-rag index --project /path/to/your/project

# Output example:
# 🔍 Scanning session files...
# 📊 Found 15 projects, 127 sessions
# ⏳ Indexing... [████████████████████] 100%
# ✓ Indexed 127 sessions, 1,523 messages
```

### 4. Install Claude Code Integration

```bash
# Install Skills (manual query commands)
claude-rag install-skills

# Output example:
# ✓ Installed Skills to ~/.claude/skills/
#   - rag-query: query all content
#   - rag-code: search source
#   - rag-docs: search docs
#   - rag-session: search session history

# Configure MCP Server (auto query)
claude-rag setup-mcp
```

## Usage

### Method 1: Claude Code Skills (Manual)

Enter directly in Claude Code conversation:

```
# Query all content (sessions+source+docs)
/rag-query How was login handled in this project before?

# Search source only
/rag-code How to validate user permissions?

# ⭐ Search code symbols (function/class level)
/rag-code How is the authentication token validated?

# Search docs only
/rag-docs What is the deployment process?

# Search session history only
/rag-session Did we discuss database migration before?
```

### Method 2: MCP Server (Auto)

Once configured, Claude automatically determines when to query RAG:

```
User: How is the login function implemented in this project?

Claude: [Auto-calls MCP tool to query project knowledge base]
       Based on project code and docs, the login function is implemented as follows...
```

### Method 3: CLI Commands

```bash
# Natural language query
claude-rag query "How to handle this error?"

# Query by type
claude-rag query --type code "How to handle this error?"
claude-rag query --type docs "Deployment steps"
claude-rag query --type session "Previous discussions"

# ⭐ Timeline query - see feature evolution
claude-rag query --timeline "login authentication"

# ⭐ Git-specific queries
claude-rag query --type commit "OAuth2 migration"
claude-rag query --file "src/auth/login.rs" --show-diffs

# ⭐ Time range filtering - filter results by time
claude-rag query "database" --max-age 7          # Last 7 days
claude-rag query "auth" --after "2025-01-01"     # Since a date
claude-rag query "bug" --after "1w" --before "7d" # Time range

# ⭐ Code symbol indexing
claude-rag index-code --branch main            # Index current branch symbols
claude-rag index-code --all                     # Index all branches
claude-rag index-code --project /path/to/project --branch feature/api

# Output example:
# 🎯 Found 3 relevant contexts
#
# [1] Similarity: 0.92  Type: Source
# File: src/auth/login.rs:10-25
# ──────────────────────────────────────────────────────
# fn handle_login() -> Result<()> {
#     // Handle login logic...
# }
#
# [2] Similarity: 0.87  Type: Session
# Session: Backend Config Cleanup (2025-01-23)
# ──────────────────────────────────────────────────────
# We discussed login config cleanup before...
```

## ⭐ Code Symbol Indexing

### Overview

Claude RAG now supports **symbol-level code indexing** with branch awareness:

- **3-tier chunking**: Symbol → Block → File levels
- **Branch isolation**: Separate indices per Git branch
- **Semantic search**: Find functions by meaning, not just keywords

### Indexing Granularity

| Level | Threshold | Description |
|-------|-----------|-------------|
| **Symbol** | < 500 lines | Individual functions, classes, methods |
| **Block** | > 500 lines | Large symbols split into 300-line chunks with 10-line overlap |
| **File** | Summary level | File overview when symbol extraction fails |

### Branch-Aware Storage

Symbols are stored with branch context:

```
{projectPath}/.rag/db/
├── symbol:main:symbol:abc123    # Symbol on main branch
├── symbol:feature/api:symbol:abc123  # Same symbol on feature branch
└── ...
```

### Usage Examples

**Index code symbols:**
```bash
# Index current branch
claude-rag index-code

# Index specific branch
claude-rag index-code --branch feature/api

# Index all branches
claude-rag index-code --all
```

**Search with granularity (MCP):**
```json
{
  "name": "rag_search_code",
  "arguments": {
    "query": "用户认证逻辑",
    "granularity": "symbol",  // or "file" or "mixed"
    "top_k": 10
  }
}
```

**Search results:**
```
1. handle_auth - 认证处理函数 (similarity: 0.92)
2. verify_token - Token验证 (similarity: 0.88)
3. TokenManager - Token管理器 (similarity: 0.85)
```

## How It Works

```
┌────────────────────────────────────────────────────────────┐
│  1. Claude Code session starts                             │
│     ↓                                                       │
│  2. session-start hook notifies daemon                     │
│     ↓                                                       │
│  3. Daemon monitors JSONL file changes                     │
│     ↓ (incremental processing, new messages only)          │
│  4. Call Zhipu embedding-3 API to generate vectors         │
│     ↓                                                       │
│  5. Store in project directory .rag/                       │
│     ├─ db/ (sled KV database)                             │
│     └─ hnsw.bin (vector index)                            │
│     ↓                                                       │
│  6. Support semantic retrieval of project context          │
└────────────────────────────────────────────────────────────┘

Data isolation:
├── /home/user/project1/.rag/  ← Project 1 knowledge base
├── /home/user/project2/.rag/  ← Project 2 knowledge base
└── ...
```

## Configuration

### Global Config

The program reads global configuration from `~/.claude/rag/config.toml`:

```toml
# Zhipu AI Embedding API Configuration
[embedding]
api_token = "your-zhipu-api-token"           # Required: Zhipu AI API token
api_url = "https://open.bigmodel.cn/api/paas/v4/embeddings"  # Optional
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

# ⭐ Git Integration
[git]
enable_git_indexing = true          # Index Git history
max_commits_to_index = 1000         # Max commits to index
include_diff_content = true         # Include diff content
conventional_commits = true         # Parse Conventional Commits
extract_breaking_changes = true     # Extract BREAKING CHANGE

# ⭐ Confidence/Temporal Weights
[confidence]
code_weight = 1.2                   # Current code weight
git_commit_weight = 1.0             # Git commit weight
session_weight = 0.7                # Session weight (lowered)

[retrieval]
enable_timeline = true              # Enable timeline building
show_git_context = true             # Show Git context
```

**Get API Token**: Visit [Zhipu AI Open Platform](https://open.bigmodel.cn/) to obtain your API token.

### Project Config (Optional)

Override global settings per project in `{projectPath}/.rag/config.json`:

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

## Advanced Usage

```bash
# Check knowledge base status
claude-rag status

# Export knowledge base
claude-rag export --output backup.json

# Clear knowledge base
claude-rag reset --confirm

# View logs
claude-rag logs --follow
```

## Project Status

🚧 **Under Development** - Project is in early development stage

See [DESIGN.md](DESIGN.md) for detailed implementation plans.

## ⭐ Time-Aware Retrieval

This project implements **temporal confidence scoring** to prioritize current information:

| Confidence | Description | Weight |
|------------|-------------|--------|
| 🟢 Highest | Current code (matches Git HEAD) | 1.2x |
| 🔵 High | Git commits/diffs (authoritative) | 1.0x |
| 🟡 Medium | Recent sessions (≤7 days) | 0.85x |
| 🟠 Low | Old sessions (7-30 days) | 0.6x |
| 🔴 Lowest | Stale discussions (>30 days) | 0.4x |

### Timeline Query Example

```bash
claude-rag query --timeline "登录认证"
```

Output shows feature evolution:
- 📌 **Current State**: What's implemented now
- 📜 **Timeline**: Git changes + discussions in chronological order
- 🎯 **Confidence**: Color-coded by temporal relevance

**Example Output:**
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

---

### 🟠 2025-01-10 (15天前)

**[Git 变更]** 修改 src/auth/login.rs: feat: implement JWT authentication

初始实现 JWT token 认证。
```

## Contributing

Issues and Pull Requests are welcome!

## License

MIT License
