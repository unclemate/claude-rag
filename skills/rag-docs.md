---
description: 搜索项目文档，包括 README、设计文档、注释等
allowed-tools: Bash(**), Read(**)
argument-hint: <搜索查询>
---

## 用法

`/rag-docs <搜索查询>`

## 目标

使用 Claude RAG 搜索项目中的文档内容，包括 README、设计文档、内联注释等。

## 执行步骤

**步骤 1**：获取当前项目路径。

```bash
echo "${CLAUDE_PROJECT_PATH:-$(pwd)}"
```

**步骤 2**：调用 claude-rag 进行文档搜索。

```bash
/home/changh/Projects/claude-rag/target/release/claude-rag query "$ARGUMENTS" \
    --type docs \
    --top-k 10 \
    --format markdown
```

**步骤 3**：解析并展示搜索结果。

## 注意事项

- 文档类型包括：README、DESIGN、API 文档、内联注释等
- 搜索结果会包含文档的出处和上下文
- 适合查找设计决策、使用说明、架构描述等

## 输出格式

```markdown
## 📚 文档搜索结果

**查询**: $ARGUMENTS

### 结果 [N]
- **相似度**: XX%
- **来源**: `path/to/doc.md`

> 文档内容片段...

**上下文**: 简要说明
```
