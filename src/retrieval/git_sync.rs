//! Git status synchronization module.
//!
//! This module provides functionality to check whether indexed content
//! matches the current Git HEAD, enabling accurate `is_current` and
//! `is_deprecated` marking in query results.

use crate::error::{RagError, Result};
use chrono::{DateTime, Utc};
use git2::{Repository, Status};
use lru::LruCache;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tracing::{debug, info};

/// Git synchronization status for a file or symbol.
#[derive(Debug, Clone, PartialEq)]
pub enum GitSyncStatus {
    /// Content matches Git HEAD (current).
    Current,
    /// Content differs from Git HEAD (deprecated).
    ///
    /// Indicates that the indexed content has been modified, deleted, or staged
    /// since it was last indexed, and may not reflect the current state of the codebase.
    ///
    /// # Fields
    ///
    /// * `reason` - Human-readable description of why the content is deprecated
    ///   (e.g., "Modified since last commit", "File deleted", "Staged changes")
    Deprecated {
        /// Description of why the content is deprecated
        reason: String,
    },
    /// Not applicable (non-Git repository or untracked file).
    ///
    /// Returned when:
    /// - The project is not a Git repository
    /// - The file is not tracked by Git
    /// - Git status cannot be determined
    NotApplicable,
}

impl GitSyncStatus {
    /// Returns true if the content is current.
    pub fn is_current(&self) -> bool {
        matches!(self, Self::Current)
    }
}

/// Maximum file size for hash computation (10 MB).
///
/// Files larger than this will skip hash caching to avoid blocking
/// the thread pool on large file reads.
const MAX_HASH_SIZE: usize = 10 * 1024 * 1024;

/// Cache entry for Git status checks.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct GitStatusEntry {
    /// HEAD commit hash when cached.
    head_hash: String,
    /// Whether the file is current (matches Git HEAD).
    is_current: bool,
    /// When the cache entry was created.
    cached_at: DateTime<Utc>,
    /// File content hash (SHA-256) for detecting modifications.
    ///
    /// When the file hash matches the current file content, we can safely
    /// return the cached status without re-checking Git.
    file_hash: Option<String>,
    /// Original deprecation reason (only set for Deprecated status).
    ///
    /// This preserves the original reason when returning cached Deprecated
    /// status, providing accurate feedback to users.
    reason: Option<String>,
}

impl Default for GitStatusEntry {
    fn default() -> Self {
        Self {
            head_hash: String::new(),
            is_current: false,
            cached_at: Utc::now(),
            file_hash: None,
            reason: None,
        }
    }
}

/// Persistent cache for Git sync status.
///
/// Stored to disk as JSON and loaded on startup to preserve cache
/// across program restarts.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct GitSyncCache {
    /// Cache version for future migrations.
    version: u32,
    /// HEAD commit hash when cache was created.
    head_hash: String,
    /// Cache entries (file_path -> entry).
    entries: HashMap<String, GitStatusEntry>,
    /// Maximum number of entries to keep in persistent cache.
    max_entries: usize,
    /// When cache was last updated.
    updated_at: DateTime<Utc>,
}

impl Default for GitSyncCache {
    fn default() -> Self {
        Self {
            version: 1,
            head_hash: String::new(),
            entries: HashMap::new(),
            max_entries: 10000, // Default maximum entries for persistent cache
            updated_at: Utc::now(),
        }
    }
}

impl GitSyncCache {
    /// Clean up old entries if the cache exceeds max_entries.
    /// Removes oldest entries based on cached_at timestamp.
    fn cleanup_old_entries(&mut self) {
        if self.entries.len() <= self.max_entries {
            return;
        }

        // Collect entries with their cached_at times
        let mut entries_with_time: Vec<_> = self.entries
            .iter()
            .map(|(path, entry)| (entry.cached_at, path.clone()))
            .collect();

        // Sort by cached_at (oldest first)
        entries_with_time.sort_by_key(|(time, _)| *time);

        // Remove oldest entries
        let to_remove = self.entries.len() - self.max_entries;
        for (_, path) in entries_with_time.iter().take(to_remove) {
            self.entries.remove(path);
        }

        tracing::debug!(
            "Cleaned up {} old cache entries, now have {} entries",
            to_remove,
            self.entries.len()
        );
    }
}

/// Default cache capacity (maximum number of entries).
const DEFAULT_CACHE_CAPACITY: usize = 1000;

/// Git status synchronizer with caching support.
///
/// Provides async API for checking file status against Git HEAD
/// using `tokio::task::spawn_blocking` to avoid blocking async tasks.
///
/// ## Cache Architecture
///
/// Two-tier caching system:
/// - **L1 (Memory)**: LRU cache for fast access to hot data
/// - **L2 (Disk)**: Persistent cache that survives program restarts
pub struct GitSync {
    /// Project path (repository root).
    project_path: std::path::PathBuf,
    /// Git repository (optional for non-Git projects).
    repo: Option<Repository>,
    /// L1: Memory LRU cache for hot data (fast access).
    memory_cache: Arc<RwLock<LruCache<String, GitStatusEntry>>>,
    /// L2: Persistent disk cache for cold data (survives restarts).
    persistent_cache: Arc<RwLock<GitSyncCache>>,
    /// Cache TTL in seconds.
    cache_ttl_seconds: u64,
    /// Current HEAD commit hash (cached).
    head_hash: Arc<RwLock<Option<String>>>,
    /// Dirty flag indicating cache needs persistence.
    dirty: Arc<AtomicBool>,
    /// Interval for persisting cache to disk.
    persist_interval: Duration,
    /// Handle for the background persistence task.
    _persist_handle: Option<tokio::task::JoinHandle::<()>>,
}

impl GitSync {
    /// Create a new GitSync instance.
    ///
    /// # Arguments
    ///
    /// * `project_path` - Path to the project directory
    /// * `cache_ttl_seconds` - Cache TTL in seconds
    ///
    /// # Returns
    ///
    /// Returns `Ok(GitSync)` if successful, `Err` if the path is invalid.
    pub fn new(project_path: &Path, cache_ttl_seconds: u64) -> Result<Self> {
        Self::with_capacity(project_path, cache_ttl_seconds, DEFAULT_CACHE_CAPACITY)
    }

    /// Create a new GitSync instance with custom cache capacity.
    ///
    /// # Arguments
    ///
    /// * `project_path` - Path to the project directory
    /// * `cache_ttl_seconds` - Cache TTL in seconds
    /// * `cache_capacity` - Maximum number of memory cache entries
    ///
    /// # Returns
    ///
    /// Returns `Ok(GitSync)` if successful, `Err` if the path is invalid.
    pub fn with_capacity(project_path: &Path, cache_ttl_seconds: u64, cache_capacity: usize) -> Result<Self> {
        Self::with_options(project_path, cache_ttl_seconds, cache_capacity, true)
    }

    /// Create a new GitSync instance with full options.
    ///
    /// # Arguments
    ///
    /// * `project_path` - Path to the project directory
    /// * `cache_ttl_seconds` - Cache TTL in seconds
    /// * `cache_capacity` - Maximum number of memory cache entries
    /// * `enable_persistence` - Whether to enable persistent caching
    ///
    /// # Returns
    ///
    /// Returns `Ok(GitSync)` if successful, `Err` if the path is invalid.
    pub fn with_options(
        project_path: &Path,
        cache_ttl_seconds: u64,
        cache_capacity: usize,
        enable_persistence: bool,
    ) -> Result<Self> {
        let project_path = if project_path.is_absolute() {
            project_path.to_path_buf()
        } else {
            std::env::current_dir()?.join(project_path)
        };

        // Try to open as Git repository (fail gracefully)
        let repo = match Repository::discover(&project_path) {
            Ok(r) => Some(r),
            Err(_) => None,
        };

        let capacity = NonZeroUsize::new(cache_capacity)
            .unwrap_or(NonZeroUsize::new(DEFAULT_CACHE_CAPACITY).unwrap());

        let persist_interval = Duration::from_secs(300); // 5 minutes

        let mut sync = Self {
            project_path,
            repo,
            memory_cache: Arc::new(RwLock::new(LruCache::new(capacity))),
            persistent_cache: Arc::new(RwLock::new(GitSyncCache::default())),
            cache_ttl_seconds,
            head_hash: Arc::new(RwLock::new(None)),
            dirty: Arc::new(AtomicBool::new(false)),
            persist_interval,
            _persist_handle: None,
        };

        // Load persistent cache if enabled
        if enable_persistence {
            // Try to load existing cache
            if let Err(e) = sync.load_persistent_cache_sync() {
                tracing::debug!("Failed to load persistent cache: {}", e);
            }

            // Start background persistence task only if we're in a tokio runtime
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                let project_path_clone = sync.project_path.clone();
                let persistent_cache_clone = sync.persistent_cache.clone();
                let dirty_clone = sync.dirty.clone();
                let interval = sync.persist_interval;

                let persist_handle = handle.spawn(async move {
                    Self::persist_task(project_path_clone, persistent_cache_clone, dirty_clone, interval).await;
                });

                sync._persist_handle = Some(persist_handle);
            } else {
                tracing::debug!("No tokio runtime available, skipping background persistence task");
            }
        }

        Ok(sync)
    }

    /// Check if a single file's content matches Git HEAD.
    ///
    /// # Arguments
    ///
    /// * `file_path` - Relative path to the file
    ///
    /// # Returns
    ///
    /// Returns the Git sync status for the file.
    pub async fn check_file_sync(&self, file_path: &str) -> Result<GitSyncStatus> {
        let results = self.batch_check_files(&[file_path.to_string()]).await?;
        Ok(results.get(file_path).cloned().unwrap_or(GitSyncStatus::NotApplicable))
    }

    /// Check multiple files' Git status in a single batch operation.
    ///
    /// This is more efficient than calling `check_file_sync` multiple times
    /// as it avoids creating multiple tokio runtimes.
    ///
    /// # Arguments
    ///
    /// * `file_paths` - Slice of file paths to check
    ///
    /// # Returns
    ///
    /// Returns a HashMap mapping file paths to their Git status.
    pub async fn batch_check_files(&self, file_paths: &[String]) -> Result<HashMap<String, GitSyncStatus>> {
        if file_paths.is_empty() {
            return Ok(HashMap::new());
        }

        info!("Batch checking {} files for Git sync status", file_paths.len());

        // Check if we have a Git repository
        if self.repo.is_none() {
            debug!("No Git repository found, marking all files as NotApplicable");
            let mut results = HashMap::new();
            for path in file_paths {
                results.insert(path.clone(), GitSyncStatus::NotApplicable);
            }
            return Ok(results);
        }

        // Spawn blocking task for Git operations
        let memory_cache = self.memory_cache.clone();
        let persistent_cache = self.persistent_cache.clone();
        let dirty = self.dirty.clone();
        let head_hash_ref = self.head_hash.clone();
        let cache_ttl = self.cache_ttl_seconds;
        let project_path = self.project_path.clone();
        let paths = file_paths.to_vec();

        tokio::task::spawn_blocking(move || {
            Self::batch_check_internal(&project_path, &paths, memory_cache, persistent_cache, dirty, head_hash_ref, cache_ttl)
        })
        .await
        .map_err(|e| RagError::Git(format!("Join error: {}", e)))?
    }

    /// Check if a symbol's content matches Git HEAD.
    ///
    /// Symbols inherit their Git status from the file they belong to.
    ///
    /// # Arguments
    ///
    /// * `file_path` - Path to the file containing the symbol
    ///
    /// # Returns
    ///
    /// Returns the Git sync status for the symbol.
    pub async fn check_symbol_sync(&self, file_path: &str) -> Result<GitSyncStatus> {
        // Symbols inherit the file's status
        self.check_file_sync(file_path).await
    }

    /// Clear all cached status entries.
    pub async fn clear_cache(&self) {
        self.memory_cache.write().await.clear();
        *self.head_hash.write().await = None;
    }

    /// Invalidate cache for a specific file.
    pub async fn invalidate_file(&self, file_path: &str) {
        self.memory_cache.write().await.pop(file_path);
    }

    /// Manually flush persistent cache to disk.
    ///
    /// This can be called to ensure cache is persisted before program exit.
    pub async fn flush(&self) -> Result<()> {
        self.flush_persistent_cache().await
    }

    // ========================================================================
    // Persistent cache methods
    // ========================================================================

    /// Get the path to the persistent cache file.
    fn cache_path(&self) -> PathBuf {
        self.project_path.join(".rag/git_sync_cache.json")
    }

    /// Load persistent cache from disk (synchronous version).
    fn load_persistent_cache_sync(&mut self) -> Result<()> {
        let path = self.cache_path();
        if !path.exists() {
            tracing::debug!("No persistent cache found at {:?}", path);
            return Ok(());
        }

        let data = fs::read(&path).map_err(|e| {
            RagError::Git(format!("Failed to read cache file: {}", e))
        })?;

        let cache: GitSyncCache = serde_json::from_slice(&data).map_err(|e| {
            RagError::Git(format!("Failed to parse cache: {}", e))
        })?;

        // Validate cache version
        if cache.version != 1 {
            tracing::warn!("Unsupported cache version {}, ignoring", cache.version);
            return Ok(());
        }

        // Replace the default cache with loaded data
        *self.persistent_cache.try_write().map_err(|e| {
            RagError::Git(format!("Failed to acquire lock: {}", e))
        })? = cache;

        tracing::debug!("Loaded persistent cache from {:?}", path);
        Ok(())
    }

    /// Flush persistent cache to disk using atomic write.
    pub async fn flush_persistent_cache(&self) -> Result<()> {
        let cache = self.persistent_cache.read().await;
        let path = self.cache_path();
        let data = serde_json::to_vec_pretty(&*cache).map_err(|e| {
            RagError::Git(format!("Failed to serialize cache: {}", e))
        })?;

        let path_clone = path.clone();
        tokio::task::spawn_blocking(move || {
            // Use atomic write pattern: temp file + rename
            if let Some(parent) = path_clone.parent() {
                fs::create_dir_all(parent)?;
            }

            // Write to temporary file first
            let temp_path = path_clone.with_extension("tmp");
            fs::write(&temp_path, &data)?;

            // Atomically rename to final location
            fs::rename(&temp_path, &path_clone)?;

            Ok::<(), std::io::Error>(())
        })
        .await
        .map_err(|e| RagError::Git(format!("Failed to join persist task: {}", e)))?
        .map_err(|e| RagError::from(e))?;

        tracing::debug!("Git sync cache persisted to {:?}", path);
        Ok(())
    }

    /// Background task for periodic cache persistence.
    async fn persist_task(
        project_path: PathBuf,
        persistent_cache: Arc<RwLock<GitSyncCache>>,
        dirty: Arc<AtomicBool>,
        interval: Duration,
    ) {
        let mut ticker = tokio::time::interval(interval);
        loop {
            ticker.tick().await;
            if dirty.load(Ordering::Relaxed) {
                let cache = persistent_cache.read().await;
                match serde_json::to_vec_pretty(&*cache) {
                    Ok(data) => {
                        let path = project_path.join(".rag/git_sync_cache.json");
                        let path_clone = path.clone();
                        let result = tokio::task::spawn_blocking(move || {
                            // Use atomic write pattern
                            if let Some(parent) = path_clone.parent() {
                                fs::create_dir_all(parent)?;
                            }
                            let temp_path = path_clone.with_extension("tmp");
                            fs::write(&temp_path, &data)?;
                            fs::rename(&temp_path, &path_clone)?;
                            Ok::<(), std::io::Error>(())
                        })
                        .await;

                        match result {
                            Ok(Ok(_)) => {
                                dirty.store(false, Ordering::Relaxed);
                                tracing::debug!("Persisted cache to {:?}", path);
                            }
                            Ok(Err(e)) => {
                                tracing::warn!("Failed to persist cache: {}", e);
                            }
                            Err(e) => {
                                tracing::warn!("Failed to join persist task: {}", e);
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!("Failed to serialize cache: {}", e);
                    }
                }
            }
        }
    }

    // ========================================================================
    // Internal (synchronous) implementation
    // ========================================================================

    /// Normalize file path for cross-platform compatibility.
    ///
    /// Converts Windows backslashes to forward slashes, which Git uses internally.
    /// This ensures consistent path handling across Windows and Unix-like systems.
    fn normalize_path(path: &str) -> String {
        path.replace('\\', "/")
    }

    /// Internal batch check implementation (runs in spawn_blocking).
    ///
    /// Implements two-tier caching:
    /// - L1 (Memory LRU): Fast access to recently checked files
    /// - L2 (Persistent): Complete cache that survives restarts
    fn batch_check_internal(
        project_path: &Path,
        file_paths: &[String],
        memory_cache: Arc<RwLock<LruCache<String, GitStatusEntry>>>,
        persistent_cache: Arc<RwLock<GitSyncCache>>,
        dirty: Arc<AtomicBool>,
        head_hash_ref: Arc<RwLock<Option<String>>>,
        cache_ttl_seconds: u64,
    ) -> Result<HashMap<String, GitSyncStatus>> {
        let repo = Repository::discover(project_path);
        let repo = match repo {
            Ok(r) => r,
            Err(_) => {
                // Not a Git repository
                let mut results = HashMap::new();
                for path in file_paths {
                    results.insert(path.clone(), GitSyncStatus::NotApplicable);
                }
                return Ok(results);
            }
        };

        // Get current HEAD
        let head = match repo.head() {
            Ok(h) => h,
            Err(_) => {
                // No HEAD (e.g., empty repo)
                let mut results = HashMap::new();
                for path in file_paths {
                    results.insert(path.clone(), GitSyncStatus::NotApplicable);
                }
                return Ok(results);
            }
        };

        // Get HEAD commit hash
        let head_commit = head.peel_to_commit();
        let current_hash = match head_commit {
            Ok(ref commit) => commit.id().to_string(),
            Err(e) => {
                tracing::debug!("Failed to get HEAD commit: {}", e);
                let mut results = HashMap::new();
                for path in file_paths {
                    results.insert(path.clone(), GitSyncStatus::NotApplicable);
                }
                return Ok(results);
            }
        };

        // Update cached HEAD hash
        {
            let mut head_hash = head_hash_ref.blocking_write();
            *head_hash = Some(current_hash.clone());
        }

        let now = Utc::now();
        let mut results = HashMap::new();
        let mut mem_cache = memory_cache.blocking_write();
        let mut persist_cache = persistent_cache.blocking_write();

        // Check if persistent cache HEAD has changed (invalidates all entries)
        let persist_head_changed = persist_cache.head_hash != current_hash
            && !persist_cache.head_hash.is_empty();

        for file_path in file_paths {
            // L1: Check memory cache first
            if let Some(entry) = mem_cache.get(file_path) {
                if entry.head_hash == current_hash {
                    let age = now.signed_duration_since(entry.cached_at).num_seconds();
                    if age <= cache_ttl_seconds as i64 {
                        if !entry.is_current {
                            let reason = entry.reason.clone()
                                .unwrap_or_else(|| "Deprecated file".to_string());
                            results.insert(file_path.clone(), GitSyncStatus::Deprecated { reason });
                            continue;
                        }
                        if let Some(ref cached_hash) = entry.file_hash {
                            if let Some(current_file_hash) = Self::compute_file_hash(&repo, file_path) {
                                if current_file_hash == *cached_hash {
                                    results.insert(file_path.clone(), GitSyncStatus::Current);
                                    continue;
                                }
                            }
                        }
                    }
                }
            }

            // L2: Check persistent cache
            if !persist_head_changed {
                if let Some(entry) = persist_cache.entries.get(file_path) {
                    if entry.head_hash == current_hash {
                        let age = now.signed_duration_since(entry.cached_at).num_seconds();
                        if age <= cache_ttl_seconds as i64 {
                            // Promote to memory cache
                            mem_cache.put(file_path.clone(), entry.clone());
                            if !entry.is_current {
                                let reason = entry.reason.clone()
                                    .unwrap_or_else(|| "Deprecated file".to_string());
                                results.insert(file_path.clone(), GitSyncStatus::Deprecated { reason });
                                continue;
                            }
                            if let Some(ref cached_hash) = entry.file_hash {
                                if let Some(current_file_hash) = Self::compute_file_hash(&repo, file_path) {
                                    if current_file_hash == *cached_hash {
                                        results.insert(file_path.clone(), GitSyncStatus::Current);
                                        continue;
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // Cache miss - check actual status
            let status = Self::check_file_status(&repo, file_path, &current_hash);

            let reason = match &status {
                GitSyncStatus::Deprecated { reason } => Some(reason.clone()),
                _ => None,
            };

            let file_hash = if matches!(status, GitSyncStatus::Current) {
                Self::compute_file_hash(&repo, file_path)
            } else {
                None
            };

            // Update both caches
            let entry = GitStatusEntry {
                head_hash: current_hash.clone(),
                is_current: matches!(status, GitSyncStatus::Current),
                cached_at: now,
                file_hash,
                reason,
            };

            mem_cache.put(file_path.clone(), entry.clone());
            persist_cache.entries.insert(file_path.clone(), entry);
            dirty.store(true, Ordering::Relaxed);

            results.insert(file_path.clone(), status);
        }

        // Update persistent cache metadata
        persist_cache.head_hash = current_hash;
        persist_cache.updated_at = now;

        // Clean up old entries if cache exceeds max size
        persist_cache.cleanup_old_entries();

        Ok(results)
    }

    /// Compute SHA-256 hash of file contents.
    ///
    /// Returns `None` if the file cannot be read (doesn't exist or permission denied)
    /// or if the file exceeds `MAX_HASH_SIZE` (to avoid blocking on large files).
    fn compute_file_hash(repo: &Repository, file_path: &str) -> Option<String> {
        // Normalize path for cross-platform compatibility
        let normalized_path = Self::normalize_path(file_path);

        // Get the full path to the file
        let full_path = repo.workdir()?.join(&normalized_path);

        // Check file size to avoid blocking on large files
        let metadata = fs::metadata(&full_path).ok()?;
        if metadata.len() > MAX_HASH_SIZE as u64 {
            tracing::debug!(
                "File {} ({} bytes) exceeds MAX_HASH_SIZE ({} bytes), skipping hash computation",
                file_path,
                metadata.len(),
                MAX_HASH_SIZE
            );
            return None;
        }

        // Try to open and read the file
        let mut file = fs::File::open(&full_path).ok()?;
        let mut hasher = Sha256::new();

        // Read in chunks to avoid loading large files into memory
        let mut buffer = [0u8; 8192];
        loop {
            let n = file.read(&mut buffer).ok()?;
            if n == 0 {
                break;
            }
            hasher.update(&buffer[..n]);
        }

        Some(format!("{:x}", hasher.finalize()))
    }

    /// Check the status of a single file against Git HEAD.
    fn check_file_status(repo: &Repository, file_path: &str, _head_hash: &str) -> GitSyncStatus {
        // Normalize path for cross-platform compatibility
        // Git always uses forward slashes internally
        let normalized_path = Self::normalize_path(file_path);

        // Check if file exists on disk
        let full_path = repo.workdir().map(|wd| wd.join(&normalized_path));
        let full_path = match full_path {
            Some(p) => p,
            None => return GitSyncStatus::NotApplicable,
        };

        if !full_path.exists() {
            return GitSyncStatus::Deprecated {
                reason: "File deleted".to_string(),
            };
        }

        // Check Git status using normalized path
        let status = match repo.status_file(std::path::Path::new(&normalized_path)) {
            Ok(s) => s,
            Err(_) => {
                // File might not be tracked
                return GitSyncStatus::NotApplicable;
            }
        };

        // Check if file is untracked (WT_NEW)
        if status.contains(Status::WT_NEW) {
            return GitSyncStatus::NotApplicable;
        }

        // Check if file is modified
        if status.contains(Status::WT_MODIFIED) || status.contains(Status::INDEX_MODIFIED) {
            return GitSyncStatus::Deprecated {
                reason: "Modified since last commit".to_string(),
            };
        }

        // Check if file is staged but not committed
        if status.contains(Status::INDEX_NEW) || status.contains(Status::INDEX_DELETED) {
            return GitSyncStatus::Deprecated {
                reason: "Staged changes".to_string(),
            };
        }

        // File is current (tracked and unchanged)
        GitSyncStatus::Current
    }
}

impl Drop for GitSync {
    fn drop(&mut self) {
        // Abort the background persistence task to prevent memory leaks
        if let Some(handle) = self._persist_handle.take() {
            handle.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{self, File};
    use std::io::Write;
    use std::process::Command;
    use tempfile::TempDir;

    /// Create a test Git repository with initial commit.
    fn create_test_repo() -> TempDir {
        let temp = TempDir::new().expect("Failed to create temp dir");
        let repo_path = temp.path();

        // Initialize Git repository
        Command::new("git")
            .args(["init"])
            .current_dir(repo_path)
            .output()
            .expect("Failed to init git");

        // Configure Git
        Command::new("git")
            .args(["config", "user.name", "Test User"])
            .current_dir(repo_path)
            .output()
            .expect("Failed to configure git user.name");

        Command::new("git")
            .args(["config", "user.email", "test@example.com"])
            .current_dir(repo_path)
            .output()
            .expect("Failed to configure git user.email");

        temp
    }

    /// Create and commit a file in the test repository.
    fn commit_file(repo_path: &Path, file_path: &str, content: &str, message: &str) {
        let full_path = repo_path.join(file_path);
        if let Some(parent) = full_path.parent() {
            fs::create_dir_all(parent).expect("Failed to create parent dir");
        }

        let mut file = File::create(&full_path).expect("Failed to create file");
        file.write_all(content.as_bytes()).expect("Failed to write file");

        Command::new("git")
            .args(["add", file_path])
            .current_dir(repo_path)
            .output()
            .expect("Failed to add file");

        Command::new("git")
            .args(["commit", "-m", message])
            .current_dir(repo_path)
            .output()
            .expect("Failed to commit");
    }

    #[test]
    fn test_git_sync_status_is_current() {
        assert!(GitSyncStatus::Current.is_current());
        assert!(!GitSyncStatus::Deprecated {
            reason: "test".to_string()
        }
        .is_current());
        assert!(!GitSyncStatus::NotApplicable.is_current());
    }

    #[tokio::test]
    async fn test_git_sync_new() {
        let temp = create_test_repo();
        let sync = GitSync::new(temp.path(), 60).expect("Failed to create GitSync");

        assert!(sync.repo.is_some());
        assert_eq!(sync.project_path, temp.path());
    }

    #[tokio::test]
    async fn test_git_sync_non_git_repository() {
        let temp = TempDir::new().expect("Failed to create temp dir");
        let sync = GitSync::new(temp.path(), 60).expect("Failed to create GitSync");

        assert!(sync.repo.is_none());
    }

    #[tokio::test]
    async fn test_check_file_sync_current() {
        let temp = create_test_repo();
        commit_file(temp.path(), "src/main.rs", "fn main() {}", "feat: initial commit");

        let sync = GitSync::new(temp.path(), 60).expect("Failed to create GitSync");
        let status = sync.check_file_sync("src/main.rs").await.expect("Failed to check");

        assert_eq!(status, GitSyncStatus::Current);
    }

    #[tokio::test]
    async fn test_check_file_sync_modified() {
        let temp = create_test_repo();
        commit_file(temp.path(), "src/main.rs", "fn main() {}", "feat: initial commit");

        // Modify file
        let file_path = temp.path().join("src/main.rs");
        let mut file = File::create(&file_path).expect("Failed to open file");
        file.write_all(b"fn main() { println!(\"Modified\"); }")
            .expect("Failed to write");

        let sync = GitSync::new(temp.path(), 60).expect("Failed to create GitSync");
        let status = sync.check_file_sync("src/main.rs").await.expect("Failed to check");

        assert!(matches!(status, GitSyncStatus::Deprecated { .. }));
    }

    #[tokio::test]
    async fn test_check_file_sync_deleted() {
        let temp = create_test_repo();
        commit_file(temp.path(), "src/lib.rs", "pub fn helper() {}", "feat: add lib");

        // Delete file
        fs::remove_file(temp.path().join("src/lib.rs")).expect("Failed to delete");

        let sync = GitSync::new(temp.path(), 60).expect("Failed to create GitSync");
        let status = sync.check_file_sync("src/lib.rs").await.expect("Failed to check");

        assert!(matches!(status, GitSyncStatus::Deprecated { .. }));
        if let GitSyncStatus::Deprecated { reason } = status {
            assert!(reason.contains("deleted") || reason.contains("Deleted"));
        }
    }

    #[tokio::test]
    async fn test_check_file_sync_untracked() {
        let temp = create_test_repo();
        commit_file(temp.path(), "src/main.rs", "fn main() {}", "feat: initial commit");

        // Create untracked file
        let file_path = temp.path().join("src/untracked.rs");
        let mut file = File::create(&file_path).expect("Failed to create file");
        file.write_all(b"// untracked").expect("Failed to write");

        let sync = GitSync::new(temp.path(), 60).expect("Failed to create GitSync");
        let status = sync.check_file_sync("src/untracked.rs").await.expect("Failed to check");

        // Untracked files are NotApplicable
        assert_eq!(status, GitSyncStatus::NotApplicable);
    }

    #[tokio::test]
    async fn test_batch_check_files() {
        let temp = create_test_repo();
        commit_file(temp.path(), "src/main.rs", "fn main() {}", "feat: add main");
        commit_file(temp.path(), "src/lib.rs", "pub fn lib() {}", "feat: add lib");
        commit_file(temp.path(), "src/utils.rs", "pub fn util() {}", "feat: add utils");

        let sync = GitSync::new(temp.path(), 60).expect("Failed to create GitSync");

        let paths = vec![
            "src/main.rs".to_string(),
            "src/lib.rs".to_string(),
            "src/utils.rs".to_string(),
        ];

        let results = sync.batch_check_files(&paths).await.expect("Failed to batch check");

        assert_eq!(results.len(), 3);
        assert_eq!(results.get("src/main.rs"), Some(&GitSyncStatus::Current));
        assert_eq!(results.get("src/lib.rs"), Some(&GitSyncStatus::Current));
        assert_eq!(results.get("src/utils.rs"), Some(&GitSyncStatus::Current));
    }

    #[tokio::test]
    async fn test_batch_check_empty() {
        let temp = create_test_repo();
        let sync = GitSync::new(temp.path(), 60).expect("Failed to create GitSync");

        let results = sync.batch_check_files(&[]).await.expect("Failed to batch check");

        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn test_batch_check_with_mixed_status() {
        let temp = create_test_repo();
        commit_file(temp.path(), "src/main.rs", "fn main() {}", "feat: add main");
        commit_file(temp.path(), "src/lib.rs", "pub fn lib() {}", "feat: add lib");

        // Modify lib.rs
        let file_path = temp.path().join("src/lib.rs");
        let mut file = File::create(&file_path).expect("Failed to open file");
        file.write_all(b"// modified\npub fn lib() {}").expect("Failed to write");

        let sync = GitSync::new(temp.path(), 60).expect("Failed to create GitSync");

        let paths = vec!["src/main.rs".to_string(), "src/lib.rs".to_string()];
        let results = sync.batch_check_files(&paths).await.expect("Failed to batch check");

        assert_eq!(results.len(), 2);
        assert_eq!(results.get("src/main.rs"), Some(&GitSyncStatus::Current));
        assert!(matches!(
            results.get("src/lib.rs"),
            Some(GitSyncStatus::Deprecated { .. })
        ));
    }

    #[tokio::test]
    async fn test_cache_hit_and_miss() {
        let temp = create_test_repo();
        commit_file(temp.path(), "src/main.rs", "fn main() {}", "feat: add main");

        let sync = GitSync::new(temp.path(), 60).expect("Failed to create GitSync");

        // First call - cache miss
        let status1 = sync.check_file_sync("src/main.rs").await.expect("Failed to check");
        assert_eq!(status1, GitSyncStatus::Current);

        // Second call - cache hit
        let status2 = sync.check_file_sync("src/main.rs").await.expect("Failed to check");
        assert_eq!(status2, GitSyncStatus::Current);
    }

    #[tokio::test]
    async fn test_cache_expiration() {
        let temp = create_test_repo();
        commit_file(temp.path(), "src/main.rs", "fn main() {}", "feat: add main");

        // Short TTL for testing
        let sync = GitSync::new(temp.path(), 0).expect("Failed to create GitSync");

        // First call
        let status1 = sync.check_file_sync("src/main.rs").await.expect("Failed to check");
        assert_eq!(status1, GitSyncStatus::Current);

        // Give time for cache to expire
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

        // Second call - cache should be expired
        let status2 = sync.check_file_sync("src/main.rs").await.expect("Failed to check");
        assert_eq!(status2, GitSyncStatus::Current);
    }

    #[tokio::test]
    async fn test_clear_cache() {
        let temp = create_test_repo();
        commit_file(temp.path(), "src/main.rs", "fn main() {}", "feat: add main");

        let sync = GitSync::new(temp.path(), 60).expect("Failed to create GitSync");

        // First call to populate cache
        sync.check_file_sync("src/main.rs").await.expect("Failed to check");

        // Clear cache
        sync.clear_cache().await;

        // Second call - cache should be cleared
        sync.check_file_sync("src/main.rs").await.expect("Failed to check");
    }

    #[tokio::test]
    async fn test_invalidate_file() {
        let temp = create_test_repo();
        commit_file(temp.path(), "src/main.rs", "fn main() {}", "feat: add main");

        let sync = GitSync::new(temp.path(), 60).expect("Failed to create GitSync");

        // First call to populate cache
        sync.check_file_sync("src/main.rs").await.expect("Failed to check");

        // Invalidate specific file
        sync.invalidate_file("src/main.rs").await;

        // Verify that after invalidation, re-checking still works correctly
        // (The cache was cleared, but the file is still Current)
        let status = sync
            .check_file_sync("src/main.rs")
            .await
            .expect("Failed to check after invalidation");
        assert_eq!(status, GitSyncStatus::Current);
    }

    #[tokio::test]
    async fn test_check_symbol_sync() {
        let temp = create_test_repo();
        commit_file(temp.path(), "src/lib.rs", "pub fn helper() {}", "feat: add lib");

        let sync = GitSync::new(temp.path(), 60).expect("Failed to create GitSync");
        let status = sync.check_symbol_sync("src/lib.rs").await.expect("Failed to check");

        // Symbol status should match file status
        assert_eq!(status, GitSyncStatus::Current);
    }

    #[tokio::test]
    async fn test_non_git_repo_batch_check() {
        let temp = TempDir::new().expect("Failed to create temp dir");
        let sync = GitSync::new(temp.path(), 60).expect("Failed to create GitSync");

        let paths = vec!["some/file.rs".to_string(), "other/file.rs".to_string()];
        let results = sync.batch_check_files(&paths).await.expect("Failed to batch check");

        assert_eq!(results.len(), 2);
        assert_eq!(results.get("some/file.rs"), Some(&GitSyncStatus::NotApplicable));
        assert_eq!(results.get("other/file.rs"), Some(&GitSyncStatus::NotApplicable));
    }

    #[test]
    fn test_git_status_entry() {
        let entry = GitStatusEntry {
            head_hash: "abc123".to_string(),
            is_current: true,
            cached_at: Utc::now(),
            file_hash: Some("hash123".to_string()),
            reason: Some("Modified".to_string()),
        };

        assert_eq!(entry.head_hash, "abc123");
        assert!(entry.is_current);
        assert_eq!(entry.file_hash, Some("hash123".to_string()));
        assert_eq!(entry.reason, Some("Modified".to_string()));
    }

    #[tokio::test]
    async fn test_file_hash_cache_hit_for_current_files() {
        let temp = create_test_repo();
        commit_file(temp.path(), "src/main.rs", "fn main() {}", "feat: add main");

        let sync = GitSync::new(temp.path(), 60).expect("Failed to create GitSync");

        // First check - file is Current, hash will be computed and cached
        let status1 = sync.check_file_sync("src/main.rs").await.expect("Failed to check");
        assert_eq!(status1, GitSyncStatus::Current);

        // Second check - should use cached status since file hasn't changed
        let status2 = sync.check_file_sync("src/main.rs").await.expect("Failed to check");
        assert_eq!(status2, GitSyncStatus::Current);

        // Modify the file slightly (add a comment)
        let file_path = temp.path().join("src/main.rs");
        let mut file = File::create(&file_path).expect("Failed to open file");
        file.write_all(b"fn main() {} // added comment")
            .expect("Failed to write");

        // Third check - file hash changed, should re-check Git status
        // (still Current because we haven't committed yet, but we had to re-check)
        let status3 = sync.check_file_sync("src/main.rs").await.expect("Failed to check");
        assert!(matches!(status3, GitSyncStatus::Deprecated { .. }));
    }

    #[tokio::test]
    async fn test_file_hash_ignored_for_deleted_files() {
        let temp = create_test_repo();
        commit_file(temp.path(), "src/lib.rs", "pub fn lib() {}", "feat: add lib");

        let sync = GitSync::new(temp.path(), 60).expect("Failed to create GitSync");

        // First check - file is Current
        let status1 = sync.check_file_sync("src/lib.rs").await.expect("Failed to check");
        assert_eq!(status1, GitSyncStatus::Current);

        // Delete the file
        fs::remove_file(temp.path().join("src/lib.rs")).expect("Failed to delete");

        // Second check - file doesn't exist, so hash can't be computed
        // Should return Deprecated without trying to compute hash
        let status2 = sync.check_file_sync("src/lib.rs").await.expect("Failed to check");
        assert!(matches!(status2, GitSyncStatus::Deprecated { .. }));
        if let GitSyncStatus::Deprecated { reason } = status2 {
            assert!(reason.contains("deleted") || reason.contains("Deleted"));
        }
    }

    #[tokio::test]
    async fn test_lru_cache_eviction() {
        let temp = create_test_repo();
        commit_file(temp.path(), "src/file1.rs", "pub fn f1() {}", "feat: add file1");
        commit_file(temp.path(), "src/file2.rs", "pub fn f2() {}", "feat: add file2");
        commit_file(temp.path(), "src/file3.rs", "pub fn f3() {}", "feat: add file3");

        // Create GitSync with capacity of 2
        let sync = GitSync::with_capacity(temp.path(), 60, 2).expect("Failed to create GitSync");

        // Check file1 - adds to cache
        let status1 = sync.check_file_sync("src/file1.rs").await.expect("Failed to check");
        assert_eq!(status1, GitSyncStatus::Current);

        // Check file2 - adds to cache, evicts file1 (capacity is 2)
        let status2 = sync.check_file_sync("src/file2.rs").await.expect("Failed to check");
        assert_eq!(status2, GitSyncStatus::Current);

        // Check file3 - adds to cache, evicts file2
        let status3 = sync.check_file_sync("src/file3.rs").await.expect("Failed to check");
        assert_eq!(status3, GitSyncStatus::Current);

        // Check file1 again - should re-check (was evicted), still current
        let status1_again = sync.check_file_sync("src/file1.rs").await.expect("Failed to check");
        assert_eq!(status1_again, GitSyncStatus::Current);

        // Modify file2
        let file_path = temp.path().join("src/file2.rs");
        let mut file = File::create(&file_path).expect("Failed to open file");
        file.write_all(b"// modified\npub fn f2() {}").expect("Failed to write");

        // Check file2 again - should detect modification
        let status2_modified = sync.check_file_sync("src/file2.rs").await.expect("Failed to check");
        assert!(matches!(status2_modified, GitSyncStatus::Deprecated { .. }));
    }
}
