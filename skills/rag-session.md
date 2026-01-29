---
description: 搜索 Claude Code 会话历史，查找之前讨论过的内容和决策
allowed-tools: Bash(**), Read(**)
argument-hint: <搜索查询>
---

## 用法

`/rag-session <搜索查询>`

## 目标

使用 Claude RAG 搜索项目的历史会话记录，查找之前讨论过的内容、决策和方案。

## 执行步骤

**步骤 1**：获取当前项目路径。

```bash
echo "${CLAUDE_PROJECT_PATH:-$(pwd)}"
```

**步骤 2**：调用 claude-rag 进行会话历史搜索。

```bash
/home/changh/Projects/claude-rag/target/release/claude-rag query "$ARGUMENTS" \
    --type session \
    --top-k 10 \
    --format markdown
```

**步骤 3**：解析并展示搜索结果，包括讨论的时间戳和上下文。

## 注意事项

- 会话历史包含用户输入和 AI 输出
- 结果会显示讨论的时间，便于判断内容的新旧
- 适合查找"之前是否讨论过这个问题"或"当时为什么这样决定"

## 输出格式

```markdown
## 💬 会话历史搜索结果

**查询**: $ARGUMENTS

### 结果 [N]
- **相似度**: XX%
- **时间**: YYYY-MM-DD
- **会话**: 会话标题

> **User**: 问题内容...
>
> **AI**: 回答内容...

**总结**: 该讨论的要点说明
```
