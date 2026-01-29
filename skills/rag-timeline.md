---
description: 按时间线展示功能的演进历史，包括 Git 提交和会话讨论
allowed-tools: Bash(**), Read(**)
argument-hint: <功能或主题>
---

## 用法

`/rag-timeline <功能或主题>`

## 目标

使用 Claude RAG 的时间线查询功能，展示某个功能或主题的演进历史，包括代码变更和讨论历史。

## 执行步骤

**步骤 1**：获取当前项目路径。

```bash
echo "${CLAUDE_PROJECT_PATH:-$(pwd)}"
```

**步骤 2**：调用 claude-rag 进行时间线查询。

```bash
/home/changh/Projects/claude-rag/target/release/claude-rag query "$ARGUMENTS" \
    --timeline \
    --format markdown
```

**步骤 3**：解析并按时间顺序展示结果。

## 注意事项

- 时间线查询会显示功能的完整演进过程
- 包含 Git 提交信息和会话讨论
- 按时间倒序排列，最新的在前
- 带有置信度颜色标记（绿色=当前代码，蓝色=Git提交，黄色=近期会话，红色=旧讨论）

## 输出格式

```markdown
## 📜 时间线: $ARGUMENTS

### 📌 当前状态
[当前代码实现状态...]

### 📜 变更时间线

#### 🟢 YYYY-MM-DD (时间描述)
**[Git 变更/讨论]** 描述

变更/讨论内容...

---

#### 🔵 YYYY-MM-DD
...

---

#### 🟡 YYYY-MM-DD
...
```
