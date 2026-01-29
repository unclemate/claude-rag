//! 符号缓存系统
//!
//! 实现三级缓存架构，用于加速符号检索：
//!
//! ## 缓存层级
//!
//! - **L1 缓存**: 内存中的热数据，最快但容量最小（~100 项）
//! - **L2 缓存**: 符号级缓存，中等速度和容量（~1000 项）
//! - **L3 缓存**: 文件级缓存，最慢但容量最大（按需加载）
//!
//! ## 缓存策略
//!
//! - **LRU 淘汰**: 各层级使用 LRU 策略淘汰最少使用的项
//! - **分支隔离**: 不同分支的缓存完全隔离
//! - **一致性保证**: 符号更新时自动失效相关缓存
//!
//! ## 示例
//!
//! ```ignore
//! use crate::symbol_cache::SymbolCache;
//!
//! let cache = SymbolCache::new();
//!
//! // 存储符号
//! cache.put(symbol, "main")?;
//!
//! // 获取符号
//! if let Some(symbol) = cache.get("symbol:abc123", "main")? {
//!     println!("Found: {}", symbol.name);
//! }
//! ```

use crate::error::{Result, RagError};
use crate::models::Symbol;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, trace};

/// L1 缓存容量（内存中的热数据）
const L1_CAPACITY: usize = 100;

/// L2 缓存容量（符号级缓存）
const L2_CAPACITY: usize = 1000;

/// L3 缓存目录名称
const L3_CACHE_DIR: &str = "symbol_cache";

/// 缓存条目
#[derive(Debug, Clone, Serialize, Deserialize)]
struct CacheEntry {
    /// 符号数据
    symbol: Symbol,
    /// 访问时间戳（用于 LRU）
    last_access: chrono::DateTime<chrono::Utc>,
    /// 缓存键（用于验证）
    key: String,
}

impl Default for CacheEntry {
    fn default() -> Self {
        Self {
            symbol: Symbol {
                id: String::new(),
                file_id: String::new(),
                name: String::new(),
                kind: crate::models::SymbolKind::Function,
                start_line: 0,
                end_line: 0,
                doc_comment: None,
                code: String::new(),
                parent_id: None,
                branch_name: String::new(),
                last_commit_hash: None,
            },
            last_access: chrono::Utc::now(),
            key: String::new(),
        }
    }
}

/// L1 缓存 - 内存中的热数据
///
/// 最快的缓存层级，存储最常用的符号。
#[derive(Debug)]
struct L1Cache {
    /// 缓存条目
    entries: Arc<RwLock<HashMap<String, CacheEntry>>>,
    /// LRU 访问顺序（用于淘汰）
    access_order: Arc<RwLock<Vec<String>>>,
}

impl L1Cache {
    /// 创建新的 L1 缓存
    fn new() -> Self {
        Self {
            entries: Arc::new(RwLock::new(HashMap::new())),
            access_order: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// 获取缓存条目
    async fn get(&self, key: &str) -> Option<Symbol> {
        let mut entries = self.entries.write().await;
        let mut order = self.access_order.write().await;

        if let Some(entry) = entries.get(key) {
            // 先克隆符号（在修改 entry 之前）
            let symbol = entry.symbol.clone();

            // 更新访问时间和 LRU 顺序
            let mut updated = entry.clone();
            updated.last_access = chrono::Utc::now();

            // 更新 LRU 顺序
            order.retain(|k| k != key);
            let key_clone = key.to_string();
            order.push(key_clone.clone());

            // 直接插入（key 已存在，所以 insert 只是更新）
            entries.insert(key_clone, updated);

            trace!(key = %key, cache = "L1", "Cache hit");
            Some(symbol)
        } else {
            trace!(key = %key, cache = "L1", "Cache miss");
            None
        }
    }

    /// 存储缓存条目
    async fn put(&self, key: String, symbol: Symbol) {
        let mut entries = self.entries.write().await;
        let mut order = self.access_order.write().await;

        // 检查容量，必要时淘汰
        while entries.len() >= L1_CAPACITY {
            if let Some(old_key) = order.first() {
                let old_key = old_key.clone();
                order.remove(0);
                entries.remove(&old_key);
                trace!(key = %old_key, cache = "L1", "Evicted from L1");
            } else {
                break;
            }
        }

        let entry = CacheEntry {
            symbol,
            last_access: chrono::Utc::now(),
            key: key.clone(),
        };

        entries.insert(key.clone(), entry);
        order.push(key.clone());

        trace!(key = %key, cache = "L1", "Cached");
    }

    /// 清空缓存
    async fn clear(&self) {
        let mut entries = self.entries.write().await;
        let mut order = self.access_order.write().await;

        entries.clear();
        order.clear();

        debug!(cache = "L1", "Cache cleared");
    }

    /// 失效指定键
    async fn invalidate(&self, key: &str) {
        let mut entries = self.entries.write().await;
        let mut order = self.access_order.write().await;

        entries.remove(key);
        order.retain(|k| k != key);

        trace!(key = %key, cache = "L1", "Invalidated");
    }
}

/// L2 缓存 - 符号级缓存
///
/// 中等大小的缓存，存储更多符号数据。
#[derive(Debug)]
struct L2Cache {
    /// 缓存条目
    entries: Arc<RwLock<HashMap<String, CacheEntry>>>,
    /// LRU 访问顺序
    access_order: Arc<RwLock<Vec<String>>>,
}

impl L2Cache {
    /// 创建新的 L2 缓存
    fn new() -> Self {
        Self {
            entries: Arc::new(RwLock::new(HashMap::new())),
            access_order: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// 获取缓存条目
    async fn get(&self, key: &str) -> Option<Symbol> {
        let mut entries = self.entries.write().await;
        let mut order = self.access_order.write().await;

        if let Some(entry) = entries.get(key) {
            // 先克隆符号
            let symbol = entry.symbol.clone();

            // 更新访问时间
            let mut updated = entry.clone();
            updated.last_access = chrono::Utc::now();

            // 更新 LRU 顺序
            order.retain(|k| k != key);
            let key_clone = key.to_string();
            order.push(key_clone.clone());

            // 直接插入
            entries.insert(key_clone, updated);

            trace!(key = %key, cache = "L2", "Cache hit");
            Some(symbol)
        } else {
            trace!(key = %key, cache = "L2", "Cache miss");
            None
        }
    }

    /// 存储缓存条目
    async fn put(&self, key: String, symbol: Symbol) {
        let mut entries = self.entries.write().await;
        let mut order = self.access_order.write().await;

        while entries.len() >= L2_CAPACITY {
            if let Some(old_key) = order.first() {
                let old_key = old_key.clone();
                order.remove(0);
                entries.remove(&old_key);
                trace!(key = %old_key, cache = "L2", "Evicted from L2");
            } else {
                break;
            }
        }

        let entry = CacheEntry {
            symbol,
            last_access: chrono::Utc::now(),
            key: key.clone(),
        };

        entries.insert(key.clone(), entry);
        order.push(key.clone());

        trace!(key = %key, cache = "L2", "Cached");
    }

    /// 清空缓存
    async fn clear(&self) {
        let mut entries = self.entries.write().await;
        let mut order = self.access_order.write().await;

        entries.clear();
        order.clear();

        debug!(cache = "L2", "Cache cleared");
    }

    /// 失效指定键
    async fn invalidate(&self, key: &str) {
        let mut entries = self.entries.write().await;
        let mut order = self.access_order.write().await;

        entries.remove(key);
        order.retain(|k| k != key);

        trace!(key = %key, cache = "L2", "Invalidated");
    }
}

/// L3 缓存 - 文件级缓存（持久化到磁盘）
///
/// 慢但容量大，按需从磁盘加载。
#[derive(Debug)]
struct L3Cache {
    /// 项目路径
    project_path: PathBuf,
    /// 缓存目录路径
    cache_dir: PathBuf,
}

impl L3Cache {
    /// 创建新的 L3 缓存
    fn new(project_path: &Path) -> Result<Self> {
        let cache_dir = project_path.join(".rag").join(L3_CACHE_DIR);

        // 创建缓存目录
        std::fs::create_dir_all(&cache_dir)
            .map_err(|e| RagError::Io(e))?;

        Ok(Self {
            project_path: project_path.to_path_buf(),
            cache_dir,
        })
    }

    /// 获取缓存文件路径
    fn cache_file_path(&self, key: &str) -> PathBuf {
        use sha2::{Digest, Sha256};
        let hash = format!("{:x}", Sha256::digest(key.as_bytes()));
        self.cache_dir.join(format!("{}.json", hash))
    }

    /// 获取缓存条目
    async fn get(&self, key: &str) -> Option<Symbol> {
        let cache_path = self.cache_file_path(key);

        if !cache_path.exists() {
            trace!(key = %key, cache = "L3", "Cache miss (file not found)");
            return None;
        }

        match tokio::fs::read_to_string(&cache_path).await {
            Ok(content) => {
                match serde_json::from_str::<CacheEntry>(&content) {
                    Ok(entry) => {
                        trace!(key = %key, cache = "L3", "Cache hit");
                        Some(entry.symbol)
                    }
                    Err(e) => {
                        trace!(key = %key, cache = "L3", error = %e, "Failed to deserialize");
                        None
                    }
                }
            }
            Err(e) => {
                trace!(key = %key, cache = "L3", error = %e, "Failed to read cache file");
                None
            }
        }
    }

    /// 存储缓存条目
    async fn put(&self, key: String, symbol: Symbol) -> Result<()> {
        let cache_path = self.cache_file_path(&key);

        let entry = CacheEntry {
            symbol,
            last_access: chrono::Utc::now(),
            key: key.clone(),
        };

        let content = serde_json::to_string_pretty(&entry)?;

        tokio::fs::write(&cache_path, content)
            .await
            .map_err(|e| RagError::Io(e))?;

        trace!(key = %key, cache = "L3", "Cached to disk");

        Ok(())
    }

    /// 清空缓存（删除所有缓存文件）
    async fn clear(&self) -> Result<()> {
        let mut entries = tokio::fs::read_dir(&self.cache_dir).await
            .map_err(|e| RagError::Io(e))?;

        while let Some(entry) = entries.next_entry().await
            .map_err(|e| RagError::Io(e))? {
            let path = entry.path();

            if path.extension().and_then(|s| s.to_str()) == Some("json") {
                tokio::fs::remove_file(&path).await
                    .map_err(|e| RagError::Io(e))?;
            }
        }

        debug!(cache = "L3", "Cache cleared");

        Ok(())
    }

    /// 失效指定键
    async fn invalidate(&self, key: &str) -> Result<()> {
        let cache_path = self.cache_file_path(key);

        if cache_path.exists() {
            tokio::fs::remove_file(&cache_path).await
                .map_err(|e| RagError::Io(e))?;

            trace!(key = %key, cache = "L3", "Invalidated");
        }

        Ok(())
    }
}

/// 三级符号缓存系统
///
/// 整合 L1/L2/L3 缓存层级，提供统一的缓存接口。
pub struct SymbolCache {
    /// L1 缓存（内存热数据）
    l1: L1Cache,
    /// L2 缓存（符号级）
    l2: L2Cache,
    /// L3 缓存（文件级）
    l3: L3Cache,
}

impl SymbolCache {
    /// 创建新的符号缓存
    ///
    /// # 参数
    ///
    /// * `project_path` - 项目路径
    pub fn new(project_path: &Path) -> Result<Self> {
        let l3 = L3Cache::new(project_path)?;

        Ok(Self {
            l1: L1Cache::new(),
            l2: L2Cache::new(),
            l3,
        })
    }

    /// 创建分支感知的缓存键
    ///
    /// # 参数
    ///
    /// * `symbol_id` - 符号 ID
    /// * `branch` - Git 分支名称
    pub fn cache_key(&self, symbol_id: &str, branch: &str) -> String {
        format!("{}:{}", branch, symbol_id)
    }

    /// 获取符号（自动查找所有缓存层级）
    ///
    /// 查找顺序：L1 -> L2 -> L3
    ///
    /// # 参数
    ///
    /// * `symbol_id` - 符号 ID
    /// * `branch` - Git 分支名称
    pub async fn get(&self, symbol_id: &str, branch: &str) -> Result<Option<Symbol>> {
        let key = self.cache_key(symbol_id, branch);

        // L1 缓存
        if let Some(symbol) = self.l1.get(&key).await {
            return Ok(Some(symbol));
        }

        // L2 缓存
        if let Some(symbol) = self.l2.get(&key).await {
            // 提升到 L1
            self.l1.put(key.clone(), symbol.clone()).await;
            return Ok(Some(symbol));
        }

        // L3 缓存
        if let Some(symbol) = self.l3.get(&key).await {
            // 提升到 L1 和 L2
            self.l2.put(key.clone(), symbol.clone()).await;
            self.l1.put(key, symbol.clone()).await;
            return Ok(Some(symbol));
        }

        Ok(None)
    }

    /// 存储符号（写入所有缓存层级）
    ///
    /// # 参数
    ///
    /// * `symbol` - 符号数据
    /// * `branch` - Git 分支名称
    pub async fn put(&self, symbol: Symbol, branch: &str) -> Result<()> {
        let key = self.cache_key(&symbol.id, branch);

        // 存储到所有层级
        self.l1.put(key.clone(), symbol.clone()).await;
        self.l2.put(key.clone(), symbol.clone()).await;
        self.l3.put(key, symbol).await?;

        Ok(())
    }

    /// 失效符号缓存
    ///
    /// 从所有缓存层级中移除指定符号。
    ///
    /// # 参数
    ///
    /// * `symbol_id` - 符号 ID
    /// * `branch` - Git 分支名称
    pub async fn invalidate(&self, symbol_id: &str, branch: &str) -> Result<()> {
        let key = self.cache_key(symbol_id, branch);

        self.l1.invalidate(&key).await;
        self.l2.invalidate(&key).await;
        self.l3.invalidate(&key).await?;

        Ok(())
    }

    /// 失效分支的所有缓存
    ///
    /// 当分支切换时调用此方法清理旧分支缓存。
    ///
    /// # 参数
    ///
    /// * `branch` - Git 分支名称
    pub async fn invalidate_branch(&self, branch: &str) -> Result<()> {
        // 清空 L1 和 L2（包含所有分支数据）
        self.l1.clear().await;
        self.l2.clear().await;

        // L3 缓存需要按键过滤删除
        // 为简单起见，这里清空整个 L3 缓存
        self.l3.clear().await?;

        debug!(branch = %branch, "Branch cache invalidated");

        Ok(())
    }

    /// 清空所有缓存
    pub async fn clear_all(&self) -> Result<()> {
        self.l1.clear().await;
        self.l2.clear().await;
        self.l3.clear().await?;

        debug!("All caches cleared");

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_cache_key() {
        let temp_dir = TempDir::new().unwrap();
        let cache = SymbolCache::new(temp_dir.path()).unwrap();

        let key = cache.cache_key("symbol:abc123", "main");
        assert_eq!(key, "main:symbol:abc123");
    }

    #[tokio::test]
    async fn test_put_and_get() {
        let temp_dir = TempDir::new().unwrap();
        let cache = SymbolCache::new(temp_dir.path()).unwrap();

        let symbol = Symbol {
            id: "test-symbol".to_string(),
            file_id: "file-1".to_string(),
            name: "test_func".to_string(),
            kind: crate::models::SymbolKind::Function,
            start_line: 10,
            end_line: 20,
            doc_comment: None,
            code: "fn test_func() {}".to_string(),
            parent_id: None,
            branch_name: String::new(),
            last_commit_hash: None,
        };

        // 存储符号
        cache.put(symbol.clone(), "main").await.unwrap();

        // 获取符号
        let retrieved = cache.get("test-symbol", "main").await.unwrap();

        assert!(retrieved.is_some());
        let retrieved_symbol = retrieved.unwrap();
        assert_eq!(retrieved_symbol.id, "test-symbol");
        assert_eq!(retrieved_symbol.name, "test_func");
    }

    #[tokio::test]
    async fn test_cache_miss() {
        let temp_dir = TempDir::new().unwrap();
        let cache = SymbolCache::new(temp_dir.path()).unwrap();

        let result = cache.get("nonexistent", "main").await.unwrap();

        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_invalidate() {
        let temp_dir = TempDir::new().unwrap();
        let cache = SymbolCache::new(temp_dir.path()).unwrap();

        let symbol = Symbol {
            id: "test-symbol".to_string(),
            file_id: "file-1".to_string(),
            name: "test_func".to_string(),
            kind: crate::models::SymbolKind::Function,
            start_line: 10,
            end_line: 20,
            doc_comment: None,
            code: "fn test_func() {}".to_string(),
            parent_id: None,
            branch_name: String::new(),
            last_commit_hash: None,
        };

        // 存储然后失效
        cache.put(symbol, "main").await.unwrap();
        cache.invalidate("test-symbol", "main").await.unwrap();

        // 应该获取不到
        let result = cache.get("test-symbol", "main").await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_clear_all() {
        let temp_dir = TempDir::new().unwrap();
        let cache = SymbolCache::new(temp_dir.path()).unwrap();

        let symbol = Symbol {
            id: "test-symbol".to_string(),
            file_id: "file-1".to_string(),
            name: "test_func".to_string(),
            kind: crate::models::SymbolKind::Function,
            start_line: 10,
            end_line: 20,
            doc_comment: None,
            code: "fn test_func() {}".to_string(),
            parent_id: None,
            branch_name: String::new(),
            last_commit_hash: None,
        };

        cache.put(symbol, "main").await.unwrap();
        cache.clear_all().await.unwrap();

        let result = cache.get("test-symbol", "main").await.unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_l1_capacity() {
        assert_eq!(L1_CAPACITY, 100);
    }

    #[test]
    fn test_l2_capacity() {
        assert_eq!(L2_CAPACITY, 1000);
    }

    #[test]
    fn test_l3_cache_dir() {
        assert_eq!(L3_CACHE_DIR, "symbol_cache");
    }
}
