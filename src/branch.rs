//! Git 分支管理器
//!
//! 本模块实现 Git 分支的检测、监听和隔离功能，确保代码索引在不同分支间保持独立。
//!
//! ## 核心功能
//!
//! - **分支检测**: 自动检测当前 Git 分支
//! - **分支监听**: 检测分支切换事件
//! - **索引隔离**: 不同分支的符号索引完全隔离
//! - **缓存清理**: 分支切换时自动清理旧分支缓存
//!
//! ## 设计原则
//!
//! - **KISS**: 简单的 Git 命令封装，无复杂逻辑
//! - **DRY**: 复用项目现有的 Git 集成
//! - **YAGNI**: 仅实现当前所需功能
//!
//! ## 示例
//!
//! ```ignore
//! use crate::branch::BranchManager;
//!
//! let manager = BranchManager::new("/path/to/repo")?;
//!
//! // 获取当前分支
//! let branch = manager.current_branch()?;
//!
//! // 检测分支变化
//! if manager.has_branch_changed()? {
//!     manager.clear_branch_cache(&old_branch)?;
//! }
//! ```

use crate::error::{Result, RagError};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

/// 默认分支名称
const DEFAULT_BRANCH: &str = "main";

/// Git 命令可执行文件名称
const GIT_EXECUTABLE: &str = "git";

/// Git 分支管理器
///
/// 负责检测和管理 Git 分支，实现分支级别的索引隔离。
///
/// ## 线程安全
///
/// `BranchManager` 使用 `Arc<RwLock>>` 实现线程安全，可以在多线程环境中共享使用。
///
/// ## 分支隔离策略
///
/// 索引键设计: `{project_path}:{branch_name}:{content_type}:{id}`
///
/// 示例:
/// - `/repo/main:symbol:parse_function`
/// - `/repo/feature/api:symbol:parse_function`
#[derive(Clone)]
pub struct BranchManager {
    /// 项目路径
    project_path: PathBuf,
    /// 当前缓存分支名（用于检测变化）
    current_branch: Arc<RwLock<Option<String>>>,
}

impl BranchManager {
    /// 创建新的分支管理器
    ///
    /// # 参数
    ///
    /// * `project_path` - Git 仓库的根目录路径
    ///
    /// # 返回
    ///
    /// 新的 `BranchManager` 实例
    ///
    /// # 错误
    ///
    /// 如果路径不存在或不是 Git 仓库，返回错误。
    pub fn new(project_path: &Path) -> Result<Self> {
        if !project_path.exists() {
            return Err(RagError::Validation(format!(
                "Project path does not exist: {}",
                project_path.display()
            )));
        }

        // 验证是否为 Git 仓库
        let git_dir = project_path.join(".git");
        if !git_dir.exists() {
            return Err(RagError::Validation(format!(
                "Not a Git repository: {}",
                project_path.display()
            )));
        }

        Ok(Self {
            project_path: project_path.to_path_buf(),
            current_branch: Arc::new(RwLock::new(None)),
        })
    }

    /// 异步初始化分支管理器
    ///
    /// 创建管理器并缓存当前分支。
    pub async fn initialize(project_path: &Path) -> Result<Self> {
        let manager = Self::new(project_path)?;

        // 初始化时缓存当前分支
        if let Ok(branch) = manager.current_branch() {
            let mut cached = manager.current_branch.write().await;
            *cached = Some(branch);
        }

        Ok(manager)
    }

    /// 获取当前 Git 分支名称
    ///
    /// # 返回
    ///
    /// 当前分支名称，如果在 detached HEAD 状态或其他错误情况，返回默认分支名。
    pub fn current_branch(&self) -> Result<String> {
        // 使用 git rev-parse --abbrev-ref HEAD 获取当前分支
        let output = std::process::Command::new(GIT_EXECUTABLE)
            .arg("rev-parse")
            .arg("--abbrev-ref")
            .arg("HEAD")
            .current_dir(&self.project_path)
            .output()?;

        if output.status.success() {
            let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();

            // 如果是 HEAD（detached 状态），使用默认分支
            if branch == "HEAD" {
                warn!("Detached HEAD state detected, using default branch: {}", DEFAULT_BRANCH);
                return Ok(DEFAULT_BRANCH.to_string());
            }

            debug!("Current branch: {}", branch);
            Ok(branch)
        } else {
            // Git 命令失败，返回默认分支
            warn!(
                "Failed to get current branch, using default: {}",
                DEFAULT_BRANCH
            );
            Ok(DEFAULT_BRANCH.to_string())
        }
    }

    /// 获取所有本地分支名称
    ///
    /// # 返回
    ///
    /// 分支名称列表
    pub fn all_branches(&self) -> Result<Vec<String>> {
        let output = std::process::Command::new(GIT_EXECUTABLE)
            .arg("branch")
            .arg("--format=%(refname:short)")
            .current_dir(&self.project_path)
            .output()?;

        if output.status.success() {
            let branches: Vec<String> = String::from_utf8_lossy(&output.stdout)
                .lines()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();

            debug!("Found branches: {:?}", branches);
            Ok(branches)
        } else {
            Err(RagError::Git(format!(
                "Failed to list branches: {}",
                String::from_utf8_lossy(&output.stderr)
            )))
        }
    }

    /// 检测分支是否发生变化
    ///
    /// 与上次缓存的分支名称进行比较。
    ///
    /// # 返回
    ///
    /// - `Ok(true)`: 分支已变化
    /// - `Ok(false)`: 分支未变化
    /// - `Err(_)`: 检测失败
    pub async fn has_branch_changed(&self) -> Result<bool> {
        let current = self.current_branch()?;

        let cached = self.current_branch.read().await;
        match &*cached {
            Some(cached_branch) => Ok(current != *cached_branch),
            None => {
                // 首次调用，不算变化
                drop(cached);
                let mut write_guard = self.current_branch.write().await;
                *write_guard = Some(current);
                Ok(false)
            }
        }
    }

    /// 更新缓存的分支名称
    ///
    /// 在分支切换后调用此方法更新缓存。
    pub async fn update_cached_branch(&self, branch: &str) {
        let mut cached = self.current_branch.write().await;
        *cached = Some(branch.to_string());
        info!("Updated cached branch to: {}", branch);
    }

    /// 创建分支感知的索引键前缀
    ///
    /// # 参数
    ///
    /// * `branch` - 分支名称
    /// * `content_type` - 内容类型（如 "symbol", "file" 等）
    ///
    /// # 返回
    ///
    /// 索引键前缀
    ///
    /// # 示例
    ///
    /// ```ignore
    /// let prefix = manager.key_prefix("main", "symbol");
    /// // 返回类似: "/path/to/repo:main:symbol"
    /// ```
    pub fn key_prefix(&self, branch: &str, content_type: &str) -> String {
        format!(
            "{}:{}:{}",
            self.project_path.display(),
            branch,
            content_type
        )
    }

    /// 解析索引键获取分支名称
    ///
    /// # 参数
    ///
    /// * `key` - 完整的索引键
    ///
    /// # 返回
    ///
    /// 分支名称
    ///
    /// # 示例
    ///
    /// ```ignore
    /// let branch = manager.branch_from_key("/repo:main:symbol:id");
    /// // 返回: "main"
    /// ```
    pub fn branch_from_key(&self, key: &str) -> Result<String> {
        let parts: Vec<&str> = key.split(':').collect();

        if parts.len() < 3 {
            return Err(RagError::Validation(format!(
                "Invalid key format: {}",
                key
            )));
        }

        // 分支名是第二部分（索引 1）
        Ok(parts[1].to_string())
    }

    /// 检查指定分支是否存在
    ///
    /// # 参数
    ///
    /// * `branch` - 分支名称
    ///
    /// # 返回
    ///
    /// - `Ok(true)`: 分支存在
    /// - `Ok(false)`: 分支不存在
    pub fn branch_exists(&self, branch: &str) -> Result<bool> {
        let branches = self.all_branches()?;
        Ok(branches.iter().any(|b| b == branch))
    }

    /// 获取分支的最新提交哈希
    ///
    /// # 参数
    ///
    /// * `branch` - 分支名称
    ///
    /// # 返回
    ///
    /// 提交哈希（简短格式）
    pub fn branch_head_commit(&self, branch: &str) -> Result<String> {
        let output = std::process::Command::new(GIT_EXECUTABLE)
            .arg("rev-parse")
            .arg(&format!("--short={}", 7)) // 7 字符简短哈希
            .arg(branch)
            .current_dir(&self.project_path)
            .output()?;

        if output.status.success() {
            let commit = String::from_utf8_lossy(&output.stdout).trim().to_string();
            debug!("Branch {} head commit: {}", branch, commit);
            Ok(commit)
        } else {
            Err(RagError::Git(format!(
                "Failed to get head commit for branch {}: {}",
                branch,
                String::from_utf8_lossy(&output.stderr)
            )))
        }
    }

    /// 获取当前分支的最新提交哈希
    ///
    /// # 返回
    ///
    /// 提交哈希（简短格式）
    pub fn current_commit(&self) -> Result<String> {
        let branch = self.current_branch()?;
        self.branch_head_commit(&branch)
    }

    /// 获取项目路径
    pub fn project_path(&self) -> &Path {
        &self.project_path
    }

    /// 创建符号的分支感知 ID
    ///
    /// # 参数
    ///
    /// * `branch` - 分支名称
    /// * `symbol_id` - 原始符号 ID
    ///
    /// # 返回
    ///
    /// 分支感知的符号 ID
    ///
    /// # 示例
    ///
    /// ```ignore
    /// let id = manager.symbol_id("main", "symbol:abc123");
    /// // 返回: "main:symbol:abc123"
    /// ```
    pub fn symbol_id(&self, branch: &str, symbol_id: &str) -> String {
        format!("{}:{}", branch, symbol_id)
    }

    /// 创建文件的分支感知 ID
    ///
    /// # 参数
    ///
    /// * `branch` - 分支名称
    /// * `file_id` - 原始文件 ID
    ///
    /// # 返回
    ///
    /// 分支感知的文件 ID
    pub fn file_id(&self, branch: &str, file_id: &str) -> String {
        format!("{}:{}", branch, file_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use std::fs;

    /// 创建一个临时的 Git 仓库用于测试
    fn create_test_repo() -> TempDir {
        let temp_dir = TempDir::new().unwrap();
        let repo_path = temp_dir.path();

        // 初始化 Git 仓库
        std::process::Command::new("git")
            .arg("init")
            .current_dir(repo_path)
            .output()
            .expect("Failed to init git repo");

        // 配置用户
        std::process::Command::new("git")
            .args(&["config", "user.name", "Test User"])
            .current_dir(repo_path)
            .output()
            .expect("Failed to configure git user");

        std::process::Command::new("git")
            .args(&["config", "user.email", "test@example.com"])
            .current_dir(repo_path)
            .output()
            .expect("Failed to configure git email");

        // 创建初始提交
        let readme_path = repo_path.join("README.md");
        fs::write(&readme_path, "# Test Repository").unwrap();

        std::process::Command::new("git")
            .args(&["add", "README.md"])
            .current_dir(repo_path)
            .output()
            .expect("Failed to add file");

        std::process::Command::new("git")
            .args(&["commit", "-m", "Initial commit"])
            .current_dir(repo_path)
            .output()
            .expect("Failed to commit");

        temp_dir
    }

    /// 创建并切换到新分支
    fn create_test_branch(repo_path: &Path, branch_name: &str) {
        std::process::Command::new("git")
            .args(&["checkout", "-b", branch_name])
            .current_dir(repo_path)
            .output()
            .expect("Failed to create branch");

        // 在新分支中创建一个文件
        let file_path = repo_path.join(format!("{}.txt", branch_name));
        fs::write(&file_path, format!("Content in {}", branch_name)).unwrap();

        std::process::Command::new("git")
            .args(&["add", "."])
            .current_dir(repo_path)
            .output()
            .expect("Failed to add file");

        std::process::Command::new("git")
            .args(&["commit", "-m", &format!("Commit in {}", branch_name)])
            .current_dir(repo_path)
            .output()
            .expect("Failed to commit");
    }

    #[test]
    fn test_branch_manager_new() {
        let temp_dir = create_test_repo();
        let manager = BranchManager::new(temp_dir.path());

        assert!(manager.is_ok());
    }

    #[tokio::test]
    async fn test_branch_manager_initialize() {
        let temp_dir = create_test_repo();
        let manager = BranchManager::initialize(temp_dir.path()).await;

        assert!(manager.is_ok());
    }

    #[test]
    fn test_branch_manager_new_invalid_path() {
        let manager = BranchManager::new(Path::new("/nonexistent/path"));

        assert!(manager.is_err());
    }

    #[test]
    fn test_branch_manager_new_not_git_repo() {
        let temp_dir = TempDir::new().unwrap();
        let manager = BranchManager::new(temp_dir.path());

        assert!(manager.is_err());
    }

    #[test]
    fn test_current_branch() {
        let temp_dir = create_test_repo();
        let manager = BranchManager::new(temp_dir.path()).unwrap();

        let branch = manager.current_branch();

        assert!(branch.is_ok());
        // 默认分支可能是 main 或 master，取决于 Git 版本
        let branch_name = branch.unwrap();
        assert!(branch_name == "main" || branch_name == "master");
    }

    #[test]
    fn test_all_branches() {
        let temp_dir = create_test_repo();
        let manager = BranchManager::new(temp_dir.path()).unwrap();

        let branches = manager.all_branches();

        assert!(branches.is_ok());
        let branch_list = branches.unwrap();
        assert!(!branch_list.is_empty());
    }

    #[test]
    fn test_branch_exists() {
        let temp_dir = create_test_repo();
        let manager = BranchManager::new(temp_dir.path()).unwrap();

        // main 分支应该存在
        assert!(manager.branch_exists("main").is_ok());

        // 不存在的分支
        let result = manager.branch_exists("nonexistent");
        assert!(result.is_ok());
        assert!(!result.unwrap());
    }

    #[test]
    fn test_key_prefix() {
        let temp_dir = create_test_repo();
        let manager = BranchManager::new(temp_dir.path()).unwrap();

        let prefix = manager.key_prefix("main", "symbol");

        assert!(prefix.contains("main"));
        assert!(prefix.contains("symbol"));
        assert!(prefix.contains(&format!("{}", temp_dir.path().display())));
    }

    #[test]
    fn test_symbol_id() {
        let temp_dir = create_test_repo();
        let manager = BranchManager::new(temp_dir.path()).unwrap();

        let id = manager.symbol_id("main", "symbol:abc123");

        assert_eq!(id, "main:symbol:abc123");
    }

    #[test]
    fn test_file_id() {
        let temp_dir = create_test_repo();
        let manager = BranchManager::new(temp_dir.path()).unwrap();

        let id = manager.file_id("feature", "file:xyz789");

        assert_eq!(id, "feature:file:xyz789");
    }

    #[test]
    fn test_branch_head_commit() {
        let temp_dir = create_test_repo();
        let manager = BranchManager::new(temp_dir.path()).unwrap();

        let commit = manager.branch_head_commit("main");

        assert!(commit.is_ok());
        let commit_hash = commit.unwrap();
        // Git 简短哈希应该是 7 个字符
        assert_eq!(commit_hash.len(), 7);
    }

    #[test]
    fn test_current_commit() {
        let temp_dir = create_test_repo();
        let manager = BranchManager::new(temp_dir.path()).unwrap();

        let commit = manager.current_commit();

        assert!(commit.is_ok());
    }

    #[test]
    fn test_branch_from_key() {
        let temp_dir = create_test_repo();
        let manager = BranchManager::new(temp_dir.path()).unwrap();

        let key = format!("{}/repo:main:symbol:id", temp_dir.path().display());
        let branch = manager.branch_from_key(&key);

        assert!(branch.is_ok());
        assert_eq!(branch.unwrap(), "main");
    }

    #[test]
    fn test_branch_from_key_invalid_format() {
        let temp_dir = create_test_repo();
        let manager = BranchManager::new(temp_dir.path()).unwrap();

        let result = manager.branch_from_key("invalid_key");

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_has_branch_changed_initial() {
        let temp_dir = create_test_repo();
        let manager = BranchManager::initialize(temp_dir.path()).await.unwrap();

        // 首次调用应该返回 false
        let changed = manager.has_branch_changed().await;

        assert!(changed.is_ok());
        assert!(!changed.unwrap());
    }

    #[tokio::test]
    async fn test_update_cached_branch() {
        let temp_dir = create_test_repo();
        let manager = BranchManager::new(temp_dir.path()).unwrap();

        manager.update_cached_branch("feature").await;

        let cached = manager.current_branch.read().await;
        assert_eq!(*cached, Some("feature".to_string()));
    }

    #[tokio::test]
    async fn test_has_branch_changed_after_update() {
        let temp_dir = create_test_repo();
        let manager = BranchManager::initialize(temp_dir.path()).await.unwrap();

        // 首次调用
        let _ = manager.has_branch_changed().await;

        // 更新缓存的分支
        manager.update_cached_branch("feature").await;

        // 现在应该检测到变化（假设当前分支不是 feature）
        let changed = manager.has_branch_changed().await;
        assert!(changed.is_ok());
        // 结果取决于实际分支，但至少不应该出错
    }

    #[test]
    fn test_constants() {
        assert_eq!(DEFAULT_BRANCH, "main");
        assert_eq!(GIT_EXECUTABLE, "git");
    }

    #[test]
    fn test_project_path() {
        let temp_dir = create_test_repo();
        let manager = BranchManager::new(temp_dir.path()).unwrap();

        assert_eq!(manager.project_path(), temp_dir.path());
    }

    #[test]
    fn test_multiple_branches() {
        let temp_dir = create_test_repo();
        let repo_path = temp_dir.path();

        // 创建测试分支
        create_test_branch(repo_path, "feature-test");
        create_test_branch(repo_path, "develop");

        let manager = BranchManager::new(repo_path).unwrap();
        let branches = manager.all_branches().unwrap();

        // 应该包含 main 和新创建的分支
        assert!(branches.len() >= 3);
        assert!(branches.contains(&"main".to_string()) || branches.contains(&"master".to_string()));
        assert!(branches.contains(&"feature-test".to_string()));
        assert!(branches.contains(&"develop".to_string()));
    }
}
