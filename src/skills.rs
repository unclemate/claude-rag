//! Claude Code Skills generator.
//!
//! This module generates and installs Claude Code skills for RAG queries.

use crate::error::{RagError, Result};
use std::fs;
use std::path::{Path, PathBuf};

/// Skills installer.
pub struct SkillsInstaller {
    /// Claude commands directory.
    commands_dir: PathBuf,
}

impl SkillsInstaller {
    /// Create a new skills installer.
    pub fn new() -> Result<Self> {
        let commands_dir = Self::default_commands_dir()?;

        Ok(Self { commands_dir })
    }

    /// Create installer with custom commands directory.
    pub fn with_dir(commands_dir: PathBuf) -> Self {
        Self { commands_dir }
    }

    /// Get default Claude commands directory.
    fn default_commands_dir() -> Result<PathBuf> {
        let home = dirs::home_dir()
            .ok_or_else(|| RagError::Config("Cannot determine home directory".to_string()))?;

        Ok(home.join(".claude").join("commands").join("rag"))
    }

    /// Install skills to Claude Code commands directory.
    pub fn install_skills(&self) -> Result<()> {
        // Create commands directory if it doesn't exist
        fs::create_dir_all(&self.commands_dir)
            .map_err(RagError::Io)?;

        // Install each skill (markdown format)
        self.install_skill("code.md", &self.generate_code_skill())?;
        self.install_skill("docs.md", &self.generate_docs_skill())?;
        self.install_skill("query.md", &self.generate_query_skill())?;
        self.install_skill("session.md", &self.generate_session_skill())?;
        self.install_skill("timeline.md", &self.generate_timeline_skill())?;

        Ok(())
    }

    /// Install a single skill markdown file.
    fn install_skill(&self, name: &str, content: &str) -> Result<()> {
        let skill_path = self.commands_dir.join(name);

        fs::write(&skill_path, content)
            .map_err(RagError::Io)?;

        Ok(())
    }

    /// Generate code search skill (markdown format).
    ///
    /// This skill searches source code content.
    pub fn generate_code_skill(&self) -> String {
        r#"---
description: 搜索项目源代码，基于语义理解查找相关函数、类和实现
allowed-tools: Bash(**), Read(**)
argument-hint: <搜索查询>
---

## 用法

`/rag:code <搜索查询>`

## 目标

使用 Claude RAG 语义搜索查找项目中的相关源代码，基于功能语义而非关键字匹配。

## 执行步骤

**步骤 1**：调用 claude-rag 进行源代码搜索。

```bash
export PATH="$HOME/.local/bin:$PATH"
claude-rag query "$ARGUMENTS" \
    --type source \
    --top-k 10 \
    --format markdown
```

**步骤 2**：解析并展示搜索结果，按以下格式输出：

### 输出格式

```markdown
## 🎯 源代码搜索结果

**查询**: $ARGUMENTS

### 结果 [N]
- **相似度**: XX%
- **文件**: `path/to/file.rs:行号`
- **类型**: 函数/类/方法
- **置信度**: 🟢当前 / 🔵Git / 🟡近期 / 🔴过时

代码片段和上下文说明...
```

## 注意事项

- 搜索结果基于语义相似度，可能包含功能相似但命名不同的代码
- 默认返回前 10 个最相关的结果
- 结果包含置信度标记，帮助判断代码是否为当前版本
"#.to_string()
    }

    /// Generate docs search skill (markdown format).
    ///
    /// This skill searches documentation content.
    pub fn generate_docs_skill(&self) -> String {
        r#"---
description: 搜索项目文档，包括 README、设计文档、注释等
allowed-tools: Bash(**), Read(**)
argument-hint: <搜索查询>
---

## 用法

`/rag:docs <搜索查询>`

## 目标

使用 Claude RAG 搜索项目中的文档内容，包括 README、设计文档、内联注释等。

## 执行步骤

**步骤 1**：调用 claude-rag 进行文档搜索。

```bash
export PATH="$HOME/.local/bin:$PATH"
claude-rag query "$ARGUMENTS" \
    --type docs \
    --top-k 10 \
    --format markdown
```

**步骤 2**：解析并展示搜索结果。

### 输出格式

```markdown
## 📚 文档搜索结果

**查询**: $ARGUMENTS

### 结果 [N]
- **相似度**: XX%
- **来源**: `path/to/doc.md`

文档内容片段和上下文说明...
```

## 注意事项

- 文档类型包括：README、DESIGN、API 文档、内联注释等
- 适合查找设计决策、使用说明、架构描述等
"#.to_string()
    }

    /// Generate query skill (markdown format).
    ///
    /// This skill queries all indexed content.
    pub fn generate_query_skill(&self) -> String {
        r#"---
description: 搜索全部内容（代码+文档+会话历史），综合查询项目知识库
allowed-tools: Bash(**), Read(**)
argument-hint: <搜索查询>
---

## 用法

`/rag:query <搜索查询>`

## 目标

使用 Claude RAG 搜索项目的全部知识内容，包括源代码、文档和会话历史。

## 执行步骤

**步骤 1**：调用 claude-rag 进行综合搜索。

```bash
export PATH="$HOME/.local/bin:$PATH"
claude-rag query "$ARGUMENTS" \
    --top-k 5 \
    --format markdown
```

**步骤 2**：解析并展示搜索结果，按类型分组。

### 输出格式

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

## 注意事项

- 综合搜索会返回所有类型的相关内容
- 结果按相似度排序
- 适合探索性查询，了解项目的整体情况
"#.to_string()
    }

    /// Generate session search skill (markdown format).
    ///
    /// This skill queries previous Claude sessions.
    pub fn generate_session_skill(&self) -> String {
        r#"---
description: 搜索 Claude Code 会话历史，查找之前讨论过的内容和决策
allowed-tools: Bash(**), Read(**)
argument-hint: <搜索查询>
---

## 用法

`/rag:session <搜索查询>`

## 目标

使用 Claude RAG 搜索项目的历史会话记录，查找之前讨论过的内容、决策和方案。

## 执行步骤

**步骤 1**：调用 claude-rag 进行会话历史搜索。

```bash
export PATH="$HOME/.local/bin:$PATH"
claude-rag query "$ARGUMENTS" \
    --type session \
    --top-k 10 \
    --format markdown
```

**步骤 2**：解析并展示搜索结果，包括讨论的时间戳和上下文。

### 输出格式

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

## 注意事项

- 会话历史包含用户输入和 AI 输出
- 结果会显示讨论的时间，便于判断内容的新旧
- 适合查找"之前是否讨论过这个问题"或"当时为什么这样决定"
"#.to_string()
    }

    /// Generate timeline skill (markdown format).
    ///
    /// This skill builds a feature timeline from git history and sessions.
    pub fn generate_timeline_skill(&self) -> String {
        r#"---
description: 按时间线展示功能的演进历史，包括 Git 提交和会话讨论
allowed-tools: Bash(**), Read(**)
argument-hint: <功能或主题>
---

## 用法

`/rag:timeline <功能或主题>`

## 目标

使用 Claude RAG 的时间线查询功能，展示某个功能或主题的演进历史，包括代码变更和讨论历史。

## 执行步骤

**步骤 1**：调用 claude-rag 进行时间线查询。

```bash
export PATH="$HOME/.local/bin:$PATH"
claude-rag query "$ARGUMENTS" \
    --timeline \
    --format markdown
```

**步骤 2**：解析并按时间顺序展示结果。

### 输出格式

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

## 注意事项

- 时间线查询会显示功能的完整演进过程
- 包含 Git 提交信息和会话讨论
- 按时间倒序排列，最新的在前
- 带有置信度颜色标记（绿色=当前代码，蓝色=Git提交，黄色=近期会话，红色=旧讨论）
"#.to_string()
    }

    /// Get the commands directory path.
    pub fn commands_dir(&self) -> &Path {
        &self.commands_dir
    }

    /// Check if skills are already installed.
    pub fn is_installed(&self) -> bool {
        self.commands_dir.join("code.md").exists()
    }
}

impl Default for SkillsInstaller {
    fn default() -> Self {
        Self::new().expect("Failed to create SkillsInstaller")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_skills_installer_new() {
        let installer = SkillsInstaller::new().unwrap();
        assert!(installer.commands_dir().ends_with(".claude/commands/rag"));
    }

    #[test]
    fn test_skills_installer_with_dir() {
        let temp_dir = TempDir::new().unwrap();
        let custom_dir = temp_dir.path().join("commands").join("rag");
        let installer = SkillsInstaller::with_dir(custom_dir.clone());

        assert_eq!(installer.commands_dir(), custom_dir);
    }

    #[test]
    fn test_generate_code_skill() {
        let installer = SkillsInstaller::new().unwrap();
        let skill = installer.generate_code_skill();

        assert!(skill.contains("---"));
        assert!(skill.contains("description:"));
        assert!(skill.contains("allowed-tools:"));
        assert!(skill.contains("--type source"));
        assert!(skill.contains("--top-k 10"));
    }

    #[test]
    fn test_generate_docs_skill() {
        let installer = SkillsInstaller::new().unwrap();
        let skill = installer.generate_docs_skill();

        assert!(skill.contains("allowed-tools:"));
        assert!(skill.contains("--type docs"));
    }

    #[test]
    fn test_generate_query_skill() {
        let installer = SkillsInstaller::new().unwrap();
        let skill = installer.generate_query_skill();

        assert!(skill.contains("allowed-tools:"));
        assert!(skill.contains("claude-rag query"));
        assert!(skill.contains("--top-k 5"));
    }

    #[test]
    fn test_generate_session_skill() {
        let installer = SkillsInstaller::new().unwrap();
        let skill = installer.generate_session_skill();

        assert!(skill.contains("allowed-tools:"));
        assert!(skill.contains("--type session"));
    }

    #[test]
    fn test_generate_timeline_skill() {
        let installer = SkillsInstaller::new().unwrap();
        let skill = installer.generate_timeline_skill();

        assert!(skill.contains("allowed-tools:"));
        assert!(skill.contains("--timeline"));
    }

    #[test]
    fn test_install_skills() {
        let temp_dir = TempDir::new().unwrap();
        let commands_dir = temp_dir.path().join("commands").join("rag");
        let installer = SkillsInstaller::with_dir(commands_dir.clone());

        installer.install_skills().unwrap();

        // Check that all skills were installed
        assert!(commands_dir.join("code.md").exists());
        assert!(commands_dir.join("docs.md").exists());
        assert!(commands_dir.join("query.md").exists());
        assert!(commands_dir.join("session.md").exists());
        assert!(commands_dir.join("timeline.md").exists());

        // Verify content of one skill
        let content = fs::read_to_string(commands_dir.join("code.md")).unwrap();
        assert!(content.contains("description:"));
        assert!(content.contains("--type source"));
    }

    #[test]
    fn test_is_installed() {
        let temp_dir = TempDir::new().unwrap();
        let commands_dir = temp_dir.path().join("commands").join("rag");
        let installer = SkillsInstaller::with_dir(commands_dir.clone());

        // Initially not installed
        assert!(!installer.is_installed());

        // After installation
        installer.install_skills().unwrap();
        assert!(installer.is_installed());
    }
}
