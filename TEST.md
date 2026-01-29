# Claude RAG 完整集成测试方案

> 以当前项目 `/home/changh/Projects/claude-rag` 为被测对象，执行端到端的集成测试

---

## 测试目标

验证 Claude RAG 的完整功能流程：
1. ✅ 项目初始化
2. ✅ API 配置
3. ✅ 索引创建（文件 + 向量）
4. ✅ MCP Server 启动
5. ✅ 查询功能验证

---

## 前置条件

### 1. API Token 准备

需要智谱 AI (Zhipu AI) 的 API 密钥：
- 获取地址：https://open.bigmodel.cn/
- 模型：embedding-3 (用于向量嵌入)

### 2. 二进制构建

```bash
cd /home/changh/Projects/claude-rag
cargo build --release
```

二进制位置：`/home/changh/Projects/claude-rag/target/release/claude-rag`

---

## 测试步骤

### 步骤 1: 清理环境

**目的**：确保从干净状态开始测试

```bash
cd /home/changh/Projects/claude-rag
rm -rf .rag
rm -rf ~/.claude/rag/config.toml  # 可选：清除全局配置
```

**验证点**：
- `.rag` 目录不存在
- 无旧配置影响

---

### 步骤 2: 初始化项目

**命令**：
```bash
cd /home/changh/Projects/claude-rag
./target/release/claude-rag init
```

**预期输出**：
```
Initializing knowledge base...
✓ Knowledge base initialized
  Location: /home/changh/Projects/claude-rag/.rag
  Storage size: 0 bytes
```

**验证点**：
- ✅ `.rag` 目录已创建
- ✅ `.rag/db` 目录存在
- ✅ `.rag/config.json` 文件存在
- ✅ 配置文件包含默认值

**验证命令**：
```bash
ls -la .rag/
cat .rag/config.json
```

---

### 步骤 3: 配置 API Token

**方式 A: 直接编辑配置文件**

```bash
nano .rag/config.json
```

添加 API token：
```json
{
  "embedding": {
    "api_token": "你的智谱AI密钥",
    "api_url": "https://open.bigmodel.cn/api/paas/v4/embeddings",
    "dimensions": 1024,
    "batch_size": 8,
    "timeout_ms": 30000
  },
  "hnsw": {
    "m": 16,
    "ef_construction": 200,
    "ef_search": 50
  }
}
```

**方式 B: 环境变量**

```bash
export CLAUDE_RAG_API_TOKEN="你的智谱AI密钥"
```

**验证点**：
- ✅ 配置文件包含有效的 `api_token`
- ✅ `api_token` 非空

---

### 步骤 4: 执行索引

**命令**：
```bash
./target/release/claude-rag index --all --force
```

**预期输出示例**：
```
Initializing knowledge base...
Collecting files...
Indexing files...
✓ Indexing complete
  Files indexed: 150
  Chunks created: 450
  Sessions indexed: 0
  Errors: 0
```

**验证点**：
- ✅ 索引命令成功执行
- ✅ `Files indexed > 0`
- ✅ `Chunks created > 0` (向量索引成功)
- ✅ `.rag/hnsw.bin` 文件存在且非空

**验证命令**：
```bash
# 检查 HNSW 索引文件
ls -lh .rag/hnsw.bin

# 查看索引状态
./target/release/claude-rag status
```

---

### 步骤 5: 测试查询功能

**测试 5.1: 基本查询**

```bash
./target/release/claude-rag query "HNSW 索引实现"
```

**预期输出**：
```
# 查询结果: "HNSW 索引实现"

## 1. src/storage/hnsw.rs (置信度: 0.95)
路径: src/storage/hnsw.rs
...

[返回相关的代码片段]
```

**验证点**：
- ✅ 查询返回结果
- ✅ 结果包含相关文件
- ✅ 置信度分数合理

---

**测试 5.2: 类型过滤查询**

```bash
# 只查询源代码
./target/release/claude-rag query "向量嵌入" --type source

# 只查询文档
./target/release/claude-rag query "使用说明" --type docs
```

**验证点**：
- ✅ 类型过滤正常工作

---

**测试 5.3: 时间范围查询**

```bash
# 查询最近 7 天的内容
./target/release/claude-rag query "新功能" --max-age 7

# 查询特定日期之后的内容
./target/release/claude-rag query "重构" --after "2025-01-20"
```

**验证点**：
- ✅ 时间过滤正常工作

---

### 步骤 6: 测试 MCP Server

**测试 6.1: 列出可用工具**

```bash
echo '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' | \
  ./target/release/claude-rag mcp-server
```

**预期响应**：
```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "tools": [
      {
        "name": "rag_query",
        "description": "Query the RAG knowledge base..."
      },
      {
        "name": "rag_search_code",
        "description": "Search only source code files..."
      },
      {
        "name": "rag_search_docs",
        "description": "Search only documentation files..."
      },
      {
        "name": "rag_search_session",
        "description": "Search only previous Claude sessions..."
      },
      {
        "name": "rag_timeline",
        "description": "Build a feature timeline..."
      }
    ]
  }
}
```

**验证点**：
- ✅ 返回 5 个工具
- ✅ 每个工具有名称和描述

---

**测试 6.2: 调用查询工具**

```bash
echo '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"rag_query","arguments":{"query":"HNSW索引","top_k":5}}}' | \
  ./target/release/claude-rag mcp-server
```

**验证点**：
- ✅ 返回查询结果
- ✅ 结果格式正确

---

### 步骤 7: 完整功能验证

**测试场景**：查询项目核心功能

```bash
./target/release/claude-rag query "向量嵌入如何生成" --top-k 10
```

**验证点**：
- ✅ 返回相关的实现代码
- ✅ 包含置信度评分
- ✅ 结果按相关性排序

---

## 测试检查清单

### 环境准备
- [ ] 二进制已构建
- [ ] API token 已获取
- [ ] 测试环境已清理

### 初始化测试
- [ ] `.rag` 目录创建成功
- [ ] 配置文件生成正确
- [ ] 数据库初始化成功

### 配置测试
- [ ] API token 配置成功
- [ ] 配置文件格式正确
- [ ] 配置加载无错误

### 索引测试
- [ ] 索引命令执行成功
- [ ] 文件收集正确
- [ ] 向量嵌入生成成功
- [ ] HNSW 索引文件创建
- [ ] 统计信息准确

### 查询测试
- [ ] 基本查询返回结果
- [ ] 类型过滤正常
- [ ] 时间过滤正常
- [ ] 结果相关性合理

### MCP 测试
- [ ] MCP Server 启动成功
- [ ] tools/list 返回正确
- [ ] 工具调用正常
- [ ] 错误处理正确

---

## 预期问题和解决方案

### 问题 1: API Token 未配置

**错误信息**：
```
Embedding API not configured. Please set api_token in config.
```

**解决方案**：
检查 `.rag/config.json` 中的 `api_token` 字段是否非空

---

### 问题 2: HNSW 索引文件未创建

**可能原因**：
- API 调用失败
- 网络问题
- Token 无效

**调试命令**：
```bash
# 检查索引状态
./target/release/claude-rag status

# 查看日志（如果有）
ls -la .rag/logs/
```

---

### 问题 3: 查询返回空结果

**可能原因**：
- 索引未完成
- HNSW 索引为空
- 查询词不相关

**验证命令**：
```bash
# 检查 HNSW 索引大小
ls -lh .rag/hnsw.bin

# 使用更通用的查询词
./target/release/claude-rag query "函数"
```

---

## 成功标准

测试通过的标准：
1. ✅ 所有 6 个步骤成功执行
2. ✅ HNSW 索引文件大于 0
3. ✅ 查询返回有意义的结果
4. ✅ MCP Server 正常响应
5. ✅ 无严重错误或 panic

---

## 测试报告模板

```markdown
# Claude RAG 集成测试报告

**测试日期**: 2025-01-29
**测试环境**: /home/changh/Projects/claude-rag
**版本**: 0.1.0

## 测试结果

| 步骤 | 状态 | 说明 |
|------|------|------|
| 环境准备 | ✅/❌ | |
| 初始化 | ✅/❌ | |
| 配置 | ✅/❌ | |
| 索引 | ✅/❌ | |
| 查询 | ✅/❌ | |
| MCP | ✅/❌ | |

## 统计数据

- 索引文件数: ___
- 创建的块数: ___
- HNSW 索引大小: ___
- 查询响应时间: ___

## 问题和建议

(记录发现的问题和改进建议)
```

---

## 关键文件路径

| 文件 | 用途 |
|------|------|
| `/home/changh/Projects/claude-rag/.rag/config.json` | 配置文件 |
| `/home/changh/Projects/claude-rag/.rag/hnsw.bin` | HNSW 索引 |
| `/home/changh/Projects/claude-rag/.rag/db/` | 数据库目录 |
| `/home/changh/Projects/claude-rag/target/release/claude-rag` | 二进制文件 |
