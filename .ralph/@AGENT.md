# Ralph Agent Configuration - Claude RAG

## Build Instructions

```bash
# Debug build with full checks
cargo build

# With extra compiler output
cargo build --verbose

# Check without building
cargo check

# Release build (optimized)
cargo build --release
```

## Test Instructions

```bash
# Run all tests
cargo test

# Run with output
cargo test -- --nocapture

# Run specific test
cargo test test_confidence_level

# Run tests in parallel (faster)
cargo nextest run

# Check coverage (must be ≥85%) ⭐
cargo tarpaulin --out Html --threshold 85

# View coverage report
open coverage/index.html
```

## Run Instructions

```bash
# Run main binary
cargo run -- --help

# Run with arguments
cargo run -- query "test query"

# Run with specific features
cargo run --features "git-integration" -- index
```

## Development Workflow

```bash
# Auto-rebuild on file changes
cargo watch -x build

# Auto-run tests on changes
cargo watch -x test

# Auto-check with clippy
cargo watch -x "clippy --all-targets --all-features"

# Format code
cargo fmt

# Run linter
cargo clippy --all-targets --all-features

# Run audit
cargo audit
```

## Pre-Commit Checklist

Before committing changes, ensure:

```bash
# 1. Format code
cargo fmt

# 2. Run linter
cargo clippy --all-targets --all-features

# 3. Run tests
cargo test --all-features

# 4. Check coverage (must be ≥85%) ⭐
cargo tarpaulin --threshold 85

# 5. Check documentation
cargo doc --no-deps --open
```

## Environment Setup

### Prerequisites

```bash
# Rust toolchain (2024 edition)
rustc --version      # 1.82+
cargo --version      # 1.82+

# Git
git --version        # 2.0+

# Optional: development tools
rustup component add rustfmt
rustup component add clippy
rustup component add rust-analyzer
```

### Optional Development Tools

| Tool | Purpose | Install |
|------|---------|---------|
| `cargo-watch` | Auto-rebuild | `cargo install cargo-watch` |
| `cargo-nextest` | Faster tests | `cargo install cargo-nextest` |
| `cargo-audit` | Security audit | `cargo install cargo-audit` |
| `cargo-tarpaulin` | Coverage | `cargo install cargo-tarpaulin` |

### Configuration

```bash
# Create global config directory
mkdir -p ~/.claude/rag

# Create config file (example)
cat > ~/.claude/rag/config.toml << 'EOF'
# Zhipu AI Embedding API Configuration
[embedding]
api_token = "your-zhipu-api-token"
api_url = "https://open.bigmodel.cn/api/paas/v4/embeddings"
dimensions = 1024
batch_size = 8
timeout_ms = 30000

# HNSW Index Parameters
[hnsw]
m = 16
ef_construction = 200
ef_search = 50

# Indexing Options
[index]
index_source = true
index_docs = true
index_other = false

# Daemon Options
[daemon]
session_timeout_seconds = 60
persist_interval_seconds = 300

# Git Integration ⭐
[git]
enable_git_indexing = true
max_commits_to_index = 1000
include_diff_content = true
conventional_commits = true
extract_breaking_changes = true

# Confidence/Temporal Weights ⭐
[confidence]
code_weight = 1.2
git_commit_weight = 1.0
session_weight = 0.7
EOF
```

## CLI Commands Reference

```bash
# Initialize knowledge base in current project
claude-rag init

# Index existing sessions and files
claude-rag index --all                    # All projects
claude-rag index --project /path/to/project  # Specific project
claude-rag index --type sessions          # Only sessions
claude-rag index --type source            # Only source files
claude-rag index --type git               # Only Git history ⭐

# Start daemon service
claude-rag daemon start
claude-rag daemon status
claude-rag daemon stop
claude-rag daemon restart

# Query knowledge base
claude-rag query "How to handle this error?"
claude-rag query --type code "login function"
claude-rag query --timeline "OAuth2 migration"  # ⭐ Timeline query
claude-rag query --file src/auth.rs --show-diffs  # ⭐ File history

# Check status
claude-rag status

# Install Claude Code integration
claude-rag install-skills    # Install Skills
claude-rag setup-mcp         # Configure MCP server

# Start MCP server
claude-rag mcp-server
```

## Notes

- Update this file when build process changes
- Add environment setup instructions as needed
- Include any pre-requisites or dependencies
- **Coverage Requirement**: All code must maintain 85% test coverage
