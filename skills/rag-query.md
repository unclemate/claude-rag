---
description: Search all content (code+docs+session history), comprehensive query of project knowledge base
allowed-tools: Bash(**), Read(**)
argument-hint: <search query>
---

## Usage

`/rag-query <search query>`

## Objective

Use Claude RAG to search all knowledge content in the project, including source code, documentation, and session history.

## Execution Steps

**Step 1**: Get the current project path.

```bash
echo "${CLAUDE_PROJECT_PATH:-$(pwd)}"
```

**Step 2**: Call claude-rag for comprehensive search.

```bash
/home/changh/Projects/claude-rag/target/release/claude-rag query "$ARGUMENTS" \
    --top-k 5 \
    --format markdown
```

**Step 3**: Parse and display search results, grouped by type.

## Notes

- Comprehensive search returns relevant content of all types
- Results are sorted by similarity
- Suitable for exploratory queries to understand the overall project situation

## Output Format

```markdown
## 🔍 Comprehensive Search Results

**Query**: $ARGUMENTS

### 📄 Source Code
[relevant code results...]

### 📚 Documentation
[relevant documentation results...]

### 💬 Session History
[relevant discussion results...]
```
