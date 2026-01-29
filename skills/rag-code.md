---
description: 搜索项目源代码，基于语义理解查找相关函数、类和实现
allowed-tools: Bash(**), Read(**)
argument-hint: <搜索查询>
---

## 用法

`/rag-code <搜索查询>`

## 目标

使用 Claude RAG 语义搜索查找项目中的相关源代码，基于功能语义而非关键字匹配。

## 执行步骤

**步骤 1**：获取当前项目路径。

```bash
echo "${CLAUDE_PROJECT_PATH:-$(pwd)}"
```

**步骤 2**：调用 claude-rag 进行源代码搜索。

```bash
/home/changh/Projects/claude-rag/target/release/claude-rag query "$ARGUMENTS" \
    --type source \
    --top-k 10 \
    --format markdown
```

**步骤 3**：解析并展示搜索结果。

对于每个匹配项，提供：
- 文件路径和行号范围
- 相似度分数
- 相关代码片段
- 上下文说明

## 注意事项

- 搜索结果基于语义相似度，可能包含功能相似但命名不同的代码
- 默认返回前 10 个最相关的结果
- 如需更精确的结果，可以在查询中包含具体的类名、函数名或文件名

## 输出格式

使用以下格式展示结果：

```markdown
## 🎯 源代码搜索结果

**查询**: $ARGUMENTS

### 结果 [N]
- **相似度**: XX%
- **文件**: `path/to/file.rs:行号`
- **类型**: 函数/类/方法

```rust
// 代码片段
```

**上下文**: 简要说明
```
