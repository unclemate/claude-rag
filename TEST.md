# Claude RAG Complete Integration Test Plan

> Using the current project `/home/changh/Projects/claude-rag` as the test subject, execute end-to-end integration testing

---

## Test Objectives

Verify the complete functional workflow of Claude RAG:
1. ✅ Project initialization
2. ✅ API configuration
3. ✅ Index creation (files + vectors)
4. ✅ MCP Server startup
5. ✅ Query functionality verification

---

## Prerequisites

### 1. API Token Preparation

Requires Zhipu AI API key:
- Get it at: https://open.bigmodel.cn/
- Model: embedding-3 (for vector embeddings)

### 2. Binary Build

```bash
cd /home/changh/Projects/claude-rag
cargo build --release
```

Binary location: `/home/changh/Projects/claude-rag/target/release/claude-rag`

---

## Test Steps

### Step 1: Clean Environment

**Purpose**: Ensure testing starts from a clean state

```bash
cd /home/changh/Projects/claude-rag
rm -rf .rag
rm -rf ~/.claude/rag/config.toml  # Optional: clear global config
```

**Verification points**:
- `.rag` directory does not exist
- No old configuration interference

---

### Step 2: Initialize Project

**Command**:
```bash
cd /home/changh/Projects/claude-rag
./target/release/claude-rag init
```

**Expected output**:
```
Initializing knowledge base...
✓ Knowledge base initialized
  Location: /home/changh/Projects/claude-rag/.rag
  Storage size: 0 bytes
```

**Verification points**:
- ✅ `.rag` directory created
- ✅ `.rag/db` directory exists
- ✅ `.rag/config.json` file exists
- ✅ Configuration file contains default values

**Verification commands**:
```bash
ls -la .rag/
cat .rag/config.json
```

---

### Step 3: Configure API Token

**Method A: Direct configuration file editing**

```bash
nano .rag/config.json
```

Add API token:
```json
{
  "embedding": {
    "api_token": "your-zhipu-ai-key",
    "api_url": "https://open.bigmodel.cn/api/paas/v4/embeddings",
    "dimensions": 1024,
    "batch_size": 8,
    "timeout_ms": 30000
  },
  "hnsw": {
    "m": 16,
    "ef_construction": 200,
    "ef_search": 50
  }
}
```

**Method B: Environment variable**

```bash
export CLAUDE_RAG_API_TOKEN="your-zhipu-ai-key"
```

**Verification points**:
- ✅ Configuration file contains valid `api_token`
- ✅ `api_token` is not empty

---

### Step 4: Execute Indexing

**Command**:
```bash
./target/release/claude-rag index --all --force
```

**Expected output example**:
```
Initializing knowledge base...
Collecting files...
Indexing files...
✓ Indexing complete
  Files indexed: 150
  Chunks created: 450
  Sessions indexed: 0
  Errors: 0
```

**Verification points**:
- ✅ Index command executes successfully
- ✅ `Files indexed > 0`
- ✅ `Chunks created > 0` (vector indexing successful)
- ✅ `.rag/hnsw.bin` file exists and is not empty

**Verification commands**:
```bash
# Check HNSW index file
ls -lh .rag/hnsw.bin

# View index status
./target/release/claude-rag status
```

---

### Step 5: Test Query Functionality

**Test 5.1: Basic query**

```bash
./target/release/claude-rag query "HNSW index implementation"
```

**Expected output**:
```
# Query results: "HNSW index implementation"

## 1. src/storage/hnsw.rs (confidence: 0.95)
Path: src/storage/hnsw.rs
...

[Return relevant code snippets]
```

**Verification points**:
- ✅ Query returns results
- ✅ Results contain relevant files
- ✅ Confidence scores are reasonable

---

**Test 5.2: Type-filtered query**

```bash
# Query only source code
./target/release/claude-rag query "vector embedding" --type source

# Query only documentation
./target/release/claude-rag query "usage instructions" --type docs
```

**Verification points**:
- ✅ Type filtering works correctly

---

**Test 5.3: Time range query**

```bash
# Query content from last 7 days
./target/release/claude-rag query "new features" --max-age 7

# Query content after specific date
./target/release/claude-rag query "refactoring" --after "2025-01-20"
```

**Verification points**:
- ✅ Time filtering works correctly

---

### Step 6: Test MCP Server

**Test 6.1: List available tools**

```bash
echo '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' | \
  ./target/release/claude-rag mcp-server
```

**Expected response**:
```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "tools": [
      {
        "name": "rag_query",
        "description": "Query the RAG knowledge base..."
      },
      {
        "name": "rag_search_code",
        "description": "Search only source code files..."
      },
      {
        "name": "rag_search_docs",
        "description": "Search only documentation files..."
      },
      {
        "name": "rag_search_session",
        "description": "Search only previous Claude sessions..."
      },
      {
        "name": "rag_timeline",
        "description": "Build a feature timeline..."
      }
    ]
  }
}
```

**Verification points**:
- ✅ Returns 5 tools
- ✅ Each tool has a name and description

---

**Test 6.2: Call query tool**

```bash
echo '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"rag_query","arguments":{"query":"HNSW索引","top_k":5}}}' | \
  ./target/release/claude-rag mcp-server
```

**Verification points**:
- ✅ Returns query results
- ✅ Result format is correct

---

### Step 7: Complete Functionality Verification

**Test scenario**: Query core project functionality

```bash
./target/release/claude-rag query "how are vector embeddings generated" --top-k 10
```

**Verification points**:
- ✅ Returns relevant implementation code
- ✅ Includes confidence scores
- ✅ Results sorted by relevance

---

## Test Checklist

### Environment Preparation
- [ ] Binary built
- [ ] API token obtained
- [ ] Test environment cleaned

### Initialization Test
- [ ] `.rag` directory created successfully
- [ ] Configuration file generated correctly
- [ ] Database initialized successfully

### Configuration Test
- [ ] API token configured successfully
- [ ] Configuration file format correct
- [ ] Configuration loads without errors

### Indexing Test
- [ ] Index command executes successfully
- [ ] File collection correct
- [ ] Vector embedding generation successful
- [ ] HNSW index file created
- [ ] Statistics accurate

### Query Test
- [ ] Basic query returns results
- [ ] Type filtering works
- [ ] Time filtering works
- [ ] Result relevance reasonable

### MCP Test
- [ ] MCP Server starts successfully
- [ ] tools/list returns correctly
- [ ] Tool calls work normally
- [ ] Error handling correct

---

## Expected Issues and Solutions

### Issue 1: API Token Not Configured

**Error message**:
```
Embedding API not configured. Please set api_token in config.
```

**Solution**:
Check that the `api_token` field in `.rag/config.json` is not empty

---

### Issue 2: HNSW Index File Not Created

**Possible causes**:
- API call failed
- Network issues
- Invalid token

**Debug commands**:
```bash
# Check index status
./target/release/claude-rag status

# View logs (if any)
ls -la .rag/logs/
```

---

### Issue 3: Query Returns Empty Results

**Possible causes**:
- Indexing not completed
- HNSW index is empty
- Query term not relevant

**Verification commands**:
```bash
# Check HNSW index size
ls -lh .rag/hnsw.bin

# Use more generic query terms
./target/release/claude-rag query "function"
```

---

## Success Criteria

Test passes if:
1. ✅ All 6 steps execute successfully
2. ✅ HNSW index file size > 0
3. ✅ Queries return meaningful results
4. ✅ MCP Server responds normally
5. ✅ No critical errors or panics

---

## Test Report Template

```markdown
# Claude RAG Integration Test Report

**Test date**: 2025-01-29
**Test environment**: /home/changh/Projects/claude-rag
**Version**: 0.1.0

## Test Results

| Step | Status | Notes |
|------|--------|-------|
| Environment preparation | ✅/❌ | |
| Initialization | ✅/❌ | |
| Configuration | ✅/❌ | |
| Indexing | ✅/❌ | |
| Query | ✅/❌ | |
| MCP | ✅/❌ | |

## Statistics

- Files indexed: ___
- Chunks created: ___
- HNSW index size: ___
- Query response time: ___

## Issues and Suggestions

(Record discovered issues and improvement suggestions)
```

---

## Key File Paths

| File | Purpose |
|------|---------|
| `/home/changh/Projects/claude-rag/.rag/config.json` | Configuration file |
| `/home/changh/Projects/claude-rag/.rag/hnsw.bin` | HNSW index |
| `/home/changh/Projects/claude-rag/.rag/db/` | Database directory |
| `/home/changh/Projects/claude-rag/target/release/claude-rag` | Binary file |
