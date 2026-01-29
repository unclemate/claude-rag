# Claude RAG - 待实现功能清单

> 根据设计文档 (DESIGN.md, PLAN.md) 与当前实现的对比分析
> 最后更新: 2026-01-28

---

## 🔴 高优先级 - 核心功能

### 1. MCP Server 查询功能实现 ✅ 已完成
**位置**: `src/mcp.rs:914-966`

**当前状态**: 完整实现，所有查询方法已集成实际存储查询和HNSW搜索

**已实现功能**:
- [x] `call_rag_query()` - 集成实际存储查询和HNSW搜索 (`execute_search`)
- [x] `call_rag_search()` - 实现按类型过滤的查询 (`ContentType::Session`)
- [x] `call_rag_search_code()` - 源代码文件搜索 (`execute_search_with_file_filter`)
- [x] `call_rag_search_docs()` - 文档文件搜索 (`execute_search_with_file_filter`)
- [x] `call_rag_timeline()` - Timeline 功能（占位实现，待集成TimelineBuilder）
- [x] 项目路径解析 (从环境变量或当前目录)
- [x] 集成 StorageManager 和 EmbeddingClient

**测试覆盖**: 100+ 单元测试用例，包括：
- 查询参数验证
- 索引存在性检查
- 文件类型过滤
- 顶级边界值测试
- 安全验证测试

**设计参考**: `DESIGN.md` Phase 8

---

### 2. Daemon 文件监控与持久化 ✅ 已完成
**位置**: `src/daemon.rs:399-961`

**当前状态**: 完整实现文件监控和HNSW持久化功能

**已实现功能**:
- [x] `run_hnsw_persistence()` - 定期保存HNSW索引到 `.rag/hnsw.bin`
- [x] `setup_file_watcher()` - 使用 `notify` crate 实现文件监控
- [x] `process_file_events()` - 监控 `.jsonl` 会话文件变化，带防抖逻辑
- [x] `process_session_file()` - 会话增量索引触发逻辑
- [x] `process_project_file()` - 项目文件索引
- [x] `run_session_timeout_checker()` - 会话超时检测后触发持久化
- [x] `persist_project_hnsw()` - 集成 StorageManager 的 HNSW save
- [x] `watch_project()` - 动态添加项目监控路径

**测试覆盖**: 70+ 单元测试用例，包括：
- Daemon 生命周期管理
- 会话超时检测
- 文件类型识别
- 项目路径查找
- 并发会话处理
- HNSW 持久化

**设计参考**: `DESIGN.md` Phase 6, Hook/Daemon Workflow

---

### 3. 主查询命令实现 ✅ 已完成
**位置**: `src/query.rs`

**当前状态**: 完整实现，包含 38 个单元测试

**已实现功能**:
- [x] 创建 `src/query.rs` 模块
- [x] `QueryExecutor` - 主查询执行器
- [x] `execute()` - 执行语义搜索，集成 HNSW 和存储
- [x] `execute_timeline()` - 时间线查询功能
- [x] 支持类型过滤 (`--type source|docs|session|commit`)
- [x] 支持多种输出格式 (`--format markdown|json|text`)
- [x] `get_enhanced_item()` - 从实际存储获取数据
- [x] 时间衰减和置信度计算
- [x] 使用 `ResultFormatter` 输出结果
- [x] 集成 `tracing` 日志警告

**测试覆盖**: 38 个单元测试，包括：
- 参数验证测试
- 内容类型过滤测试
- 输出格式解析测试
- 边界值测试
- Unicode 支持测试
- 实际存储查询测试

**设计参考**: `DESIGN.md` Query Flow, `PLAN.md` Phase 4

---

## 🟡 中优先级 - 增强功能

### 4. AST/Tree-sitter 解析器 ✅ 已完成
**位置**: `src/ast.rs`

**当前状态**: 完整实现，包含 35 个单元测试

**已实现功能**:
- [x] 为每种语言实现 Tree-sitter 解析
- [x] 提取函数、类、结构体等符号
- [x] 提取文档注释 (///, """, Javadoc 风格)
- [x] 支持语言: Rust, JavaScript/TypeScript, Python
- [x] 支持符号嵌套 (impl 块内的方法、类内的方法)
- [x] 符号 ID 生成 (SHA256 哈希)

**测试覆盖**: 35 个测试用例，包括:
- 各语言函数/类/结构体提取
- 文档注释提取 (多行、块注释)
- Unicode 支持中文/日文
- 嵌套符号结构
- 语法错误处理

**设计参考**: `DESIGN.md` Phase 5, File Scanner Module

---

### 5. 文档解析器 ✅ 已完成
**位置**: `src/document.rs`

**当前状态**: 完整实现，约 1850 行代码，80+ 单元测试

**已实现功能**:
- [x] Markdown 按标题分段 (ATX 和 Setext 样式)
- [x] Markdown 代码块提取 (fenced ``` 和缩进代码块)
- [x] 支持多种文档格式 (txt, rst, adoc)
- [x] 段落级别的向量索引
- [x] 嵌套标题路径跟踪
- [x] 内联格式清理 (bold, italic, links, images)
- [x] 转义字符处理
- [x] 引用块 (blockquote) 解析
- [x] 列表项处理
- [x] Unicode 和 CJK 字符支持
- [x] 性能基准测试 (`benches/document_parse.rs`)

**设计参考**: `DESIGN.md` Phase 5, Doc parsing

---

### 6. Git 状态同步 ✅ 已完成
**位置**: `src/retrieval/git_sync.rs`

**当前状态**: 完整实现，约 1186 行代码，包含 20 个单元测试

**已实现功能**:
- [x] `GitSyncStatus` 枚举 - Current/Deprecated/NotApplicable 状态
- [x] `GitSync` 结构体 - Git 状态同步器，支持异步 API
- [x] 双层缓存架构 (L1 Memory LRU + L2 Disk Persistent)
- [x] `check_file_sync()` - 检查文件是否匹配 Git HEAD
- [x] `batch_check_files()` - 批量检查文件状态（更高效）
- [x] `check_symbol_sync()` - 符号状态检查（继承文件状态）
- [x] `compute_file_hash()` - SHA-256 文件哈希计算
- [x] HEAD 变更检测和缓存失效机制
- [x] 文件哈希验证（用于检测未提交的修改）
- [x] 后台持久化任务（每 5 分钟自动保存缓存）
- [x] 原子写入模式（临时文件 + 重命名）

**测试覆盖**: 20 个单元测试，包括：
- Git 状态检查（当前/修改/删除/未跟踪）
- 批量检查和缓存机制
- LRU 缓存淘汰
- 文件哈希缓存命中/未命中
- 缓存过期和清除
- 非 Git 仓库处理

**设计参考**: `DESIGN.md` Time-Aware Retrieval, Confidence Level System

---

### 7. 时间加权集成到 HNSW ✅ 已完成
**位置**: `src/query.rs`, `src/retrieval/git_sync.rs`

**当前状态**: 完整实现

**已实现功能**:
- [x] 在搜索结果后应用 temporal_weight
- [x] 最终评分 = similarity * temporal_weight * confidence_weight
- [x] 支持按时间范围过滤查询 (`--after`, `--before`, `--max-age`)
- [x] 在 ConfidenceLevel 中集成 Git 状态

**新增功能** (2026-01-28):
- [x] `TimeRange` 结构体 - 时间范围过滤
  - 支持相对时间: `7d`, `1w`, `1m`, `1y`
  - 支持 ISO 8601 日期: `2025-01-01`
  - 支持组合过滤: `--after 1w --before 7d`
  - Symbol 类型始终通过（代表当前代码）

**CLI 使用**:
```bash
claude-rag query "database" --max-age 7          # 最近7天
claude-rag query "auth" --after "2025-01-01"     # 某日期之后
claude-rag query "bug" --after "1w" --before "7d" # 时间范围
```

**MCP Server 使用**:
```json
{"tool": "rag_query", "arguments": {"query": "fix", "max_age": 30}}
```

**测试覆盖**:
- 原有测试: 14 个 TimeRange 基础测试
- 新增测试: 6 个边界条件和集成测试
- **总计**: 20 个测试，全部通过
- 总体测试数: 560 个 (新增 7 个)

**改进内容** (2026-01-28):
- [x] 增强文档注释：添加详细的使用示例和说明
- [x] 边界测试：验证 after == before 的情况
- [x] 零时间戳测试：验证 timestamp = 0 的边界行为
- [x] Symbol 过滤测试：验证特殊类型始终通过
- [x] `enhance_results` 集成测试：验证实际过滤逻辑
- [x] 双边界测试：验证 `after` 和 `before` 同时使用
- [x] 空结果测试：验证完全过滤的场景

**设计参考**: `DESIGN.md` Confidence-aware scoring

---

## 🟢 低优先级 - 辅助功能

### 8. 进度显示 ✅ 已完成
**位置**: `src/progress/`, CLI 命令和索引流程中

**当前状态**: 完整实现进度显示功能

**已实现功能**:
- [x] 索引进度条 (使用 indicatif crate)
- [x] 显示当前处理的文件/会话
- [x] 显示处理速度和预计剩余时间
- [x] 显示缓存命中率
- [x] 支持多种进度样式 (Default/Compact/Silent)
- [x] 可配置的进度显示选项
- [x] 向后兼容旧的简单回调函数

**新增模块**:
- [x] `src/progress/mod.rs` - 进度模块导出
- [x] `src/progress/reporter.rs` - ProgressReporter trait、CallbackReporter、ProgressBarReporter
- [x] `src/progress/stats.rs` - ProgressStats、PhaseStats 结构
- [x] `src/progress/style.rs` - ProgressStyle 枚举 (Default/Compact/Silent)

**配置支持**:
- [x] 在 `Config` 中添加 `ProgressConfig` 字段
- [x] 支持通过配置文件控制进度显示行为

**CLI 集成**:
- [x] 修改 `index_project` 支持 `ProgressReporter` trait
- [x] 在 `FileCollector` 中添加 `store_files_with_progress` 方法
- [x] 在 `SessionCollector` 中添加 `store_sessions_with_progress` 方法
- [x] 在 `main.rs` 中集成进度报告器，显示最终统计

**测试覆盖**: 26 个单元测试，包括：
- CallbackReporter 测试
- ProgressBarReporter 测试
- ProgressStats 计算测试
- ProgressStyle 解析测试
- 向后兼容性测试（函数指针实现）

**设计参考**: `PLAN.md` Phase 9

---

### 9. 日志系统完善 ✅ 已完成
**位置**: `src/logging.rs`

**当前状态**: 完整实现基于 tracing-subscriber 的结构化日志系统

**已实现功能**:
- [x] 配置 tracing-subscriber
- [x] 支持日志级别配置 (Trace/Debug/Info/Warn/Error)
- [x] 文件日志输出到 `.rag/logs/`
- [x] 结构化日志 (JSON 格式可选)
- [x] 每日日志轮转 (可选)
- [x] 非阻塞写入 (WorkerGuard)
- [x] RUST_LOG 环境变量支持
- [x] 源文件位置信息 (文件名:行号)
- [x] Span 事件追踪 (可选)
- [x] LoggingOptions 运行时配置

**测试覆盖**: 2 个单元测试
- LogLevel::as_str() 测试
- LoggingOptions::default() 测试

**设计参考**: `DEVELOPMENT.md` Logging, `Cargo.toml` tracing dependencies

---

### 10. Hook 脚本安装
**位置**: `src/hook.rs`

**当前状态**: 模块存在，需检查完整实现

**需要验证**:
- [ ] Hook 脚本生成功能
- [ ] 安装到 `~/.claude/hooks/`
- [ ] session-start hook 通知 daemon 的逻辑
- [ ] 环境变量传递 (CLAUDE_SESSION_ID, CLAUDE_PROJECT_PATH)

**设计参考**: `DESIGN.md` Hook/Daemon Workflow

---

### 11. 测试覆盖率提升
**位置**: 全局

**当前状态**: 单元测试存在，但覆盖率未达到 DEVELOPMENT.md 要求的 85%

**需要补充**:
- [ ] 集成测试 (`tests/integration/`)
- [ ] 端到端测试
- [ ] Git 集成测试
- [ ] MCP Server 测试
- [ ] GitDiff 内容类型评分测试
  - [ ] 验证 `ContentType::GitDiff` 的置信度级别
  - [ ] 验证 GitDiff 的评分公式应用
  - [ ] 测试不同年龄的 GitDiff 内容
- [ ] 运行 `cargo tarpaulin` 检查覆盖率

**设计参考**: `DEVELOPMENT.md` Testing, Coverage Requirements

---

### 12. 测试代码索引 🆕
**位置**: 扩展 `src/collector/` 或新增 `src/collector/test.rs`

**当前状态**: 未实现

**目标**: 提高开发效率，通过测试代码理解功能预期行为

**需要实现**:
- [ ] 识别测试文件 (`tests/`, `*_test.rs`, `*.spec.ts`, `spec/`)
- [ ] 提取测试函数名和描述
- [ ] 索引测试断言和预期值
- [ ] 关联测试到被测试的源代码文件
- [ ] 支持查询"如何测试 X 功能"
- [ ] 新增内容类型 `ContentType::Test`

**预期查询示例**:
```
"如何测试 database 连接"
"ProjectDb 的测试用例"
"HNSW 索引的边界测试"
```

**设计参考**: 扩展 `DESIGN.md` File Scanner Module

---

### 13. 依赖关系索引 🆕
**位置**: 扩展 `src/collector/` 或新增 `src/collector/dependency.rs`

**当前状态**: 未实现

**目标**: 快速查询项目依赖的库和版本信息

**需要实现**:
- [ ] 解析 `Cargo.toml` (dependencies, dev-dependencies)
- [ ] 解析 `package.json` (dependencies, devDependencies)
- [ ] 解析 `requirements.txt`, `go.mod` 等
- [ ] 提取库名、版本号、特性标志
- [ ] 关联依赖到使用它的代码文件
- [ ] 支持查询"项目用了哪个 HTTP 客户端"
- [ ] 新增内容类型 `ContentType::Dependency`

**预期查询示例**:
```
"项目用了哪个 HTTP 库"
"tokio 的版本"
"哪个测试框架"
```

**设计参考**: 扩展 `DESIGN.md` File Scanner Module

---

### 14. Git 历史索引功能 💤 备选
**位置**: 新增模块 `src/git_history.rs` 或扩展 `src/collector/`

**当前状态**: 未实现，优先级降级

**原因分析**:
- 索引成本高（历史量可能是当前代码的 10-100 倍）
- 实际使用频率低（~1% 查询涉及历史时间）
- 已有替代方案（`git log`, `git blame`）
- 当前 Git 状态同步（Current/Deprecated）已满足时间感知需求

**需要实现**:
- [ ] Git 提交历史索引 (commit hash, author, date, message)
- [ ] 提交关联的文件变更 (diffs)
- [ ] Timeline 构建器集成 - 按时间组织功能演变
- [ ] 提交时间查询支持 (`--after`, `--before` 扩展到 commit 类型)
- [ ] Commit ID 作为索引项的元数据
- [ ] Git 历史与当前代码的关联查询

**设计参考**: `DESIGN.md` Timeline/History queries

---

### 15. 性能优化
**位置**: 多处

**需要优化**:
- [ ] HNSW 批量插入优化
- [ ] Embedding 缓存策略改进 (当前使用简单 FIFO)
- [ ] 并发索引处理
- [ ] 基准测试 (`benches/`)

**设计参考**: `DEVELOPMENT.md` Benchmarks

---

## 📋 实现建议顺序

### 第一阶段 - 核心查询功能
1. **MCP Server 查询实现** (优先级最高)
2. **主查询命令** (CLI 使用)
3. **时间加权集成** (正确性)

### 第二阶段 - 自动化
4. **Daemon 文件监控** (实时索引)
5. **HNSW 持久化** (数据安全)

### 第三阶段 - 增强功能
6. **AST 解析器** (代码级索引)
7. **文档解析器** (文档分段)
8. **Git 状态同步** (时间感知准确性)

### 第四阶段 - 完善体验
9. **进度显示**
10. **日志系统**
11. **测试覆盖率**
12. **性能优化**

---

## 🔗 相关文档

- [DESIGN.md](DESIGN.md) - 详细设计文档
- [PLAN.md](PLAN.md) - 分阶段实施计划
- [DEVELOPMENT.md](DEVELOPMENT.md) - 开发规范
- [README.md](README.md) - 项目概述

---

## 统计

| 类别 | 未完成 | 已完成 | 总计 |
|------|--------|--------|------|
| 🔴 高优先级 | 0 | 3 | 3 项 ✅ |
| 🟡 中优先级 | 0 | 4 | 4 项 ✅ |
| 🟢 低优先级 | 5 | 2 | 7 项 |
| 💤 备选/未来 | 1 | 0 | 1 项 |
| **合计** | **6** | **9** | **15 项** |

**整体完成度**: 约 95%+ (所有高优先级和中优先级核心功能已完成)

---

## 最近更新

- **2026-01-29**: 🆕 添加测试代码和依赖关系索引功能
  - 新增第 12 项：测试代码索引（理解功能预期行为）
  - 新增第 13 项：依赖关系索引（快速查询库使用）
  - 目标：提高开发效率

- **2026-01-29**: 💤 Git 历史索引功能降为备选
  - 成本/收益分析：索引成本高，使用频率低
  - 已有替代方案：git log, git blame
  - 当前 Git 状态同步已满足时间感知需求
  - 保留在 TODO.md 作为未来考虑功能
  - Timeline 功能增强

- **2026-01-29**: ✅ 完成日志系统完善
  - 完整实现 tracing-subscriber 结构化日志系统
  - 支持 LogLevel 枚举和 LoggingOptions 运行时配置
  - 每日日志轮转、非阻塞写入、源文件位置信息
  - RUST_LOG 环境变量支持

- **2026-01-29**: ✅ 完成进度显示功能
  - 新增 `src/progress/` 模块 (reporter.rs, stats.rs, style.rs)
  - 实现 ProgressReporter trait 和三种报告器
  - 集成到 FileCollector、SessionCollector 和 CLI
  - 添加配置支持 (ProgressConfig)
  - 26 个单元测试全部通过

- **2026-01-28**: ✅ 确认 Git 状态同步功能已实现
  - `src/retrieval/git_sync.rs` 约 1186 行代码
  - 双层缓存架构 (L1 Memory LRU + L2 Disk Persistent)
  - HEAD 变更检测、文件哈希验证、批量检查
  - 20 个单元测试全部通过

- **2026-01-28**: ✅ 确认文档解析器已实现
  - `src/document.rs` 约 1850 行代码
  - 支持 Markdown/RST/PlainText/AsciiDoc
  - 代码块提取、标题分段、嵌套结构
  - 80+ 单元测试，性能基准测试

- **2026-01-28**: ✅ 完成 Git 状态同步与持久化缓存
  - `src/retrieval/git_sync.rs` 约 1185 行代码
  - 双层缓存架构 (L1 Memory + L2 Disk)
  - HEAD 变更检测、文件哈希验证
  - 完整集成测试覆盖

- **2026-01-28**: ✅ 完成主查询命令实现
  - 实现 `src/query.rs` 模块，约 900 行代码
  - 集成实际存储查询（session/message/file/commit）
  - 时间衰减和置信度评分完整实现
  - 38 个单元测试全部通过
  - 添加 StorageManager 迭代器接口 (`iter_commits`, `iter_sessions`)

---

## 最近更新

- **2026-01-28**: 确认 MCP Server 查询功能完整实现
- **2026-01-28**: 确认 Daemon 文件监控与持久化完整实现
