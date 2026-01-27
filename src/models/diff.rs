//! Git diff model.

use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};

/// A Git diff for a file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitDiff {
    /// Unique diff ID.
    pub id: String,
    /// Commit ID this diff belongs to.
    pub commit_id: String,
    /// Project path.
    pub project_path: String,
    /// File path (relative to project root).
    pub file_path: String,
    /// Old object ID.
    pub old_oid: Option<String>,
    /// New object ID.
    pub new_oid: Option<String>,
    /// Change type: added, modified, deleted, renamed.
    pub change_type: ChangeType,
    /// Full diff content.
    pub diff_content: String,
    /// Diff summary.
    pub diff_summary: String,
    /// Number of lines added.
    pub added_lines: usize,
    /// Number of lines removed.
    pub removed_lines: usize,
    /// Number of lines added (hunk headers only).
    pub insertions: usize,
    /// Number of lines removed (hunk headers only).
    pub deletions: usize,
    /// Timestamp.
    pub timestamp: DateTime<Utc>,
}

/// Change type for a file diff.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ChangeType {
    /// File was added.
    Added,
    /// File was modified.
    Modified,
    /// File was deleted.
    Deleted,
    /// File was renamed.
    Renamed,
    /// File was copied.
    Copied,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_change_type() {
        assert_eq!(ChangeType::Added, ChangeType::Added);
        assert_ne!(ChangeType::Added, ChangeType::Modified);
    }

    #[test]
    fn test_git_diff_new() {
        let diff = GitDiff {
            id: "diff-1".to_string(),
            commit_id: "commit-1".to_string(),
            project_path: "/test".to_string(),
            file_path: "src/main.rs".to_string(),
            old_oid: Some("old123".to_string()),
            new_oid: Some("new456".to_string()),
            change_type: ChangeType::Modified,
            diff_content: "-old\n+new".to_string(),
            diff_summary: "1 file changed".to_string(),
            added_lines: 1,
            removed_lines: 1,
            insertions: 1,
            deletions: 1,
            timestamp: Utc::now(),
        };

        assert_eq!(diff.change_type, ChangeType::Modified);
    }
}
