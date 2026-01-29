---
description: 搜索全部内容（代码+文档+会话历史），综合查询项目知识库
allowed-tools: Bash(**), Read(**)
argument-hint: <搜索查询>
---

## 用法

`/rag-query <搜索查询>`

## 目标

使用 Claude RAG 搜索项目的全部知识内容，包括源代码、文档和会话历史。

## 执行步骤

**步骤 1**：获取当前项目路径。

```bash
echo "${CLAUDE_PROJECT_PATH:-$(pwd)}"
```

**步骤 2**：调用 claude-rag 进行综合搜索。

```bash
/home/changh/Projects/claude-rag/target/release/claude-rag query "$ARGUMENTS" \
    --top-k 5 \
    --format markdown
```

**步骤 3**：解析并展示搜索结果，按类型分组。

## 注意事项

- 综合搜索会返回所有类型的相关内容
- 结果会按照相似度排序
- 适合探索性查询，了解项目的整体情况

## 输出格式

```markdown
## 🔍 综合搜索结果

**查询**: $ARGUMENTS

### 📄 源代码
[相关代码结果...]

### 📚 文档
[相关文档结果...]

### 💬 会话历史
[相关讨论结果...]
```
