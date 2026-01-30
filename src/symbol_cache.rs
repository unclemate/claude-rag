//! Symbol cache system
//!
//! Implements a three-tier cache architecture for accelerating symbol retrieval:
//!
//! ## Cache Tiers
//!
//! - **L1 Cache**: Hot data in memory, fastest but smallest capacity (~100 items)
//! - **L2 Cache**: Symbol-level cache, medium speed and capacity (~1000 items)
//! - **L3 Cache**: File-level cache, slowest but largest capacity (on-demand loading)
//!
//! ## Cache Strategies
//!
//! - **LRU Eviction**: Each tier uses LRU strategy to evict least recently used items
//! - **Branch Isolation**: Caches for different branches are completely isolated
//! - **Consistency Guarantee**: Symbol updates automatically invalidate related caches
//!
//! ## Example
//!
//! ```ignore
//! use crate::symbol_cache::SymbolCache;
//!
//! let cache = SymbolCache::new();
//!
//! // Store symbol
//! cache.put(symbol, "main")?;
//!
//! // Get symbol
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

/// L1 cache capacity (hot data in memory)
const L1_CAPACITY: usize = 100;

/// L2 cache capacity (symbol-level cache)
const L2_CAPACITY: usize = 1000;

/// L3 cache directory name
const L3_CACHE_DIR: &str = "symbol_cache";

/// Cache entry
#[derive(Debug, Clone, Serialize, Deserialize)]
struct CacheEntry {
    /// Symbol data
    symbol: Symbol,
    /// Access timestamp (for LRU)
    last_access: chrono::DateTime<chrono::Utc>,
    /// Cache key (for validation)
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

/// L1 cache - hot data in memory
///
/// Fastest cache tier, storing most frequently used symbols.
#[derive(Debug)]
struct L1Cache {
    /// Cache entries
    entries: Arc<RwLock<HashMap<String, CacheEntry>>>,
    /// LRU access order (for eviction)
    access_order: Arc<RwLock<Vec<String>>>,
}

impl L1Cache {
    /// Create new L1 cache
    fn new() -> Self {
        Self {
            entries: Arc::new(RwLock::new(HashMap::new())),
            access_order: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Get cache entry
    async fn get(&self, key: &str) -> Option<Symbol> {
        let mut entries = self.entries.write().await;
        let mut order = self.access_order.write().await;

        if let Some(entry) = entries.get(key) {
            // Clone symbol first (before modifying entry)
            let symbol = entry.symbol.clone();

            // Update access time and LRU order
            let mut updated = entry.clone();
            updated.last_access = chrono::Utc::now();

            // Update LRU order
            order.retain(|k| k != key);
            let key_clone = key.to_string();
            order.push(key_clone.clone());

            // Insert directly (key exists, so insert just updates)
            entries.insert(key_clone, updated);

            trace!(key = %key, cache = "L1", "Cache hit");
            Some(symbol)
        } else {
            trace!(key = %key, cache = "L1", "Cache miss");
            None
        }
    }

    /// Store cache entry
    async fn put(&self, key: String, symbol: Symbol) {
        let mut entries = self.entries.write().await;
        let mut order = self.access_order.write().await;

        // Check capacity, evict if necessary
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

    /// Clear cache
    async fn clear(&self) {
        let mut entries = self.entries.write().await;
        let mut order = self.access_order.write().await;

        entries.clear();
        order.clear();

        debug!(cache = "L1", "Cache cleared");
    }

    /// Invalidate specific key
    async fn invalidate(&self, key: &str) {
        let mut entries = self.entries.write().await;
        let mut order = self.access_order.write().await;

        entries.remove(key);
        order.retain(|k| k != key);

        trace!(key = %key, cache = "L1", "Invalidated");
    }
}

/// L2 cache - symbol-level cache
///
/// Medium-sized cache, storing more symbol data.
#[derive(Debug)]
struct L2Cache {
    /// Cache entries
    entries: Arc<RwLock<HashMap<String, CacheEntry>>>,
    /// LRU access order
    access_order: Arc<RwLock<Vec<String>>>,
}

impl L2Cache {
    /// Create new L2 cache
    fn new() -> Self {
        Self {
            entries: Arc::new(RwLock::new(HashMap::new())),
            access_order: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Get cache entry
    async fn get(&self, key: &str) -> Option<Symbol> {
        let mut entries = self.entries.write().await;
        let mut order = self.access_order.write().await;

        if let Some(entry) = entries.get(key) {
            // Clone symbol first
            let symbol = entry.symbol.clone();

            // Update access time
            let mut updated = entry.clone();
            updated.last_access = chrono::Utc::now();

            // Update LRU order
            order.retain(|k| k != key);
            let key_clone = key.to_string();
            order.push(key_clone.clone());

            // Insert directly
            entries.insert(key_clone, updated);

            trace!(key = %key, cache = "L2", "Cache hit");
            Some(symbol)
        } else {
            trace!(key = %key, cache = "L2", "Cache miss");
            None
        }
    }

    /// Store cache entry
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

    /// Clear cache
    async fn clear(&self) {
        let mut entries = self.entries.write().await;
        let mut order = self.access_order.write().await;

        entries.clear();
        order.clear();

        debug!(cache = "L2", "Cache cleared");
    }

    /// Invalidate specific key
    async fn invalidate(&self, key: &str) {
        let mut entries = self.entries.write().await;
        let mut order = self.access_order.write().await;

        entries.remove(key);
        order.retain(|k| k != key);

        trace!(key = %key, cache = "L2", "Invalidated");
    }
}

/// L3 cache - file-level cache (persisted to disk)
///
/// Slow but large capacity, loaded from disk on demand.
#[derive(Debug)]
struct L3Cache {
    /// Project path (kept for future use)
    #[allow(dead_code)]
    project_path: PathBuf,
    /// Cache directory path
    cache_dir: PathBuf,
}

impl L3Cache {
    /// Create new L3 cache
    fn new(project_path: &Path) -> Result<Self> {
        let cache_dir = project_path.join(".rag").join(L3_CACHE_DIR);

        // Create cache directory
        std::fs::create_dir_all(&cache_dir)
            .map_err(|e| RagError::Io(e))?;

        Ok(Self {
            project_path: project_path.to_path_buf(),
            cache_dir,
        })
    }

    /// Get cache file path
    fn cache_file_path(&self, key: &str) -> PathBuf {
        use sha2::{Digest, Sha256};
        let hash = format!("{:x}", Sha256::digest(key.as_bytes()));
        self.cache_dir.join(format!("{}.json", hash))
    }

    /// Get cache entry
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

    /// Store cache entry
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

    /// Clear cache (delete all cache files)
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

    /// Invalidate specific key
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

/// Three-tier symbol cache system
///
/// Integrates L1/L2/L3 cache tiers, providing a unified cache interface.
pub struct SymbolCache {
    /// L1 cache (in-memory hot data)
    l1: L1Cache,
    /// L2 cache (symbol-level)
    l2: L2Cache,
    /// L3 cache (file-level)
    l3: L3Cache,
}

impl SymbolCache {
    /// Create new symbol cache
    ///
    /// # Arguments
    ///
    /// * `project_path` - Project path
    pub fn new(project_path: &Path) -> Result<Self> {
        let l3 = L3Cache::new(project_path)?;

        Ok(Self {
            l1: L1Cache::new(),
            l2: L2Cache::new(),
            l3,
        })
    }

    /// Create branch-aware cache key
    ///
    /// # Arguments
    ///
    /// * `symbol_id` - Symbol ID
    /// * `branch` - Git branch name
    pub fn cache_key(&self, symbol_id: &str, branch: &str) -> String {
        format!("{}:{}", branch, symbol_id)
    }

    /// Get symbol (search all cache tiers automatically)
    ///
    /// Search order: L1 -> L2 -> L3
    ///
    /// # Arguments
    ///
    /// * `symbol_id` - Symbol ID
    /// * `branch` - Git branch name
    pub async fn get(&self, symbol_id: &str, branch: &str) -> Result<Option<Symbol>> {
        let key = self.cache_key(symbol_id, branch);

        // L1 cache
        if let Some(symbol) = self.l1.get(&key).await {
            return Ok(Some(symbol));
        }

        // L2 cache
        if let Some(symbol) = self.l2.get(&key).await {
            // Promote to L1
            self.l1.put(key.clone(), symbol.clone()).await;
            return Ok(Some(symbol));
        }

        // L3 cache
        if let Some(symbol) = self.l3.get(&key).await {
            // Promote to L1 and L2
            self.l2.put(key.clone(), symbol.clone()).await;
            self.l1.put(key, symbol.clone()).await;
            return Ok(Some(symbol));
        }

        Ok(None)
    }

    /// Store symbol (write to all cache tiers)
    ///
    /// # Arguments
    ///
    /// * `symbol` - Symbol data
    /// * `branch` - Git branch name
    pub async fn put(&self, symbol: Symbol, branch: &str) -> Result<()> {
        let key = self.cache_key(&symbol.id, branch);

        // Store to all tiers
        self.l1.put(key.clone(), symbol.clone()).await;
        self.l2.put(key.clone(), symbol.clone()).await;
        self.l3.put(key, symbol).await?;

        Ok(())
    }

    /// Invalidate symbol cache
    ///
    /// Remove specified symbol from all cache tiers.
    ///
    /// # Arguments
    ///
    /// * `symbol_id` - Symbol ID
    /// * `branch` - Git branch name
    pub async fn invalidate(&self, symbol_id: &str, branch: &str) -> Result<()> {
        let key = self.cache_key(symbol_id, branch);

        self.l1.invalidate(&key).await;
        self.l2.invalidate(&key).await;
        self.l3.invalidate(&key).await?;

        Ok(())
    }

    /// Invalidate all cache for a branch
    ///
    /// Call this method when switching branches to clean up old branch cache.
    ///
    /// # Arguments
    ///
    /// * `branch` - Git branch name
    pub async fn invalidate_branch(&self, branch: &str) -> Result<()> {
        // Clear L1 and L2 (contains all branch data)
        self.l1.clear().await;
        self.l2.clear().await;

        // L3 cache needs filtered deletion by key
        // For simplicity, clear entire L3 cache here
        self.l3.clear().await?;

        debug!(branch = %branch, "Branch cache invalidated");

        Ok(())
    }

    /// Clear all caches
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

        // Store symbol
        cache.put(symbol.clone(), "main").await.unwrap();

        // Get symbol
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

        // Store then invalidate
        cache.put(symbol, "main").await.unwrap();
        cache.invalidate("test-symbol", "main").await.unwrap();

        // Should not get it
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
