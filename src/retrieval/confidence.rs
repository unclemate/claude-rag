//! Confidence level calculation for time-aware retrieval.

use crate::models::ContentType;
use serde::{Deserialize, Serialize};

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
}
