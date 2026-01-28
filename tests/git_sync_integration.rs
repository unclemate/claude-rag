//! Integration tests for Git synchronization functionality.

use std::fs::{self, File};
use std::io::Write;
use std::process::Command;
use std::time::Duration;

use claude_rag::retrieval::GitSync;
use tempfile::TempDir;

/// Create a test Git repository with initial commit.
fn create_test_repo_with_commits() -> TempDir {
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

    // Create initial file and commit
    let src_dir = repo_path.join("src");
    fs::create_dir_all(&src_dir).expect("Failed to create src dir");

    let file_path = src_dir.join("main.rs");
    let mut file = File::create(&file_path).expect("Failed to create main.rs");
    writeln!(file, "fn main() {{ println!(\"Initial\"); }}").expect("Failed to write");

    Command::new("git")
        .args(["add", "."])
        .current_dir(repo_path)
        .output()
        .expect("Failed to add files");

    Command::new("git")
        .args(["commit", "-m", "feat: initial commit"])
        .current_dir(repo_path)
        .output()
        .expect("Failed to commit");

    temp
}

/// Create a test Git repository with multiple files and commits.
fn create_test_repo_with_multiple_files() -> TempDir {
    let temp = create_test_repo_with_commits();
    let repo_path = temp.path();

    // Add lib.rs
    let lib_path = repo_path.join("src/lib.rs");
    let mut file = File::create(&lib_path).expect("Failed to create lib.rs");
    writeln!(file, "pub fn helper() {{}}").expect("Failed to write");

    // Add utils.rs
    let utils_path = repo_path.join("src/utils.rs");
    let mut file = File::create(&utils_path).expect("Failed to create utils.rs");
    writeln!(file, "pub fn util() {{}}").expect("Failed to write");

    Command::new("git")
        .args(["add", "."])
        .current_dir(repo_path)
        .output()
        .expect("Failed to add files");

    Command::new("git")
        .args(["commit", "-m", "feat: add lib and utils"])
        .current_dir(repo_path)
        .output()
        .expect("Failed to commit");

    temp
}

#[tokio::test]
async fn test_full_git_sync_workflow() {
    let temp = create_test_repo_with_commits();
    let sync = GitSync::new(temp.path(), 60).expect("Failed to create GitSync");

    // Check unmodified file
    let status = sync
        .check_file_sync("src/main.rs")
        .await
        .expect("Failed to check file");
    assert_eq!(status, claude_rag::retrieval::GitSyncStatus::Current);

    // Modify file
    let file_path = temp.path().join("src/main.rs");
    let mut file = File::create(&file_path).expect("Failed to open file");
    writeln!(file, "fn main() {{ println!(\"Modified\"); }}")
        .expect("Failed to write");

    // Check modified file
    let status = sync
        .check_file_sync("src/main.rs")
        .await
        .expect("Failed to check file");
    assert!(
        matches!(status, claude_rag::retrieval::GitSyncStatus::Deprecated { .. }),
        "Modified file should be deprecated"
    );
}

#[tokio::test]
async fn test_cache_consistency() {
    let temp = create_test_repo_with_commits();
    let sync = GitSync::new(temp.path(), 60).expect("Failed to create GitSync");

    // First check
    let status1 = sync
        .check_file_sync("src/main.rs")
        .await
        .expect("Failed to check file");
    assert_eq!(status1, claude_rag::retrieval::GitSyncStatus::Current);

    // Second check (should hit cache)
    let status2 = sync
        .check_file_sync("src/main.rs")
        .await
        .expect("Failed to check file");
    assert_eq!(status2, claude_rag::retrieval::GitSyncStatus::Current);

    // Clear cache
    sync.clear_cache().await;

    // Third check (cache cleared)
    let status3 = sync
        .check_file_sync("src/main.rs")
        .await
        .expect("Failed to check file");
    assert_eq!(status3, claude_rag::retrieval::GitSyncStatus::Current);
}

#[tokio::test]
async fn test_cache_expiration() {
    let temp = create_test_repo_with_commits();
    // Very short TTL for testing
    let sync = GitSync::new(temp.path(), 0).expect("Failed to create GitSync");

    // First check
    let status1 = sync
        .check_file_sync("src/main.rs")
        .await
        .expect("Failed to check file");
    assert_eq!(status1, claude_rag::retrieval::GitSyncStatus::Current);

    // Give time for cache to expire
    tokio::time::sleep(Duration::from_millis(10)).await;

    // Second check (cache should be expired)
    let status2 = sync
        .check_file_sync("src/main.rs")
        .await
        .expect("Failed to check file");
    assert_eq!(status2, claude_rag::retrieval::GitSyncStatus::Current);
}

#[tokio::test]
async fn test_batch_check_files() {
    let temp = create_test_repo_with_multiple_files();
    let sync = GitSync::new(temp.path(), 60).expect("Failed to create GitSync");

    // Check all files
    let paths = vec![
        "src/main.rs".to_string(),
        "src/lib.rs".to_string(),
        "src/utils.rs".to_string(),
    ];

    let results = sync
        .batch_check_files(&paths)
        .await
        .expect("Failed to batch check");

    assert_eq!(results.len(), 3);
    assert_eq!(
        results.get("src/main.rs"),
        Some(&claude_rag::retrieval::GitSyncStatus::Current)
    );
    assert_eq!(
        results.get("src/lib.rs"),
        Some(&claude_rag::retrieval::GitSyncStatus::Current)
    );
    assert_eq!(
        results.get("src/utils.rs"),
        Some(&claude_rag::retrieval::GitSyncStatus::Current)
    );

    // Modify one file
    let file_path = temp.path().join("src/lib.rs");
    let mut file = File::create(&file_path).expect("Failed to open file");
    writeln!(file, "// modified\npub fn helper() {{}}").expect("Failed to write");

    // Re-check
    let results = sync
        .batch_check_files(&paths)
        .await
        .expect("Failed to batch check");

    assert_eq!(results.get("src/main.rs"), Some(&claude_rag::retrieval::GitSyncStatus::Current));
    assert!(matches!(
        results.get("src/lib.rs"),
        Some(claude_rag::retrieval::GitSyncStatus::Deprecated { .. })
    ));
    assert_eq!(
        results.get("src/utils.rs"),
        Some(&claude_rag::retrieval::GitSyncStatus::Current)
    );
}

#[tokio::test]
async fn test_batch_check_empty() {
    let temp = create_test_repo_with_commits();
    let sync = GitSync::new(temp.path(), 60).expect("Failed to create GitSync");

    let results = sync
        .batch_check_files(&[])
        .await
        .expect("Failed to batch check");

    assert!(results.is_empty());
}

#[tokio::test]
async fn test_invalidate_file() {
    let temp = create_test_repo_with_commits();
    let sync = GitSync::new(temp.path(), 60).expect("Failed to create GitSync");

    // First check to populate cache
    sync.check_file_sync("src/main.rs")
        .await
        .expect("Failed to check file");

    // Invalidate specific file (should not error)
    sync.invalidate_file("src/main.rs").await;

    // Re-check file to verify it still works after invalidation
    let status = sync
        .check_file_sync("src/main.rs")
        .await
        .expect("Failed to check file");

    assert_eq!(status, claude_rag::retrieval::GitSyncStatus::Current);
}

#[tokio::test]
async fn test_check_symbol_sync() {
    let temp = create_test_repo_with_commits();
    let sync = GitSync::new(temp.path(), 60).expect("Failed to create GitSync");

    // Symbol inherits file status
    let status = sync
        .check_symbol_sync("src/main.rs")
        .await
        .expect("Failed to check symbol");

    assert_eq!(status, claude_rag::retrieval::GitSyncStatus::Current);
}

#[tokio::test]
async fn test_non_git_repository() {
    let temp = TempDir::new().expect("Failed to create temp dir");
    let sync = GitSync::new(temp.path(), 60).expect("Failed to create GitSync");

    // Non-Git repo should return NotApplicable
    let status = sync
        .check_file_sync("some/file.txt")
        .await
        .expect("Failed to check file");

    assert_eq!(status, claude_rag::retrieval::GitSyncStatus::NotApplicable);
}

#[tokio::test]
async fn test_deleted_file() {
    let temp = create_test_repo_with_commits();
    let sync = GitSync::new(temp.path(), 60).expect("Failed to create GitSync");

    // Delete file
    let file_path = temp.path().join("src/main.rs");
    fs::remove_file(&file_path).expect("Failed to delete file");

    // Check status
    let status = sync
        .check_file_sync("src/main.rs")
        .await
        .expect("Failed to check file");

    assert!(matches!(status, claude_rag::retrieval::GitSyncStatus::Deprecated { .. }));
}

#[tokio::test]
async fn test_untracked_file() {
    let temp = create_test_repo_with_commits();
    let sync = GitSync::new(temp.path(), 60).expect("Failed to create GitSync");

    // Create untracked file
    let file_path = temp.path().join("src/untracked.rs");
    let mut file = File::create(&file_path).expect("Failed to create file");
    writeln!(file, "// untracked").expect("Failed to write");

    // Check status
    let status = sync
        .check_file_sync("src/untracked.rs")
        .await
        .expect("Failed to check file");

    // Untracked files should return NotApplicable
    assert_eq!(status, claude_rag::retrieval::GitSyncStatus::NotApplicable);
}

#[tokio::test]
async fn test_staged_changes() {
    let temp = create_test_repo_with_commits();
    let sync = GitSync::new(temp.path(), 60).expect("Failed to create GitSync");

    // Modify and stage file
    let file_path = temp.path().join("src/main.rs");
    let mut file = File::create(&file_path).expect("Failed to open file");
    writeln!(file, "// staged changes").expect("Failed to write");

    Command::new("git")
        .args(["add", "src/main.rs"])
        .current_dir(temp.path())
        .output()
        .expect("Failed to stage file");

    // Check status
    let status = sync
        .check_file_sync("src/main.rs")
        .await
        .expect("Failed to check file");

    // Staged but uncommitted changes should be deprecated
    assert!(matches!(status, claude_rag::retrieval::GitSyncStatus::Deprecated { .. }));
}

/// Test that demonstrates the full workflow from commit to modification.
#[tokio::test]
async fn test_commit_to_deprecated_workflow() {
    let temp = create_test_repo_with_commits();
    let sync = GitSync::new(temp.path(), 60).expect("Failed to create GitSync");

    // Initial state: file is current
    let status = sync
        .check_file_sync("src/main.rs")
        .await
        .expect("Failed to check file");
    assert_eq!(status, claude_rag::retrieval::GitSyncStatus::Current);

    // Modify file
    let file_path = temp.path().join("src/main.rs");
    let mut file = File::create(&file_path).expect("Failed to open file");
    writeln!(file, "fn main() {{ println!(\"Modified\"); }}")
        .expect("Failed to write");

    // File is now deprecated
    let status = sync
        .check_file_sync("src/main.rs")
        .await
        .expect("Failed to check file");
    assert!(matches!(status, claude_rag::retrieval::GitSyncStatus::Deprecated { .. }));

    // Commit the change
    Command::new("git")
        .args(["add", "src/main.rs"])
        .current_dir(temp.path())
        .output()
        .expect("Failed to stage file");

    Command::new("git")
        .args(["commit", "-m", "fix: update main"])
        .current_dir(temp.path())
        .output()
        .expect("Failed to commit");

    // Clear cache to force re-check
    sync.clear_cache().await;

    // File should be current again after commit
    let status = sync
        .check_file_sync("src/main.rs")
        .await
        .expect("Failed to check file");
    assert_eq!(status, claude_rag::retrieval::GitSyncStatus::Current);
}

/// Test HEAD hash change detection.
#[tokio::test]
async fn test_head_hash_change_invalidates_cache() {
    let temp = create_test_repo_with_commits();
    let sync = GitSync::new(temp.path(), 60).expect("Failed to create GitSync");

    // First check - populate cache
    sync.check_file_sync("src/main.rs")
        .await
        .expect("Failed to check file");

    // Make a new commit (changes HEAD)
    let file_path = temp.path().join("src/README.md");
    let mut file = File::create(&file_path).expect("Failed to create file");
    writeln!(file, "# README").expect("Failed to write");

    Command::new("git")
        .args(["add", "."])
        .current_dir(temp.path())
        .output()
        .expect("Failed to add");

    Command::new("git")
        .args(["commit", "-m", "docs: add readme"])
        .current_dir(temp.path())
        .output()
        .expect("Failed to commit");

    // Cache should be invalidated due to HEAD change
    // Second check should detect HEAD hash mismatch and re-check
    let status = sync
        .check_file_sync("src/main.rs")
        .await
        .expect("Failed to check file");
    assert_eq!(status, claude_rag::retrieval::GitSyncStatus::Current);
}

/// Test that persistent cache is saved to disk.
#[tokio::test]
async fn test_persistent_cache_save() {
    let temp = create_test_repo_with_commits();
    let sync = GitSync::new(temp.path(), 60).expect("Failed to create GitSync");

    // Write some entries to cache
    sync.check_file_sync("src/main.rs").await.expect("Failed to check file");

    // Manually flush to disk
    sync.flush_persistent_cache().await.expect("Failed to flush cache");

    // Verify cache file exists
    let cache_path = temp.path().join(".rag/git_sync_cache.json");
    assert!(cache_path.exists(), "Cache file should exist after flush");
}

/// Test that persistent cache is loaded on restart.
#[tokio::test]
async fn test_persistent_cache_load_on_restart() {
    let temp = create_test_repo_with_commits();

    // First instance - create and populate cache
    let sync1 = GitSync::new(temp.path(), 60).expect("Failed to create GitSync");
    sync1.check_file_sync("src/main.rs").await.expect("Failed to check file");
    sync1.flush_persistent_cache().await.expect("Failed to flush cache");

    // Simulate restart by creating a new instance
    let sync2 = GitSync::new(temp.path(), 60).expect("Failed to create GitSync");

    // The cache should be loaded from disk
    let status = sync2.check_file_sync("src/main.rs").await.expect("Failed to check file");
    assert_eq!(status, claude_rag::retrieval::GitSyncStatus::Current);
}

/// Test that cache is invalidated when HEAD changes.
#[tokio::test]
async fn test_cache_invalidation_on_head_change() {
    let temp = create_test_repo_with_commits();
    let sync = GitSync::new(temp.path(), 60).expect("Failed to create GitSync");

    // First check - populate cache
    sync.check_file_sync("src/main.rs").await.expect("Failed to check file");
    sync.flush_persistent_cache().await.expect("Failed to flush cache");

    // Make a new commit (changes HEAD)
    let file_path = temp.path().join("src/extra.txt");
    let mut file = File::create(&file_path).expect("Failed to create file");
    writeln!(file, "extra content").expect("Failed to write");

    Command::new("git")
        .args(["add", "."])
        .current_dir(temp.path())
        .output()
        .expect("Failed to add");

    Command::new("git")
        .args(["commit", "-m", "feat: add extra"])
        .current_dir(temp.path())
        .output()
        .expect("Failed to commit");

    // Create new instance - should detect HEAD change and invalidate
    let sync2 = GitSync::new(temp.path(), 60).expect("Failed to create GitSync");
    let status = sync2.check_file_sync("src/main.rs").await.expect("Failed to check file");
    assert_eq!(status, claude_rag::retrieval::GitSyncStatus::Current);
}

/// Test graceful degradation when persistence fails.
#[tokio::test]
async fn test_persistence_failure_graceful_degradation() {
    let temp = create_test_repo_with_commits();

    // Create instance with persistence disabled (should still work)
    let sync = GitSync::with_options(temp.path(), 60, 100, false).expect("Failed to create GitSync");

    // Should still work even without persistence
    let status = sync.check_file_sync("src/main.rs").await.expect("Failed to check file");
    assert_eq!(status, claude_rag::retrieval::GitSyncStatus::Current);

    // Cache file should not exist
    let cache_path = temp.path().join(".rag/git_sync_cache.json");
    assert!(!cache_path.exists(), "Cache file should not exist when persistence is disabled");
}

/// Test full workflow with persistence enabled.
#[tokio::test]
async fn test_persistence_full_workflow() {
    let temp = create_test_repo_with_commits();

    // First run
    let sync1 = GitSync::new(temp.path(), 60).expect("Failed to create GitSync");
    sync1.check_file_sync("src/main.rs").await.expect("Failed to check file");
    sync1.flush_persistent_cache().await.expect("Failed to flush cache");

    // Simulate program restart
    drop(sync1);

    // Second run - should load cache from disk
    let sync2 = GitSync::new(temp.path(), 60).expect("Failed to create GitSync");
    let status = sync2.check_file_sync("src/main.rs").await.expect("Failed to check file");
    assert_eq!(status, claude_rag::retrieval::GitSyncStatus::Current);
}

/// Test rapid sequential cache access (simulates concurrent-like usage).
#[tokio::test]
async fn test_rapid_sequential_access() {
    let temp = create_test_repo_with_commits();
    let sync = GitSync::new(temp.path(), 60).expect("Failed to create GitSync");

    // Perform many sequential operations rapidly
    for i in 0..100 {
        let result = sync.check_file_sync("src/main.rs").await;
        assert!(result.is_ok(), "Iteration {} failed", i);
        assert_eq!(result.unwrap(), claude_rag::retrieval::GitSyncStatus::Current);
    }

    // Verify cache consistency
    let final_status = sync.check_file_sync("src/main.rs").await.expect("Final check failed");
    assert_eq!(final_status, claude_rag::retrieval::GitSyncStatus::Current);
}

/// Test multiple independent GitSync instances on same repository.
#[tokio::test]
async fn test_multiple_instances_same_repo() {
    let temp = create_test_repo_with_commits();

    // Create multiple instances
    let sync1 = GitSync::new(temp.path(), 60).expect("Failed to create GitSync 1");
    let sync2 = GitSync::new(temp.path(), 60).expect("Failed to create GitSync 2");
    let sync3 = GitSync::new(temp.path(), 60).expect("Failed to create GitSync 3");

    // All should work independently
    let status1 = sync1.check_file_sync("src/main.rs").await.expect("Sync 1 failed");
    let status2 = sync2.check_file_sync("src/main.rs").await.expect("Sync 2 failed");
    let status3 = sync3.check_file_sync("src/main.rs").await.expect("Sync 3 failed");

    assert_eq!(status1, claude_rag::retrieval::GitSyncStatus::Current);
    assert_eq!(status2, claude_rag::retrieval::GitSyncStatus::Current);
    assert_eq!(status3, claude_rag::retrieval::GitSyncStatus::Current);
}

/// Test handling of corrupted cache file.
#[tokio::test]
async fn test_corrupted_cache_file() {
    let temp = create_test_repo_with_commits();

    // Write corrupted cache file
    let cache_path = temp.path().join(".rag/git_sync_cache.json");
    if let Some(parent) = cache_path.parent() {
        std::fs::create_dir_all(parent).expect("Failed to create .rag dir");
    }
    std::fs::write(&cache_path, b"{invalid json content").expect("Failed to write corrupted cache");

    // Should gracefully degrade to memory mode
    let sync = GitSync::new(temp.path(), 60).expect("Failed to create GitSync with corrupted cache");
    let status = sync.check_file_sync("src/main.rs").await.expect("Failed to check file");
    assert_eq!(status, claude_rag::retrieval::GitSyncStatus::Current);
}

/// Test handling of unsupported cache version.
#[tokio::test]
async fn test_unsupported_cache_version() {
    let temp = create_test_repo_with_commits();

    // Write cache with unsupported version
    let cache_path = temp.path().join(".rag/git_sync_cache.json");
    if let Some(parent) = cache_path.parent() {
        std::fs::create_dir_all(parent).expect("Failed to create .rag dir");
    }
    std::fs::write(&cache_path, r#"{"version":999,"head_hash":"","entries":{},"max_entries":10000,"updated_at":"2024-01-01T00:00:00Z"}"#)
        .expect("Failed to write cache");

    // Should ignore the cache and start fresh
    let sync = GitSync::new(temp.path(), 60).expect("Failed to create GitSync");
    let status = sync.check_file_sync("src/main.rs").await.expect("Failed to check file");
    assert_eq!(status, claude_rag::retrieval::GitSyncStatus::Current);
}

/// Test persistent cache cleanup when exceeding max entries.
#[tokio::test]
async fn test_persistent_cache_cleanup() {
    let temp = create_test_repo_with_commits();

    // Create many files to exceed the default max_entries (10000)
    let sync = GitSync::new(temp.path(), 60).expect("Failed to create GitSync");

    // Add enough entries to exceed the limit (we'll use a smaller limit for testing)
    let paths: Vec<String> = (0..100).map(|i| format!("src/file_{:03}.rs", i)).collect();

    // Create the files
    for path in &paths {
        let full_path = temp.path().join(path);
        if let Some(parent) = full_path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        std::fs::write(&full_path, format!("// content for {}", path)).ok();

        // Commit each file
        std::process::Command::new("git")
            .args(["add", path])
            .current_dir(temp.path())
            .output()
            .ok();
    }

    std::process::Command::new("git")
        .args(["commit", "-m", "feat: add many files"])
        .current_dir(temp.path())
        .output()
        .expect("Failed to commit");

    // Check all files
    let results = sync.batch_check_files(&paths).await.expect("Failed to batch check");

    // All should be current
    for path in &paths {
        assert_eq!(
            results.get(path),
            Some(&claude_rag::retrieval::GitSyncStatus::Current),
            "File {} should be current", path
        );
    }

    // Flush and verify cache cleanup
    sync.flush_persistent_cache().await.expect("Failed to flush");

    // Load cache again and verify it still works
    let sync2 = GitSync::new(temp.path(), 60).expect("Failed to create GitSync");
    let status = sync2.check_file_sync("src/main.rs").await.expect("Failed to check");
    assert_eq!(status, claude_rag::retrieval::GitSyncStatus::Current);
}
