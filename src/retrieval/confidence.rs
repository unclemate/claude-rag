//! Confidence level calculation for time-aware retrieval.

use crate::models::ContentType;
use serde::{Deserialize, Serialize};
use tracing::trace;

/// Confidence level for indexed content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ConfidenceLevel {
    /// Highest confidence - current code (matches Git HEAD).
    Highest = 5,
    /// High confidence - Git commits/diffs.
    High = 4,
    /// Medium confidence - Recent sessions (≤7 days).
    Medium = 3,
    /// Low confidence - Old sessions (7-30 days).
    Low = 2,
    /// Lowest confidence - Stale discussions (>30 days).
    Lowest = 1,
}

impl ConfidenceLevel {
    /// Get the base weight for this confidence level.
    pub fn base_weight(&self) -> f32 {
        match self {
            Self::Highest => 1.2,
            Self::High => 1.0,
            Self::Medium => 0.85,
            Self::Low => 0.6,
            Self::Lowest => 0.4,
        }
    }
}

/// Confidence score for a retrieval result.
#[derive(Debug, Clone, Copy)]
pub struct ConfidenceScore {
    /// Confidence level.
    pub level: ConfidenceLevel,
    /// Calculated weight.
    pub weight: f32,
}

impl ConfidenceScore {
    /// Calculate confidence from content type and age (days).
    pub fn from_content_type(content_type: ContentType, age_days: i64) -> Self {
        let level = match content_type {
            ContentType::File | ContentType::Symbol => {
                // Current code has highest confidence
                ConfidenceLevel::Highest
            }
            ContentType::Commit | ContentType::GitDiff => {
                // Git commits have high confidence
                ConfidenceLevel::High
            }
            ContentType::Plan => {
                // Plans have high confidence (design documents)
                ConfidenceLevel::High
            }
            ContentType::Message | ContentType::Session => {
                // Sessions decay over time
                if age_days <= 7 {
                    ConfidenceLevel::Medium
                } else if age_days <= 30 {
                    ConfidenceLevel::Low
                } else {
                    ConfidenceLevel::Lowest
                }
            }
        };

        trace!(
            "Calculated confidence: type={:?}, age_days={} -> level={:?}, weight={}",
            content_type, age_days, level, level.base_weight()
        );

        Self {
            level,
            weight: level.base_weight(),
        }
    }

    /// Calculate confidence from content type, age, and Git status.
    ///
    /// This method considers whether the content matches the current Git HEAD
    /// when determining confidence for files and symbols.
    ///
    /// # Arguments
    ///
    /// * `content_type` - Type of content
    /// * `age_days` - Age in days since indexing
    /// * `is_git_current` - Whether content matches Git HEAD
    ///
    /// # Returns
    ///
    /// Returns a confidence score with appropriate level and weight.
    pub fn from_content_type_with_git(
        content_type: ContentType,
        age_days: i64,
        is_git_current: bool,
    ) -> Self {
        let level = match content_type {
            ContentType::File | ContentType::Symbol => {
                if is_git_current {
                    // Current code has highest confidence
                    ConfidenceLevel::Highest
                } else {
                    // Deprecated code has low confidence
                    ConfidenceLevel::Low
                }
            }
            ContentType::Commit | ContentType::GitDiff => {
                // Git commits have high confidence
                ConfidenceLevel::High
            }
            ContentType::Plan => {
                // Plans have high confidence (design documents)
                ConfidenceLevel::High
            }
            ContentType::Message | ContentType::Session => {
                // Sessions decay over time
                if age_days <= 7 {
                    ConfidenceLevel::Medium
                } else if age_days <= 30 {
                    ConfidenceLevel::Low
                } else {
                    ConfidenceLevel::Lowest
                }
            }
        };

        Self {
            level,
            weight: level.base_weight(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_confidence_level_weights() {
        assert_eq!(ConfidenceLevel::Highest.base_weight(), 1.2);
        assert_eq!(ConfidenceLevel::High.base_weight(), 1.0);
        assert_eq!(ConfidenceLevel::Medium.base_weight(), 0.85);
    }

    #[test]
    fn test_confidence_from_content_type() {
        let score = ConfidenceScore::from_content_type(ContentType::File, 0);
        assert_eq!(score.level, ConfidenceLevel::Highest);

        let score = ConfidenceScore::from_content_type(ContentType::Message, 3);
        assert_eq!(score.level, ConfidenceLevel::Medium);

        let score = ConfidenceScore::from_content_type(ContentType::Message, 40);
        assert_eq!(score.level, ConfidenceLevel::Lowest);
    }

    #[test]
    fn test_confidence_from_content_type_with_git() {
        // Current file should have highest confidence
        let score =
            ConfidenceScore::from_content_type_with_git(ContentType::File, 0, true);
        assert_eq!(score.level, ConfidenceLevel::Highest);
        assert_eq!(score.weight, 1.2);

        // Deprecated file should have low confidence
        let score =
            ConfidenceScore::from_content_type_with_git(ContentType::File, 0, false);
        assert_eq!(score.level, ConfidenceLevel::Low);
        assert_eq!(score.weight, 0.6);

        // Current symbol should have highest confidence
        let score =
            ConfidenceScore::from_content_type_with_git(ContentType::Symbol, 10, true);
        assert_eq!(score.level, ConfidenceLevel::Highest);

        // Deprecated symbol should have low confidence
        let score =
            ConfidenceScore::from_content_type_with_git(ContentType::Symbol, 10, false);
        assert_eq!(score.level, ConfidenceLevel::Low);

        // Git commits are unaffected by git_current parameter
        let score =
            ConfidenceScore::from_content_type_with_git(ContentType::Commit, 5, true);
        assert_eq!(score.level, ConfidenceLevel::High);

        let score =
            ConfidenceScore::from_content_type_with_git(ContentType::Commit, 5, false);
        assert_eq!(score.level, ConfidenceLevel::High);

        // Recent session with medium confidence
        let score =
            ConfidenceScore::from_content_type_with_git(ContentType::Session, 3, true);
        assert_eq!(score.level, ConfidenceLevel::Medium);

        // Old session with lowest confidence
        let score =
            ConfidenceScore::from_content_type_with_git(ContentType::Message, 40, true);
        assert_eq!(score.level, ConfidenceLevel::Lowest);
    }
}
