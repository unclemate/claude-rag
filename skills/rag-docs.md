---
description: Search project documentation including README, design docs, comments, etc.
allowed-tools: Bash(**), Read(**)
argument-hint: <search query>
---

## Usage

`/rag-docs <search query>`

## Objective

Use Claude RAG to search documentation content in the project, including README, design documents, inline comments, etc.

## Execution Steps

**Step 1**: Get the current project path.

```bash
echo "${CLAUDE_PROJECT_PATH:-$(pwd)}"
```

**Step 2**: Call claude-rag to search documentation.

```bash
/home/changh/Projects/claude-rag/target/release/claude-rag query "$ARGUMENTS" \
    --type docs \
    --top-k 10 \
    --format markdown
```

**Step 3**: Parse and display search results.

## Notes

- Documentation types include: README, DESIGN, API docs, inline comments, etc.
- Search results include the source and context of the documentation
- Suitable for finding design decisions, usage instructions, architecture descriptions, etc.

## Output Format

```markdown
## 📚 Documentation Search Results

**Query**: $ARGUMENTS

### Result [N]
- **Similarity**: XX%
- **Source**: `path/to/doc.md`

> Documentation content snippet...

**Context**: brief description
```
