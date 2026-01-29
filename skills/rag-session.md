---
description: Search Claude Code session history for previously discussed content and decisions
allowed-tools: Bash(**), Read(**)
argument-hint: <search query>
---

## Usage

`/rag-session <search query>`

## Objective

Use Claude RAG to search the project's historical session records for previously discussed content, decisions, and solutions.

## Execution Steps

**Step 1**: Get the current project path.

```bash
echo "${CLAUDE_PROJECT_PATH:-$(pwd)}"
```

**Step 2**: Call claude-rag to search session history.

```bash
/home/changh/Projects/claude-rag/target/release/claude-rag query "$ARGUMENTS" \
    --type session \
    --top-k 10 \
    --format markdown
```

**Step 3**: Parse and display search results, including discussion timestamps and context.

## Notes

- Session history includes both user inputs and AI outputs
- Results show the time of discussion to help assess the recency of content
- Suitable for finding "have we discussed this before" or "why did we decide this"

## Output Format

```markdown
## 💬 Session History Search Results

**Query**: $ARGUMENTS

### Result [N]
- **Similarity**: XX%
- **Time**: YYYY-MM-DD
- **Session**: session title

> **User**: question content...
>
> **AI**: response content...

**Summary**: key points of the discussion
```
