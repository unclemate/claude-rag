//! Message model.

use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};

/// Message role in the conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Role {
    /// User message.
    User,
    /// Assistant (AI) message.
    Assistant,
    /// System message.
    System,
}

/// A single message within a session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    /// Unique message ID.
    pub id: String,
    /// Session ID this message belongs to.
    pub session_id: String,
    /// Message role.
    pub role: Role,
    /// Message content.
    pub content: String,
    /// Message timestamp.
    pub timestamp: DateTime<Utc>,
    /// Token count (if available).
    pub tokens: Option<usize>,
    /// Model used for assistant messages.
    pub model: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_role_equality() {
        assert_eq!(Role::User, Role::User);
        assert_ne!(Role::User, Role::Assistant);
    }

    #[test]
    fn test_message_serialize() {
        let msg = Message {
            id: "msg-1".to_string(),
            session_id: "session-1".to_string(),
            role: Role::User,
            content: "Hello".to_string(),
            timestamp: Utc::now(),
            tokens: Some(5),
            model: None,
        };

        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("Hello"));
    }
}
