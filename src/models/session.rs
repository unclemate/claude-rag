//! Session model.

use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};

/// A Claude Code session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    /// Unique session ID.
    pub id: String,
    /// Session title/summary.
    pub title: Option<String>,
    /// Project path for this session.
    pub project_path: String,
    /// Session start time.
    pub started_at: DateTime<Utc>,
    /// Session end time (None if active).
    pub ended_at: Option<DateTime<Utc>>,
    /// Number of messages in session.
    pub message_count: usize,
    /// Whether this session has been fully indexed.
    pub indexed: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_serialize() {
        let session = Session {
            id: "test-id".to_string(),
            title: Some("Test Session".to_string()),
            project_path: "/test/project".to_string(),
            started_at: Utc::now(),
            ended_at: None,
            message_count: 0,
            indexed: false,
        };

        let json = serde_json::to_string(&session).unwrap();
        assert!(json.contains("test-id"));
    }
}
