---
description: Search project source code using semantic understanding to find relevant functions, classes, and implementations
allowed-tools: Bash(**), Read(**)
argument-hint: <search query>
---

## Usage

`/rag-code <search query>`

## Objective

Use Claude RAG semantic search to find relevant source code in the project based on functional semantics rather than keyword matching.

## Execution Steps

**Step 1**: Get the current project path.

```bash
echo "${CLAUDE_PROJECT_PATH:-$(pwd)}"
```

**Step 2**: Call claude-rag to search source code.

```bash
/home/changh/Projects/claude-rag/target/release/claude-rag query "$ARGUMENTS" \
    --type source \
    --top-k 10 \
    --format markdown
```

**Step 3**: Parse and display search results.

For each match, provide:
- File path and line number range
- Similarity score
- Relevant code snippet
- Context description

## Notes

- Search results are based on semantic similarity and may include code with similar functionality but different naming
- By default, returns the top 10 most relevant results
- For more precise results, include specific class names, function names, or file names in your query

## Output Format

Display results using the following format:

```markdown
## 🎯 Source Code Search Results

**Query**: $ARGUMENTS

### Result [N]
- **Similarity**: XX%
- **File**: `path/to/file.rs:line_number`
- **Type**: function/class/method

```rust
// code snippet
```

**Context**: brief description
```
