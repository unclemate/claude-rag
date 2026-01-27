//! File model.

use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};

/// A source file in the project.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct File {
    /// Unique file ID.
    pub id: String,
    /// Project path this file belongs to.
    pub project_path: String,
    /// File path relative to project root.
    pub file_path: String,
    /// File language/type.
    pub language: Option<String>,
    /// File modification time.
    pub modified_at: DateTime<Utc>,
    /// File size in bytes.
    pub size: u64,
    /// SHA-256 hash of file content.
    pub content_hash: String,
    /// Whether file is indexed.
    pub indexed: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_file_new() {
        let file = File {
            id: "file-1".to_string(),
            project_path: "/test".to_string(),
            file_path: "src/main.rs".to_string(),
            language: Some("rust".to_string()),
            modified_at: Utc::now(),
            size: 1024,
            content_hash: "abc123".to_string(),
            indexed: false,
        };

        assert_eq!(file.id, "file-1");
    }
}
