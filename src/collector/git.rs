//! Git history collection.
//!
//! This module provides functionality to collect Git commit history and file diffs
//! for time-aware RAG indexing. It uses the git2 crate to interact with Git repositories.

use crate::error::{RagError, Result};
use crate::models::{Commit, GitDiff};
use crate::models::diff::ChangeType;
use chrono::{DateTime, TimeZone, Utc};
use git2::{Diff, Oid, Repository, Time};
use std::path::{Path, PathBuf};
use tracing::{debug, info, trace, warn};

/// Conventional commit types.
const CONVENTIONAL_TYPES: &[&str] = &[
    "feat", "fix", "docs", "style", "refactor", "perf", "test", "chore", "build", "ci", "revert",
];

/// Git collector for repository history.
pub struct GitCollector {
    /// Path to the Git repository
    repo_path: PathBuf,
    /// git2 Repository handle
    repo: Option<Repository>,
}

impl GitCollector {
    /// Create a new Git collector for a project.
    ///
    /// # Arguments
    /// * `project_path` - Path to the project (should contain .git directory)
    ///
    /// # Returns
    /// * `Result<Self>` - The collector or error if not a Git repository
    pub fn new(project_path: &Path) -> Result<Self> {
        debug!("Creating GitCollector for project: {}", project_path.display());
        let repo_path = project_path.to_path_buf();
        let repo = Repository::discover(project_path)
            .map_err(|e| RagError::Git(format!("Failed to open repository: {e}")))?;

        Ok(Self {
            repo_path,
            repo: Some(repo),
        })
    }

    /// Get the repository handle.
    fn repo(&self) -> &Repository {
        self.repo.as_ref().expect("Repository not initialized")
    }

    /// Collect all commits from the repository.
    ///
    /// # Arguments
    /// * `max_count` - Maximum number of commits to collect (None for all)
    ///
    /// # Returns
    /// * `Result<Vec<Commit>>` - List of commits
    pub fn collect_all_commits(&self, max_count: Option<usize>) -> Result<Vec<Commit>> {
        info!(
            max_count = max_count,
            "Collecting commits from repository"
        );
        let repo = self.repo();
        let mut revwalk = repo.revwalk()
            .map_err(|e| RagError::Git(format!("Failed to create revwalk: {e}")))?;

        // Push HEAD onto the revision walker
        revwalk.push_head()
            .map_err(|e| RagError::Git(format!("Failed to push HEAD: {e}")))?;

        let mut commits = Vec::new();

        for (count, oid) in revwalk.enumerate() {
            if let Some(max) = max_count {
                if count >= max {
                    break;
                }
            }

            let oid = oid.map_err(|e| RagError::Git(format!("Failed to get OID: {e}")))?;
            let commit = repo.find_commit(oid)
                .map_err(|e| RagError::Git(format!("Failed to find commit: {e}")))?;

            commits.push(self.convert_commit(&commit, repo)?);
        }

        info!(
            commits_count = commits.len(),
            "Collected commits from repository"
        );
        debug!(
            commits = ?commits.iter().map(|c| &c.short_hash).collect::<Vec<_>>(),
            "Commit list"
        );
        Ok(commits)
    }

    /// Get file diffs for a specific commit.
    ///
    /// # Arguments
    /// * `commit_id` - The commit hash
    ///
    /// # Returns
    /// * `Result<Vec<GitDiff>>` - List of file diffs
    pub fn get_file_diffs(&self, commit_id: &str) -> Result<Vec<GitDiff>> {
        debug!("Getting file diffs for commit: {}", commit_id);
        let repo = self.repo();
        let oid = Oid::from_str(commit_id)
            .map_err(|e| RagError::Git(format!("Invalid commit ID: {e}")))?;

        let commit = repo.find_commit(oid)
            .map_err(|e| RagError::Git(format!("Failed to find commit: {e}")))?;

        let parent = commit.parent(0).ok();
        let tree = commit.tree().map_err(|e| RagError::Git(format!("Failed to get tree: {e}")))?;
        let parent_tree = parent.as_ref().and_then(|p| p.tree().ok());

        // Create diff object
        let diff = if let Some(pt) = parent_tree {
            repo.diff_tree_to_tree(Some(&pt), Some(&tree), None)
        } else {
            repo.diff_tree_to_tree(None, Some(&tree), None)
        }.map_err(|e| RagError::Git(format!("Failed to create diff: {e}")))?;

        let mut diffs = Vec::new();

        // Process each delta in the diff
        for delta in diff.deltas() {
            let file_diff = self.convert_delta(&delta, &diff, commit_id, &commit)?;
            diffs.push(file_diff);
        }

        debug!("Found {} file diffs for commit {}", diffs.len(), commit_id);
        Ok(diffs)
    }

    /// Parse conventional commit information from a message.
    ///
    /// # Arguments
    /// * `message` - The commit message
    ///
    /// # Returns
    /// * `(Option<String>, Option<String>, bool)` - (type, scope, is_breaking)
    pub fn parse_conventional_commit(message: &str) -> (Option<String>, Option<String>, bool) {
        let first_line = message.lines().next().unwrap_or("");

        // Check for conventional commit format: type(scope)!: or type(scope):
        let is_breaking = first_line.contains("!:");

        // Remove the breaking change marker
        let cleaned = first_line.replace("!:", ":");

        // Find the type
        let conv_type = CONVENTIONAL_TYPES.iter()
            .find(|&&t| cleaned.starts_with(&format!("{t}(")) || cleaned.starts_with(&format!("{t}:")))
            .map(|&s| s.to_string());

        // Extract scope if present
        let scope = if let Some(typ) = &conv_type {
            let scope_start = cleaned.find(&format!("{typ}("));
            scope_start.and_then(|start| {
                cleaned[start..].find(')').map(|end| {
                    cleaned[start + typ.len() + 1..start + end].to_string()
                })
            })
        } else {
            None
        };

        // Also check for BREAKING CHANGE in footer
        let has_breaking_footer = message.lines()
            .skip(1)
            .any(|line| line.to_uppercase().contains("BREAKING CHANGE"));

        let is_breaking = is_breaking || has_breaking_footer;

        trace!(
            "Parsed conventional commit: type={:?}, scope={:?}, breaking={}",
            conv_type, scope, is_breaking
        );

        (conv_type, scope, is_breaking)
    }

    /// Extract breaking changes from a commit message.
    ///
    /// # Arguments
    /// * `message` - The commit message
    ///
    /// # Returns
    /// * `Vec<String>` - List of breaking change descriptions
    pub fn extract_breaking_changes(message: &str) -> Vec<String> {
        let mut breaking_changes = Vec::new();

        // Check for !: in title
        if message.contains("!:") {
            if let Some(first_line) = message.lines().next() {
                let cleaned = first_line.replace("!:", ":");
                if let Some(colon_pos) = cleaned.find(':') {
                    breaking_changes.push(cleaned[colon_pos + 1..].trim().to_string());
                }
            }
        }

        // Check for BREAKING CHANGE in footer
        for line in message.lines().skip(1) {
            let upper = line.to_uppercase();
            if upper.contains("BREAKING CHANGE") || upper.contains("BREAKING-CHANGE") {
                if let Some(colon_pos) = line.find(':') {
                    breaking_changes.push(line[colon_pos + 1..].trim().to_string());
                }
            }
        }

        breaking_changes
    }

    /// Convert a git2 commit to our Commit model.
    fn convert_commit(&self, commit: &git2::Commit, _repo: &Repository) -> Result<Commit> {
        let id = commit.id().to_string();
        let short_hash = id[..7.min(id.len())].to_string();

        let author = commit.author();
        let author_name = author.name().unwrap_or("Unknown").to_string();
        let author_email = author.email().unwrap_or("unknown@example.com").to_string();

        // Convert git2 Time to DateTime<Utc>
        let commit_time = commit.time();
        let commit_date = self.git_time_to_datetime(commit_time);

        let message = commit.message().unwrap_or("").to_string();
        let message_summary = message.lines().next().unwrap_or("").to_string();

        // Parse conventional commit
        let (conv_type, conv_scope, is_breaking) = Self::parse_conventional_commit(&message);

        // Get parent hashes
        let parent_hashes: Vec<String> = commit.parent_ids()
            .map(|oid| oid.to_string())
            .collect();

        // Count files changed (using parent count as estimate)
        let files_changed = commit.parent_ids().count();

        let insertions = 0;
        let deletions = 0;

        Ok(Commit {
            id,
            short_hash,
            project_path: self.repo_path.to_string_lossy().to_string(),
            author_name,
            author_email,
            commit_date,
            message,
            message_summary,
            conv_type,
            conv_scope,
            is_breaking,
            parent_hashes,
            files_changed,
            insertions,
            deletions,
        })
    }

    /// Convert a git2 Delta to our GitDiff model.
    fn convert_delta(
        &self,
        delta: &git2::DiffDelta,
        diff: &Diff,
        commit_id: &str,
        commit: &git2::Commit,
    ) -> Result<GitDiff> {
        let file_path = delta.new_file().path()
            .or_else(|| delta.old_file().path())
            .and_then(|p| p.to_str())
            .unwrap_or("")
            .to_string();

        let old_oid = if delta.old_file().id() != Oid::zero() {
            Some(delta.old_file().id().to_string())
        } else {
            None
        };

        let new_oid = if delta.new_file().id() != Oid::zero() {
            Some(delta.new_file().id().to_string())
        } else {
            None
        };

        let change_type = match delta.status() {
            git2::Delta::Added => ChangeType::Added,
            git2::Delta::Deleted => ChangeType::Deleted,
            git2::Delta::Renamed => ChangeType::Renamed,
            git2::Delta::Copied => ChangeType::Copied,
            _ => ChangeType::Modified,
        };

        // Get diff content and stats
        let diff_content = self.get_diff_text(diff, delta);
        let (added_lines, removed_lines) = self.count_diff_lines(&diff_content);

        // Get timestamp from commit
        let commit_time = commit.time();
        let timestamp = self.git_time_to_datetime(commit_time);

        // Generate diff ID
        let id = format!("diff:{commit_id}:{file_path}");

        Ok(GitDiff {
            id,
            commit_id: commit_id.to_string(),
            project_path: self.repo_path.to_string_lossy().to_string(),
            file_path,
            old_oid,
            new_oid,
            change_type,
            diff_content,
            diff_summary: format!("{change_type:?} change"),
            added_lines,
            removed_lines,
            insertions: added_lines,
            deletions: removed_lines,
            timestamp,
        })
    }

    /// Get the text content of a diff.
    fn get_diff_text(&self, diff: &Diff, _delta: &git2::DiffDelta) -> String {
        let mut content = String::new();
        if diff.print(git2::DiffFormat::Patch, |_, _, line| {
            content.push_str(std::str::from_utf8(line.content()).unwrap_or(""));
            true
        }).is_err() {
            return String::new();
        }
        content
    }

    /// Count added and removed lines in a diff.
    fn count_diff_lines(&self, diff_content: &str) -> (usize, usize) {
        let mut added = 0;
        let mut removed = 0;

        for line in diff_content.lines() {
            if line.starts_with('+') && !line.starts_with("+++") {
                added += 1;
            } else if line.starts_with('-') && !line.starts_with("---") {
                removed += 1;
            }
        }

        (added, removed)
    }

    /// Convert git2 Time to DateTime<Utc>.
    fn git_time_to_datetime(&self, time: Time) -> DateTime<Utc> {
        Utc.timestamp_opt(time.seconds(), 0).unwrap()
    }

    /// Check if the project is a Git repository.
    ///
    /// # Arguments
    /// * `project_path` - Path to check
    ///
    /// # Returns
    /// * `bool` - true if it's a Git repository
    pub fn is_git_repository(project_path: &Path) -> bool {
        let is_git = Repository::discover(project_path).is_ok();
        if !is_git {
            warn!("Not a Git repository: {}", project_path.display());
        }
        is_git
    }

    /// Get commits since a specific date.
    ///
    /// # Arguments
    /// * `since` - DateTime to start from
    ///
    /// # Returns
    /// * `Result<Vec<Commit>>` - List of commits since the date
    pub fn commits_since(&self, since: DateTime<Utc>) -> Result<Vec<Commit>> {
        let all_commits = self.collect_all_commits(None)?;
        let filtered = all_commits.into_iter()
            .filter(|c| c.commit_date > since)
            .collect();
        Ok(filtered)
    }

    /// Get commit history for a specific file.
    ///
    /// # Arguments
    /// * `file_path` - Path to the file (relative to repo root)
    /// * `max_count` - Maximum number of commits (None for all)
    ///
    /// # Returns
    /// * `Result<Vec<Commit>>` - List of commits that touched the file
    pub fn file_history(&self, file_path: &str, max_count: Option<usize>) -> Result<Vec<Commit>> {
        let repo = self.repo();
        let mut revwalk = repo.revwalk()
            .map_err(|e| RagError::Git(format!("Failed to create revwalk: {e}")))?;

        revwalk.push_head()
            .map_err(|e| RagError::Git(format!("Failed to push HEAD: {e}")))?;

        let mut commits = Vec::new();
        let mut count = 0;

        for oid in revwalk {
            if let Some(max) = max_count {
                if count >= max {
                    break;
                }
            }

            let oid = oid.map_err(|e| RagError::Git(format!("Failed to get OID: {e}")))?;
            let commit = repo.find_commit(oid)
                .map_err(|e| RagError::Git(format!("Failed to find commit: {e}")))?;

            // Check if this commit touched the file
            if self.commit_touched_file(&commit, file_path)? {
                commits.push(self.convert_commit(&commit, repo)?);
                count += 1;
            }
        }

        Ok(commits)
    }

    /// Check if a commit touched a specific file.
    fn commit_touched_file(&self, commit: &git2::Commit, file_path: &str) -> Result<bool> {
        let repo = self.repo();
        let parent = commit.parent(0).ok();

        let tree = commit.tree().map_err(|e| RagError::Git(format!("Failed to get tree: {e}")))?;
        let parent_tree = parent.as_ref().and_then(|p| p.tree().ok());

        let diff = if let Some(pt) = parent_tree {
            repo.diff_tree_to_tree(Some(&pt), Some(&tree), None)
        } else {
            repo.diff_tree_to_tree(None, Some(&tree), None)
        }.map_err(|e| RagError::Git(format!("Failed to create diff: {e}")))?;

        for delta in diff.deltas() {
            let path = delta.new_file().path()
                .or_else(|| delta.old_file().path())
                .and_then(|p| p.to_str())
                .unwrap_or("");

            if path == file_path {
                return Ok(true);
            }
        }

        Ok(false)
    }
}

impl Default for GitCollector {
    fn default() -> Self {
        Self::new(Path::new(".")).unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;
    use std::process::Command;
    use tempfile::TempDir;

    /// Create a test Git repository with some commits
    fn create_test_repo() -> TempDir {
        let temp = TempDir::new().unwrap();
        let repo_path = temp.path();

        // Initialize repo
        Command::new("git")
            .args(["init"])
            .current_dir(repo_path)
            .output()
            .expect("Failed to init git repo");

        // Configure git
        Command::new("git")
            .args(["config", "user.name", "Test User"])
            .current_dir(repo_path)
            .output()
            .expect("Failed to configure git");

        Command::new("git")
            .args(["config", "user.email", "test@example.com"])
            .current_dir(repo_path)
            .output()
            .expect("Failed to configure git");

        // Create initial file and commit
        let file_path = repo_path.join("test.txt");
        let mut file = File::create(&file_path).unwrap();
        writeln!(file, "Initial content").unwrap();

        Command::new("git")
            .args(["add", "test.txt"])
            .current_dir(repo_path)
            .output()
            .expect("Failed to add file");

        Command::new("git")
            .args(["commit", "-m", "feat: initial commit"])
            .current_dir(repo_path)
            .output()
            .expect("Failed to commit");

        // Modify file and create second commit
        let mut file = File::create(&file_path).unwrap();
        writeln!(file, "Modified content").unwrap();

        Command::new("git")
            .args(["add", "test.txt"])
            .current_dir(repo_path)
            .output()
            .expect("Failed to add file");

        Command::new("git")
            .args(["commit", "-m", "fix: update content"])
            .current_dir(repo_path)
            .output()
            .expect("Failed to commit");

        temp
    }

    #[test]
    fn test_collector_new() {
        let temp = create_test_repo();
        let collector = GitCollector::new(temp.path()).unwrap();
        assert!(collector.repo.is_some());
    }

    #[test]
    fn test_is_git_repository() {
        let temp = create_test_repo();
        assert!(GitCollector::is_git_repository(temp.path()));

        let non_repo = TempDir::new().unwrap();
        assert!(!GitCollector::is_git_repository(non_repo.path()));
    }

    #[test]
    fn test_collect_commits() {
        let temp = create_test_repo();
        let collector = GitCollector::new(temp.path()).unwrap();
        let commits = collector.collect_all_commits(None).unwrap();

        assert!(commits.len() >= 2);
        assert_eq!(commits[0].conv_type.as_deref(), Some("fix"));
        assert_eq!(commits[1].conv_type.as_deref(), Some("feat"));
    }

    #[test]
    fn test_collect_commits_limited() {
        let temp = create_test_repo();
        let collector = GitCollector::new(temp.path()).unwrap();
        let commits = collector.collect_all_commits(Some(1)).unwrap();

        assert_eq!(commits.len(), 1);
    }

    #[test]
    fn test_parse_conventional_commit() {
        let (typ, scope, breaking) = GitCollector::parse_conventional_commit("feat: add feature");
        assert_eq!(typ.as_deref(), Some("feat"));
        assert_eq!(scope, None);
        assert!(!breaking);

        let (typ, scope, breaking) = GitCollector::parse_conventional_commit("feat(api)!: breaking change");
        assert_eq!(typ.as_deref(), Some("feat"));
        assert_eq!(scope.as_deref(), Some("api"));
        assert!(breaking);

        let (typ, scope, breaking) = GitCollector::parse_conventional_commit("random message");
        assert_eq!(typ, None);
        assert_eq!(scope, None);
        assert!(!breaking);
    }

    #[test]
    fn test_extract_breaking_changes() {
        let breaking = GitCollector::extract_breaking_changes("feat!: this is breaking");
        assert_eq!(breaking.len(), 1);
        assert_eq!(breaking[0], "this is breaking");

        let breaking = GitCollector::extract_breaking_changes(
            "feat: add feature\n\nBREAKING CHANGE: this breaks everything"
        );
        assert_eq!(breaking.len(), 1);
        assert!(breaking[0].contains("breaks everything"));
    }

    #[test]
    fn test_file_history() {
        let temp = create_test_repo();
        let collector = GitCollector::new(temp.path()).unwrap();
        let history = collector.file_history("test.txt", None).unwrap();

        assert!(history.len() >= 2);
    }

    #[test]
    fn test_commits_since() {
        let temp = create_test_repo();
        let collector = GitCollector::new(temp.path()).unwrap();
        let since = Utc::now() - chrono::Duration::hours(1);
        let commits = collector.commits_since(since).unwrap();

        assert!(commits.len() >= 2);
    }
}
