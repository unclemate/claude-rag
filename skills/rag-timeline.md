---
description: Display feature evolution history over time, including Git commits and session discussions
allowed-tools: Bash(**), Read(**)
argument-hint: <feature or topic>
---

## Usage

`/rag-timeline <feature or topic>`

## Objective

Use Claude RAG's timeline query feature to display the evolution history of a feature or topic, including code changes and discussion history.

## Execution Steps

**Step 1**: Get the current project path.

```bash
echo "${CLAUDE_PROJECT_PATH:-$(pwd)}"
```

**Step 2**: Call claude-rag for timeline query.

```bash
/home/changh/Projects/claude-rag/target/release/claude-rag query "$ARGUMENTS" \
    --timeline \
    --format markdown
```

**Step 3**: Parse and display results in chronological order.

## Notes

- Timeline query shows the complete evolution process of a feature
- Includes Git commit information and session discussions
- Sorted in reverse chronological order, newest first
- Includes confidence color indicators (green=current code, blue=Git commit, yellow=recent session, red=old discussion)

## Output Format

```markdown
## 📜 Timeline: $ARGUMENTS

### 📌 Current Status
[current code implementation status...]

### 📜 Change Timeline

#### 🟢 YYYY-MM-DD (time description)
**[Git Change/Discussion]** description

change/discussion content...

---

#### 🔵 YYYY-MM-DD
...

---

#### 🟡 YYYY-MM-DD
...
```
