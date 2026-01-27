//! Enhanced query results with temporal confidence and Git context.

use crate::models::{ContentType, Commit};
use crate::retrieval::ConfidenceLevel;
use serde::{Deserialize, Serialize};

/// Enhanced query result with temporal weighting and Git context.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnhancedItem {
    /// Result ID.
    pub id: String,
    /// Content type.
    pub content_type: ContentType,
    /// Semantic similarity score (0-1).
    pub similarity: f32,
    /// Temporal weight based on confidence level (0-1.2).
    pub temporal_weight: f32,
    /// Final combined score.
    pub final_score: f32,
    /// Confidence level for this result.
    pub confidence_level: ConfidenceLevel,
    /// Human-readable age description.
    pub age_description: String,
    /// Git information (if applicable).
    pub git_info: Option<GitInfo>,
    /// Whether this content matches current Git HEAD.
    pub is_current: bool,
    /// Whether this content is deprecated.
    pub is_deprecated: bool,
    /// If deprecated, what supersedes this.
    pub superseded_by: Option<SupersededInfo>,
    /// Content snippet.
    pub content: String,
    /// Timestamp of content (Unix seconds).
    pub timestamp: i64,
}

/// Git-related information for a result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitInfo {
    /// Most recent commit that modified this content.
    pub commit_hash: String,
    /// Short commit hash (7 chars).
    pub short_hash: String,
    /// Commit message summary.
    pub commit_message: String,
    /// Commit date (Unix seconds).
    pub commit_date: i64,
    /// Author name.
    pub author: String,
    /// Whether the file has been modified since this commit.
    pub is_stale: bool,
}

/// Information about what supersedes deprecated content.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SupersededInfo {
    /// ID of content that supersedes this.
    pub newer_id: String,
    /// Description of what changed.
    pub reason: String,
    /// Link to the change (commit hash or similar).
    pub reference: String,
}

impl EnhancedItem {
    /// Create a new enhanced item.
    pub fn new(
        id: String,
        content_type: ContentType,
        similarity: f32,
        timestamp: i64,
        content: String,
    ) -> Self {
        Self {
            id,
            content_type,
            similarity,
            temporal_weight: 1.0,
            final_score: similarity,
            confidence_level: ConfidenceLevel::Low,
            age_description: String::new(),
            git_info: None,
            is_current: false,
            is_deprecated: false,
            superseded_by: None,
            content,
            timestamp,
        }
    }

    /// Set temporal weight and recalculate final score.
    pub fn with_temporal_weight(mut self, weight: f32) -> Self {
        self.temporal_weight = weight;
        self.final_score = self.similarity * weight;
        self
    }

    /// Set confidence level.
    pub fn with_confidence(mut self, level: ConfidenceLevel) -> Self {
        self.confidence_level = level;
        self.temporal_weight = level.base_weight();
        self.final_score = self.similarity * self.temporal_weight;
        self
    }

    /// Set age description.
    pub fn with_age_description(mut self, desc: impl Into<String>) -> Self {
        self.age_description = desc.into();
        self
    }

    /// Set Git information.
    pub fn with_git_info(mut self, info: GitInfo) -> Self {
        self.git_info = Some(info);
        self
    }

    /// Mark as current (matches Git HEAD).
    pub fn mark_current(mut self) -> Self {
        self.is_current = true;
        self.is_deprecated = false;
        self
    }

    /// Mark as deprecated.
    pub fn mark_deprecated(mut self, superseded_by: SupersededInfo) -> Self {
        self.is_deprecated = true;
        self.is_current = false;
        self.superseded_by = Some(superseded_by);
        self
    }

    /// Calculate age description from timestamp.
    pub fn calculate_age_description(&mut self, current_time: i64) {
        let age_seconds = current_time.saturating_sub(self.timestamp);

        self.age_description = if age_seconds < 3600 {
            "Just now".to_string()
        } else if age_seconds < 86400 {
            let hours = age_seconds / 3600;
            format!("{} hour{} ago", hours, if hours > 1 { "s" } else { "" })
        } else if age_seconds < 604800 {
            let days = age_seconds / 86400;
            format!("{} day{} ago", days, if days > 1 { "s" } else { "" })
        } else if age_seconds < 2592000 {
            let weeks = age_seconds / 604800;
            format!("{} week{} ago", weeks, if weeks > 1 { "s" } else { "" })
        } else {
            let months = age_seconds / 2592000;
            format!("{} month{} ago", months, if months > 1 { "s" } else { "" })
        };
    }

    /// Create GitInfo from a Commit.
    pub fn git_info_from_commit(commit: &Commit, is_stale: bool) -> GitInfo {
        GitInfo {
            commit_hash: commit.id.clone(),
            short_hash: commit.short_hash.clone(),
            commit_message: commit.message_summary.clone(),
            commit_date: commit.commit_date.timestamp(),
            author: commit.author_name.clone(),
            is_stale,
        }
    }
}

impl GitInfo {
    /// Create new GitInfo.
    pub fn new(
        commit_hash: String,
        short_hash: String,
        commit_message: String,
        commit_date: i64,
        author: String,
        is_stale: bool,
    ) -> Self {
        Self {
            commit_hash,
            short_hash,
            commit_message,
            commit_date,
            author,
            is_stale,
        }
    }

    /// Get commit link for GitHub/GitLab etc.
    pub fn commit_link(&self, base_url: Option<&str>) -> String {
        if let Some(base) = base_url {
            format!("{}/commit/{}", base, self.short_hash)
        } else {
            self.short_hash.clone()
        }
    }
}

impl SupersededInfo {
    /// Create new SupersededInfo.
    pub fn new(newer_id: String, reason: String, reference: String) -> Self {
        Self {
            newer_id,
            reason,
            reference,
        }
    }

    /// Create from commit.
    pub fn from_commit(commit: &Commit, reason: String) -> Self {
        Self {
            newer_id: commit.id.clone(),
            reason,
            reference: commit.short_hash.clone(),
        }
    }
}

/// Builder for EnhancedItem.
pub struct EnhancedItemBuilder {
    id: String,
    content_type: ContentType,
    similarity: f32,
    timestamp: i64,
    content: String,
}

impl EnhancedItemBuilder {
    /// Create a new builder.
    pub fn new(
        id: impl Into<String>,
        content_type: ContentType,
        similarity: f32,
        timestamp: i64,
        content: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            content_type,
            similarity,
            timestamp,
            content: content.into(),
        }
    }

    /// Build the EnhancedItem.
    pub fn build(self) -> EnhancedItem {
        EnhancedItem::new(
            self.id,
            self.content_type,
            self.similarity,
            self.timestamp,
            self.content,
        )
    }

    /// Build with confidence level.
    pub fn with_confidence(self, level: ConfidenceLevel) -> EnhancedItem {
        self.build().with_confidence(level)
    }

    /// Build as current.
    pub fn as_current(self) -> EnhancedItem {
        self.build().mark_current()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_enhanced_item_new() {
        let item = EnhancedItem::new(
            "test-id".to_string(),
            ContentType::Message,
            0.85,
            1640000000,
            "test content".to_string(),
        );

        assert_eq!(item.id, "test-id");
        assert_eq!(item.similarity, 0.85);
        assert_eq!(item.final_score, 0.85);
        assert!(!item.is_current);
        assert!(!item.is_deprecated);
    }

    #[test]
    fn test_enhanced_item_with_temporal_weight() {
        let item = EnhancedItem::new(
            "test-id".to_string(),
            ContentType::Message,
            0.85,
            1640000000,
            "test content".to_string(),
        )
        .with_temporal_weight(1.2);

        assert_eq!(item.temporal_weight, 1.2);
        assert_eq!(item.final_score, 0.85 * 1.2);
    }

    #[test]
    fn test_enhanced_item_with_confidence() {
        let item = EnhancedItem::new(
            "test-id".to_string(),
            ContentType::Message,
            0.85,
            1640000000,
            "test content".to_string(),
        )
        .with_confidence(ConfidenceLevel::Highest);

        assert_eq!(item.confidence_level, ConfidenceLevel::Highest);
        assert_eq!(item.temporal_weight, 1.2);
        assert_eq!(item.final_score, 0.85 * 1.2);
    }

    #[test]
    fn test_enhanced_item_mark_current() {
        let item = EnhancedItem::new(
            "test-id".to_string(),
            ContentType::File,
            0.85,
            1640000000,
            "test content".to_string(),
        )
        .mark_current();

        assert!(item.is_current);
        assert!(!item.is_deprecated);
    }

    #[test]
    fn test_enhanced_item_mark_deprecated() {
        let superseded = SupersededInfo::new(
            "new-id".to_string(),
            "Replaced by new implementation".to_string(),
            "abc123".to_string(),
        );

        let item = EnhancedItem::new(
            "test-id".to_string(),
            ContentType::File,
            0.85,
            1640000000,
            "test content".to_string(),
        )
        .mark_deprecated(superseded);

        assert!(item.is_deprecated);
        assert!(!item.is_current);
        assert!(item.superseded_by.is_some());
        assert_eq!(item.superseded_by.as_ref().unwrap().newer_id, "new-id");
    }

    #[test]
    fn test_age_description_calculations() {
        let current = 1640000000;

        let mut item_now = EnhancedItem::new(
            "test-id".to_string(),
            ContentType::Message,
            0.85,
            current - 100, // 100 seconds ago
            "test content".to_string(),
        );
        item_now.calculate_age_description(current);
        assert_eq!(item_now.age_description, "Just now");

        let mut item_hours = EnhancedItem::new(
            "test-id".to_string(),
            ContentType::Message,
            0.85,
            current - 7200, // 2 hours ago
            "test content".to_string(),
        );
        item_hours.calculate_age_description(current);
        assert_eq!(item_hours.age_description, "2 hours ago");

        let mut item_days = EnhancedItem::new(
            "test-id".to_string(),
            ContentType::Message,
            0.85,
            current - 172800, // 2 days ago
            "test content".to_string(),
        );
        item_days.calculate_age_description(current);
        assert_eq!(item_days.age_description, "2 days ago");

        let mut item_weeks = EnhancedItem::new(
            "test-id".to_string(),
            ContentType::Message,
            0.85,
            current - 1209600, // 2 weeks ago
            "test content".to_string(),
        );
        item_weeks.calculate_age_description(current);
        assert_eq!(item_weeks.age_description, "2 weeks ago");

        let mut item_months = EnhancedItem::new(
            "test-id".to_string(),
            ContentType::Message,
            0.85,
            current - 5184000, // 2 months ago
            "test content".to_string(),
        );
        item_months.calculate_age_description(current);
        assert_eq!(item_months.age_description, "2 months ago");
    }

    #[test]
    fn test_git_info_new() {
        let git_info = GitInfo::new(
            "abc123def456".to_string(),
            "abc123d".to_string(),
            "Fix bug in parser".to_string(),
            1640000000,
            "Alice".to_string(),
            false,
        );

        assert_eq!(git_info.short_hash, "abc123d");
        assert_eq!(git_info.author, "Alice");
        assert!(!git_info.is_stale);
    }

    #[test]
    fn test_git_info_commit_link() {
        let git_info = GitInfo::new(
            "abc123def456".to_string(),
            "abc123d".to_string(),
            "Fix bug".to_string(),
            1640000000,
            "Alice".to_string(),
            false,
        );

        assert_eq!(git_info.commit_link(None), "abc123d");
        assert_eq!(
            git_info.commit_link(Some("https://github.com/user/repo")),
            "https://github.com/user/repo/commit/abc123d"
        );
    }

    #[test]
    fn test_superseded_info_new() {
        let info = SupersededInfo::new(
            "new-id".to_string(),
            "Better implementation".to_string(),
            "def456".to_string(),
        );

        assert_eq!(info.newer_id, "new-id");
        assert_eq!(info.reason, "Better implementation");
        assert_eq!(info.reference, "def456");
    }

    #[test]
    fn test_enhanced_item_builder() {
        let item = EnhancedItemBuilder::new(
            "test-id".to_string(),
            ContentType::Message,
            0.85,
            1640000000,
            "test content",
        )
        .build();

        assert_eq!(item.id, "test-id");
        assert_eq!(item.content_type, ContentType::Message);
    }

    #[test]
    fn test_enhanced_item_builder_with_confidence() {
        let item = EnhancedItemBuilder::new(
            "test-id".to_string(),
            ContentType::File,
            0.85,
            1640000000,
            "test content",
        )
        .with_confidence(ConfidenceLevel::High)
        .mark_current();

        assert_eq!(item.confidence_level, ConfidenceLevel::High);
        assert!(item.is_current);
    }
}
