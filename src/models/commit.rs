//! Git commit model.

use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};

/// A Git commit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Commit {
    /// Unique commit ID (full hash).
    pub id: String,
    /// Short commit hash (7 characters).
    pub short_hash: String,
    /// Project path this commit belongs to.
    pub project_path: String,
    /// Author name.
    pub author_name: String,
    /// Author email.
    pub author_email: String,
    /// Commit date.
    pub commit_date: DateTime<Utc>,
    /// Commit message.
    pub message: String,
    /// Message summary (first line).
    pub message_summary: String,
    /// Conventional commit type (if applicable).
    pub conv_type: Option<String>,
    /// Conventional commit scope (if applicable).
    pub conv_scope: Option<String>,
    /// Whether this is a breaking change.
    pub is_breaking: bool,
    /// Parent commit hashes.
    pub parent_hashes: Vec<String>,
    /// Number of files changed.
    pub files_changed: usize,
    /// Number of insertions.
    pub insertions: usize,
    /// Number of deletions.
    pub deletions: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_commit_new() {
        let commit = Commit {
            id: "abc123def456".to_string(),
            short_hash: "abc123d".to_string(),
            project_path: "/test".to_string(),
            author_name: "Test Author".to_string(),
            author_email: "test@example.com".to_string(),
            commit_date: Utc::now(),
            message: "feat: add feature".to_string(),
            message_summary: "feat: add feature".to_string(),
            conv_type: Some("feat".to_string()),
            conv_scope: None,
            is_breaking: false,
            parent_hashes: vec![],
            files_changed: 1,
            insertions: 10,
            deletions: 0,
        };

        assert_eq!(commit.conv_type.as_deref(), Some("feat"));
    }
}
