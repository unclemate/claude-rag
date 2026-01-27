# Claude RAG - Development Guide

> Code standards, Rust best practices, and build instructions

## Table of Contents

- [Development Environment](#development-environment)
- [Project Setup](#project-setup)
- [Code Standards](#code-standards)
- [Rust Best Practices](#rust-best-practices)
- [Build & Run](#build--run)
- [Testing](#testing)
- [Git Workflow](#git-workflow)
- [Code Review](#code-review)

---

## Development Environment

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

### Recommended Tools

| Tool | Purpose | Install |
|------|---------|---------|
| `cargo-watch` | Auto-rebuild on file changes | `cargo install cargo-watch` |
| `cargo-nextest` | Faster test runner | `cargo install cargo-nextest` |
| `cargo-audit` | Security audit | `cargo install cargo-audit` |
| `cargo-outdated` | Dependency updates | `cargo install cargo-outdated` |
| `hyperfine` | Benchmarking | `cargo install hyperfine` |

---

## Project Setup

### 1. Clone Repository

```bash
git clone https://github.com/yourusername/claude-rag.git
cd claude-rag
```

### 2. Install Dependencies

```bash
# Install with dev dependencies
cargo install --path .

# Or just build
cargo build
```

### 3. Configuration

```bash
# Create global config directory
mkdir -p ~/.claude/rag

# Copy example config
cp config.example.toml ~/.claude/rag/config.toml

# Edit with your API token
nano ~/.claude/rag/config.toml
```

### 4. Verify Installation

```bash
cargo --version
claude-rag --help
```

---

## Code Standards

### File Organization

```
src/
├── main.rs              # CLI entry point
├── lib.rs               # Library exports (if applicable)
├── config.rs            # Configuration management
├── models/              # Data models
│   ├── mod.rs
│   ├── session.rs
│   ├── message.rs
│   ├── commit.rs        # ⭐ Git models
│   └── diff.rs          # ⭐ Git diff models
├── storage/             # Storage layer
│   ├── mod.rs
│   ├── sled.rs
│   └── hnsw.rs
├── collector/           # Data collectors
│   ├── mod.rs
│   ├── session.rs
│   ├── file.rs
│   └── git.rs           # ⭐ Git collector
├── retrieval/           # ⭐ Time-aware retrieval
│   ├── mod.rs
│   ├── confidence.rs
│   ├── decay.rs
│   └── timeline.rs
├── embedding.rs
├── parser.rs
├── scanner.rs
├── hook.rs
├── daemon.rs
├── skills.rs
├── mcp.rs
└── error.rs             # Centralized error types
```

### Naming Conventions

| Category | Convention | Example |
|----------|------------|---------|
| **Modules** | `snake_case` | `mod git_collector` |
| **Types** | `PascalCase` | `struct GitCollector` |
| **Functions** | `snake_case` | `fn collect_commits()` |
| **Constants** | `SCREAMING_SNAKE_CASE` | `const MAX_RETRIES: u32` |
| **Static** | `SCREAMING_SNAKE_CASE` | `static DEFAULT_CONFIG: &str` |
| **Traits** | `PascalCase` | `trait VectorIndex` |
| **Lifetime** | Short, lowercase | `'a`, `'session` |

### Module Structure

```rust
// src/git_collector/mod.rs
mod error;
mod commit;
mod diff;

pub use commit::Commit;
pub use diff::GitDiff;
pub use error::{GitError, Result};

use std::path::PathBuf;

/// Git history collector using git2.
///
/// Collects commit history and file diffs from Git repositories
/// for time-aware RAG indexing.
pub struct GitCollector {
    repo: git2::Repository,
    project_path: PathBuf,
}

impl GitCollector {
    /// Create a new GitCollector for the given project path.
    ///
    /// # Errors
    ///
    /// Returns an error if the path is not a valid Git repository.
    pub fn new(project_path: &str) -> Result<Self> {
        // Implementation...
    }
}
```

---

## Rust Best Practices

### 1. Error Handling

#### Use `thiserror` for Error Types

```rust
// src/error.rs
use thiserror::Error;

#[derive(Error, Debug)]
pub enum RagError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Sled database error: {0}")]
    Sled(#[from] sled::Error),

    #[error("Git operation failed: {0}")]
    Git(String),

    #[error("Embedding API error: {0}")]
    Embedding(String),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Item not found: {0}")]
    NotFound(String),
}

pub type Result<T> = std::result::Result<T, RagError>;
```

#### Use `anyhow` for Application Errors

```rust
use anyhow::{Context, Result};

async fn index_project(path: &Path) -> Result<usize> {
    let files = scan_directory(path)
        .context("Failed to scan project directory")?;

    let count = process_files(files)
        .await
        .context("Failed to process files")?;

    Ok(count)
}
```

### 2. Use Builder Pattern for Complex Types

```rust
pub struct HnswIndex {
    m: usize,
    ef_construction: usize,
    ef_search: usize,
    // ...
}

impl HnswIndex {
    pub fn builder() -> HnswBuilder {
        HnswBuilder::default()
    }
}

pub struct HnswBuilder {
    m: usize,
    ef_construction: usize,
    ef_search: usize,
}

impl Default for HnswBuilder {
    fn default() -> Self {
        Self {
            m: 16,
            ef_construction: 200,
            ef_search: 50,
        }
    }
}

impl HnswBuilder {
    pub fn m(mut self, m: usize) -> Self {
        self.m = m;
        self
    }

    pub fn ef_construction(mut self, ef: usize) -> Self {
        self.ef_construction = ef;
        self
    }

    pub fn build(self) -> HnswIndex {
        HnswIndex {
            m: self.m,
            ef_construction: self.ef_construction,
            ef_search: self.ef_search,
        }
    }
}
```

### 3. Trait Design

```rust
/// Vector index trait for semantic search.
pub trait VectorIndex: Send + Sync {
    /// Insert a vector with associated ID.
    fn insert(&mut self, vector: Vec<f32>, id: String) -> Result<()>;

    /// Search for nearest neighbors.
    fn search(&self, query: &[f32], k: usize) -> Result<Vec<SearchResult>>;

    /// Persist index to disk.
    fn save(&self, path: &Path) -> Result<()>;

    /// Load index from disk.
    fn load(path: &Path) -> Result<Self>
    where
        Self: Sized;
}

/// Search result with similarity score.
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub id: String,
    pub similarity: f32,
}
```

### 4. Async/Await Patterns

```rust
use tokio::time::{timeout, Duration};

// Use timeout for external API calls
async fn call_embedding_api(text: &str) -> Result<Vec<f32>> {
    let result = timeout(
        Duration::from_secs(30),
        reqwest::Client::new()
            .post(&settings.api_url)
            .json(&serde_json::json!({ "text": text }))
            .send()
    )
    .await
    .context("API request timed out")?
    .context("Failed to send request")?;

    // Parse response...
    Ok(vec)
}

// Use `join!` for concurrent operations
async fn index_concurrently(files: Vec<PathBuf>) -> Result<usize> {
    let (a, b) = tokio::join!(
        index_source_files(&files),
        index_doc_files(&files)
    );

    Ok(a? + b?)
}
```

### 5. Memory Efficiency

```rust
// Use Cow for conditional ownership
use std::borrow::Cow;

fn format_message(msg: &str) -> Cow<str> {
    if msg.contains('\n') {
        Cow::Owned(msg.replace('\n', " "))
    } else {
        Cow::Borrowed(msg)
    }
}

// Use slices instead of cloning
fn process_vectors(vectors: &[Vec<f32>]) -> f32 {
    vectors.iter()
        .map(|v| v.len())
        .sum::<usize>() as f32
}

// Use SmallVec for small collections
use smallvec::{SmallVec, smallvec};

fn add_prefix(mut items: Vec<String>, prefix: &str) -> Vec<String> {
    items.into_iter()
        .map(|s| format!("{}{}", prefix, s))
        .collect()
}
```

### 6. Clippy Lints

```rust
#![deny(clippy::all)]
#![deny(clippy::pedantic)]
#![allow(clippy::must_use_candidate)]  // Too noisy
#![warn(clippy::cargo)]  # Check Cargo.toml

// Specific allowances with reasons
#[allow(clippy::too_many_arguments)]  // Required by trait
pub fn complex_function(a: i32, b: i32, c: i32, d: i32, e: i32) {
    // ...
}
```

---

## Build & Run

### Development Build

```bash
# Debug build with full checks
cargo build

# With extra compiler output
cargo build --verbose

# Check without building
cargo check
```

### Release Build

```bash
# Optimized release build
cargo build --release

# With specific profile
cargo build --profile release-lto
```

### Custom Profiles (Cargo.toml)

```toml
[profile.release-lto]
inherits = "release"
lto = true
codegen-units = 1
opt-level = 3
strip = true
```

### Run Commands

```bash
# Run main binary
cargo run -- --help

# Run with arguments
cargo run -- query "test query"

# Run with specific features
cargo run --features "git-integration" -- index
```

### Development Workflow

```bash
# Auto-rebuild on file changes
cargo watch -x build

# Auto-run tests on changes
cargo watch -x test

# Auto-check with clippy
cargo watch -x "clippy --all-targets --all-features"
```

---

## Testing

### Test Organization

```
tests/
├── integration/
│   ├── mod.rs
│   ├── e2e_tests.rs
│   └── git_integration.rs
└── fixtures/

src/
├── git_collector/
│   ├── mod.rs
│   └── tests.rs        # Unit tests
```

### Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_confidence_level_from_content() {
        let level = ConfidenceLevel::from_content_type(
            &ContentType::File,
            1234567890,
            Some(1234567890),
        );
        assert_eq!(level, ConfidenceLevel::Highest);
    }

    #[test]
    fn test_base_weight() {
        assert_eq!(ConfidenceLevel::Highest.base_weight(), 1.2);
        assert_eq!(ConfidenceLevel::High.base_weight(), 1.0);
    }

    #[tokio::test]
    async fn test_async_function() {
        let result = async_function().await.unwrap();
        assert!(result > 0);
    }
}
```

### Integration Tests

```rust
// tests/integration/git_integration.rs
use claude_rag::GitCollector;
use tempfile::TempDir;

#[tokio::test]
async fn test_git_collector_full_workflow() {
    let temp_dir = TempDir::new().unwrap();
    let repo = setup_test_repo(temp_dir.path());

    let collector = GitCollector::new(temp_dir.path().to_str().unwrap()).unwrap();
    let commits = collector.collect_all_commits().unwrap();

    assert!(!commits.is_empty());
    assert_eq!(commits[0].files_changed.len(), 1);
}
```

### Test Utilities

```rust
// tests/common/mod.rs
use tempfile::TempDir;

pub fn setup_test_repo(dir: &Path) -> git2::Repository {
    git2::Repository::init(dir)
}

pub fn create_test_commit(repo: &git2::Repository, message: &str) -> git2::Oid {
    // ...
}
```

### Running Tests

```bash
# Run all tests
cargo test

# Run with output
cargo test -- --nocapture

# Run specific test
cargo test test_confidence_level

# Run tests in parallel
cargo nextest run

# Run with coverage
cargo install cargo-tarpaulin
cargo tarpaulin --out Html

# Run ignored tests
cargo test -- --ignored
```

### Coverage Requirements

**Minimum Required Coverage: 85%**

All new code must maintain a minimum of 85% test coverage. This ensures code reliability and maintainability.

```bash
# Generate coverage report
cargo tarpaulin --out Html --output-dir coverage

# Check if coverage meets threshold (fails if below 85%)
cargo tarpaulin --out Html --output-dir coverage --threshold 85

# Generate line-by-line coverage
cargo tarpaulin --out Html --output-dir coverage --line

# Generate CI-friendly output
cargo tarpaulin --out Json --output-dir coverage
```

**Coverage Guidelines:**

| Coverage Level | Status | Action |
|---------------|--------|--------|
| ≥ 90% | 🟢 Excellent | No action required |
| 85-89% | 🟢 Acceptable | Meets minimum requirement |
| 70-84% | 🟡 Below Threshold | Additional tests needed |
| < 70% | 🔴 Unacceptable | PR blocked, add tests |

**Coverage Exclusions:**

The following may be excluded from coverage calculation:

```toml
# .tarpaulin.toml or inline
[coverage]
exclude = [
    "src/main.rs",           # Entry point
    "tests/*",               # Test code
    "benches/*",             # Benchmark code
]

# Inline exclusions for specific lines
#[cfg_attr(test, coverage(off))]
```

**Per-Module Coverage Tracking:**

```bash
# Generate coverage per module
cargo tarpaulin --out Html --output-dir coverage -- --test-threads=1

# View coverage report
open coverage/index.html
```

### Benchmarks

```rust
// benches/hnsw_bench.rs
use criterion::{black_box, criterion_group, criterion_main, Criterion, BenchmarkId};

fn bench_hnsw_insert(c: &mut Criterion) {
    let mut group = c.benchmark_group("hnsw_insert");
    for size in [100, 1000, 10000].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            b.iter(|| {
                let mut hnsw = HnswIndex::new();
                for i in 0..size {
                    hnsw.insert(vec![0.0; 1024], format!("id_{}", i));
                }
            });
        });
    }
    group.finish();
}

criterion_group!(benches, bench_hnsw_insert);
criterion_main!(benches);
```

---

## Git Workflow

### Branch Strategy

```
main          ──────────────────────────────────────────>
              ↑           ↑           ↑
               \           \           \
                v           v           v
feature/git   ──●───────────>           (merged)
feature/cli   ──────────────●───────────> (merged)
feature/mcp   ──────────────────────────●──> (in progress)
```

### Branch Naming

| Type | Pattern | Example |
|------|---------|---------|
| Feature | `feature/<name>` | `feature/git-collector` |
| Fix | `fix/<name>` | `fix/memory-leak` |
| Refactor | `refactor/<name>` | `refactor/storage-layer` |
| Docs | `docs/<name>` | `docs/api-reference` |
| Chore | `chore/<name>` | `chore/update-deps` |

### Commit Messages (Conventional Commits)

```
<type>[optional scope]: <description>

[optional body]

[optional footer(s)]
```

**Types:**
- `feat`: New feature
- `fix`: Bug fix
- `docs`: Documentation changes
- `style`: Code style changes (formatting, etc.)
- `refactor`: Code refactoring
- `test`: Adding or updating tests
- `chore`: Maintenance tasks
- `perf`: Performance improvements
- `ci`: CI/CD changes

**Examples:**

```bash
feat(git): add commit history collection

Implement GitCollector using git2 to collect commit metadata
and file diffs for time-aware indexing.

Closes #123
```

```bash
fix(storage): resolve sled transaction deadlock

Use separate transactions for read and write operations
to prevent deadlock under high concurrency.

Fixes #456
```

```bash
refactor(hnsw): extract search logic into trait

Separate search interface from implementation for better
testability and future extensibility.
```

### Commit Checklist

- [ ] Code follows style guide (`cargo fmt`)
- [ ] No clippy warnings (`cargo clippy`)
- [ ] All tests pass (`cargo test`)
- [ ] **Coverage ≥ 85%** (`cargo tarpaulin --threshold 85`) ⭐
- [ ] Documentation updated
- [ ] Commit message follows convention

---

## Code Review

### Review Criteria

1. **Correctness**: Does the code do what it's supposed to?
2. **Style**: Does it follow Rust best practices?
3. **Performance**: Are there obvious performance issues?
4. **Testing**: Are there adequate tests?
5. **Documentation**: Is the code well-documented?

### Review Checklist

```markdown
## Code Review: PR #XXX

### Overview
Brief description of changes.

### Items to Review
- [ ] Logic is correct
- [ ] Error handling is proper
- [ ] No unwrap/expect in production code
- [ ] Tests cover edge cases
- [ ] **Coverage ≥ 85%** ⭐
- [ ] Documentation is updated
- [ ] No dead code commented out

### Coverage Analysis
- Current coverage: __%
- New code coverage: __%
- [ ] Meets 85% threshold

### Suggestions
List any suggestions for improvement.

### Approval
- [ ] Approve
- [ ] Request changes
- [ ] Comment
```

### Self-Review Before PR

```bash
# Format code
cargo fmt

# Run linter
cargo clippy --all-targets --all-features

# Run tests
cargo test --all-features

# ⭐ Check coverage (must be ≥85%)
cargo tarpaulin --out Html --threshold 85

# Check documentation
cargo doc --no-deps --open

# Run audit
cargo audit

# Check for outdated deps
cargo outdated
```

**Coverage Checklist:**
- [ ] Coverage ≥ 85% (run `cargo tarpaulin --threshold 85`)
- [ ] New code has corresponding tests
- [ ] Edge cases are covered
- [ ] Error paths are tested

---

## Continuous Integration

### GitHub Actions Example

```yaml
# .github/workflows/ci.yml
name: CI

on:
  push:
    branches: [main]
  pull_request:
    branches: [main]

jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - uses: dtolnay/rust-toolchain@stable
        with:
          components: rustfmt, clippy

      - name: Format check
        run: cargo fmt --all -- --check

      - name: Clippy
        run: cargo clippy --all-targets --all-features -- -D warnings

      - name: Tests
        run: cargo test --all-features

      - name: Build
        run: cargo build --release

  coverage:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - uses: dtolnay/rust-toolchain@stable

      - name: Install cargo-tarpaulin
        run: cargo install cargo-tarpaulin

      - name: Generate coverage report
        run: cargo tarpaulin --out Xml --output-dir coverage --threshold 85

      - name: Upload coverage to Codecov
        uses: codecov/codecov-action@v4
        with:
          files: ./coverage/cobertura.xml
          fail_ci_if_error: true
          token: ${{ secrets.CODECOV_TOKEN }}

      - name: Check coverage threshold
        run: |
          COVERAGE=$(cargo tarpaulin --out Json --output-dir coverage | jq '.coverage')
          echo "Coverage: $COVERAGE%"
          if (( $(echo "$COVERAGE < 85" | bc -l) )); then
            echo "❌ Coverage $COVERAGE% is below 85% threshold"
            exit 1
          fi
          echo "✅ Coverage $COVERAGE% meets 85% threshold"

  lint:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - uses: dtolnay/rust-toolchain@stable

      - name: Install cargo-audit
        run: cargo install cargo-audit

      - name: Security audit
        run: cargo audit
```

---

## Additional Resources

- [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/)
- [The Rust Style Guide](https://github.com/rust-dev-tools/fmt-rfcs/blob/master/guide/guide.md)
- [Effective Rust](https://www.lurklurk.org/effective-rust/)
- [Rust Design Patterns](https://rust-unofficial.github.io/patterns/)
