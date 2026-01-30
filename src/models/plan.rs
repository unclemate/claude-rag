//! Plan model for Claude Code design documents.

use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};

/// A Claude Code plan (design document).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    /// Unique plan ID (filename without extension).
    pub id: String,
    /// Plan title (first heading).
    pub title: Option<String>,
    /// Full content of the plan.
    pub content: String,
    /// Plan file modification time.
    pub modified_at: DateTime<Utc>,
    /// Number of chunks when indexed.
    pub chunk_count: usize,
    /// Whether this plan has been indexed.
    pub indexed: bool,
}

/// Plan chunk for vector indexing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanChunk {
    /// Chunk ID.
    pub id: String,
    /// Parent plan ID.
    pub plan_id: String,
    /// Chunk content.
    pub content: String,
    /// Chunk section (heading level + title).
    pub section: Option<String>,
    /// Line range in original plan.
    pub line_range: (usize, usize),
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn test_plan_serialize() {
        let plan = Plan {
            id: "test-plan".to_string(),
            title: Some("Test Plan".to_string()),
            content: "# Content".to_string(),
            modified_at: Utc::now(),
            chunk_count: 5,
            indexed: false,
        };

        let json = serde_json::to_string(&plan).unwrap();
        assert!(json.contains("test-plan"));
        assert!(json.contains("Test Plan"));
    }

    #[test]
    fn test_plan_deserialize() {
        let json = r##"{
            "id": "test-plan",
            "title": "Test Plan",
            "content": "# Content",
            "modified_at": "2024-01-01T00:00:00Z",
            "chunk_count": 5,
            "indexed": false
        }"##;

        let plan: Plan = serde_json::from_str(json).unwrap();
        assert_eq!(plan.id, "test-plan");
        assert_eq!(plan.title, Some("Test Plan".to_string()));
    }

    #[test]
    fn test_plan_chunk() {
        let chunk = PlanChunk {
            id: "chunk-1".to_string(),
            plan_id: "plan-1".to_string(),
            content: "Chunk content".to_string(),
                section: Some("## Section".to_string()),
            line_range: (1, 10),
        };

        let json = serde_json::to_string(&chunk).unwrap();
        assert!(json.contains("chunk-1"));
        assert!(json.contains("plan-1"));
    }
}
