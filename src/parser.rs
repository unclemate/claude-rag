//! Session JSONL parsing and project scanning.
//!
//! This module handles scanning for Claude Code projects and parsing
//! session files from the `.claude/sessions/` directory structure.

use crate::error::{RagError, Result};
use crate::models::{Message, Role, Session};
use chrono::{DateTime, Utc};
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// Default Claude sessions directory name.
const SESSIONS_DIR: &str = ".claude/sessions";
/// Session index file name.
const INDEX_FILE: &str = "index.jsonl";

/// Session metadata from the index file.
#[derive(Debug, Clone)]
pub struct SessionMeta {
    /// Session ID.
    pub id: String,
    /// Session title.
    pub title: Option<String>,
    /// Project path.
    pub project_path: String,
    /// Creation timestamp.
    pub created_at: DateTime<Utc>,
}

/// Parsed session with messages.
#[derive(Debug, Clone)]
pub struct ParsedSession {
    /// Session metadata.
    pub session: Session,
    /// Messages in the session.
    pub messages: Vec<Message>,
}

/// Claude session parser.
pub struct SessionParser {
    /// Claude config directory (usually ~/.claude).
    config_dir: PathBuf,
}

impl SessionParser {
    /// Create a new session parser.
    ///
    /// # Arguments
    /// * `config_dir` - Path to Claude config directory (defaults to ~/.claude)
    pub fn new(config_dir: Option<PathBuf>) -> Self {
        let config_dir = config_dir.unwrap_or_else(|| {
            dirs::home_dir()
                .expect("Unable to determine home directory")
                .join(".claude")
        });
        Self { config_dir }
    }

    /// Scan for all Claude projects with sessions.
    ///
    /// Returns a map of project paths to their session directories.
    ///
    /// # Returns
    /// * `HashMap<String, PathBuf>` - Project path -> sessions directory
    pub fn scan_claude_projects(&self) -> Result<HashMap<String, PathBuf>> {
        let mut projects = HashMap::new();

        let sessions_dir = self.config_dir.join("sessions");
        if !sessions_dir.exists() {
            return Ok(projects);
        }

        // Each subdirectory in .claude/sessions/ corresponds to a project
        for entry in fs::read_dir(&sessions_dir)
            .map_err(RagError::Io)?
        {
            let entry = entry.map_err(RagError::Io)?;
            let path = entry.path();

            // Skip if not a directory
            if !path.is_dir() {
                continue;
            }

            // The directory name should be the project path (with / replaced by _)
            // For now, we'll use the directory name as the session ID
            if let Some(session_dir) = path.to_str() {
                projects.insert(session_dir.to_string(), path);
            }
        }

        Ok(projects)
    }

    /// Parse session metadata from index.jsonl.
    ///
    /// # Arguments
    /// * `session_dir` - Path to the session directory
    ///
    /// # Returns
    /// * `SessionMeta` - Parsed session metadata
    pub fn parse_sessions_index(&self, session_dir: &Path) -> Result<SessionMeta> {
        let index_path = session_dir.join(INDEX_FILE);
        if !index_path.exists() {
            return Err(RagError::NotFound(index_path.display().to_string()));
        }

        // Read the index.jsonl file (first line should contain session metadata)
        let content = fs::read_to_string(&index_path)
            .map_err(RagError::Io)?;

        // Parse first line as session metadata
        let first_line = content
            .lines()
            .next()
            .ok_or_else(|| RagError::Parse("Empty index file".to_string()))?;

        let json: Value = serde_json::from_str(first_line)
            .map_err(|e| RagError::Parse(format!("Invalid JSON in index: {e}")))?;

        // Extract session ID from directory name
        let id = session_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();

        // Parse title
        let title = json
            .get("title")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        // Parse project path (may be stored differently depending on Claude version)
        let project_path = json
            .get("projectPath")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .or_else(|| {
                // Try to derive from session directory structure
                session_dir
                    .parent()
                    .and_then(|p| p.to_str())
                    .map(|s| s.to_string())
            })
            .unwrap_or_else(|| ".".to_string());

        // Parse creation time
        let created_at = json
            .get("createdAt")
            .and_then(|v| v.as_str())
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|dt: DateTime<chrono::FixedOffset>| dt.with_timezone(&Utc))
            .unwrap_or_else(Utc::now);

        Ok(SessionMeta {
            id,
            title,
            project_path,
            created_at,
        })
    }

    /// Parse a JSONL session file into messages.
    ///
    /// # Arguments
    /// * `session_dir` - Path to the session directory
    /// * `meta` - Session metadata
    ///
    /// # Returns
    /// * `ParsedSession` - Session with parsed messages
    pub fn parse_jsonl_file(&self, session_dir: &Path, meta: SessionMeta) -> Result<ParsedSession> {
        let index_path = session_dir.join(INDEX_FILE);
        if !index_path.exists() {
            return Err(RagError::NotFound(index_path.display().to_string()));
        }

        let content = fs::read_to_string(&index_path)
            .map_err(RagError::Io)?;

        let mut messages = Vec::new();
        let mut message_count = 0;

        // Parse each line as a separate JSON object
        for (line_num, line) in content.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }

            let json: Value = serde_json::from_str(line)
                .map_err(|e| {
                    RagError::Parse(format!("Invalid JSON on line {}: {}", line_num + 1, e))
                })?;

            // Check if this is a message (has 'role' and 'content' fields)
            if let (Some(role), Some(content)) = (json.get("role"), json.get("content")) {
                if let (Some(role_str), Some(content_str)) = (role.as_str(), content.as_str()) {
                    let role = match role_str {
                        "user" => Role::User,
                        "assistant" => Role::Assistant,
                        "system" => Role::System,
                        _ => continue, // Skip unknown roles
                    };

                    // Parse timestamp
                    let timestamp = json
                        .get("timestamp")
                        .and_then(|v| v.as_str())
                        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                        .map(|dt: DateTime<chrono::FixedOffset>| dt.with_timezone(&Utc))
                        .unwrap_or_else(Utc::now);

                    // Parse token count
                    let tokens = json
                        .get("tokens")
                        .and_then(|v| v.as_u64())
                        .map(|v| v as usize);

                    // Parse model (for assistant messages)
                    let model = json
                        .get("model")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());

                    let message = Message {
                        id: format!("{}-msg-{}", meta.id, line_num),
                        session_id: meta.id.clone(),
                        role,
                        content: content_str.to_string(),
                        timestamp,
                        tokens,
                        model,
                    };

                    messages.push(message);
                    message_count += 1;
                }
            }
        }

        // Create session object
        let session = Session {
            id: meta.id,
            title: meta.title,
            project_path: meta.project_path,
            started_at: meta.created_at,
            ended_at: None, // Will be updated if session ends
            message_count,
            indexed: false, // Will be set to true after indexing
        };

        Ok(ParsedSession { session, messages })
    }

    /// Parse a session with full metadata and messages.
    ///
    /// # Arguments
    /// * `session_dir` - Path to the session directory
    ///
    /// # Returns
    /// * `ParsedSession` - Fully parsed session
    pub fn parse_session(&self, session_dir: &Path) -> Result<ParsedSession> {
        let meta = self.parse_sessions_index(session_dir)?;
        self.parse_jsonl_file(session_dir, meta)
    }

    /// Parse all sessions from a project directory.
    ///
    /// # Arguments
    /// * `project_path` - Path to the project
    ///
    /// # Returns
    /// * `Vec<ParsedSession>` - All parsed sessions
    pub fn parse_project_sessions(&self, project_path: &Path) -> Result<Vec<ParsedSession>> {
        let sessions_dir = project_path.join(SESSIONS_DIR);
        if !sessions_dir.exists() {
            return Ok(Vec::new());
        }

        let mut sessions = Vec::new();

        for entry in WalkDir::new(&sessions_dir)
            .min_depth(1)
            .max_depth(1)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if path.is_dir() {
                if let Ok(session) = self.parse_session(path) {
                    sessions.push(session);
                }
                // Skip sessions that fail to parse (log warning in real implementation)
            }
        }

        Ok(sessions)
    }

    /// Check incremental parsing state.
    ///
    /// # Arguments
    /// * `session_id` - Session identifier
    /// * `storage` - Storage manager to check index state
    ///
    /// # Returns
    /// * `bool` - true if session needs re-indexing
    pub fn needs_indexing(&self, session_dir: &Path, last_indexed: Option<DateTime<Utc>>) -> Result<bool> {
        let index_path = session_dir.join(INDEX_FILE);
        if !index_path.exists() {
            return Ok(true);
        }

        // Check file modification time
        let metadata = fs::metadata(&index_path)
            .map_err(RagError::Io)?;
        let modified = metadata.modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| DateTime::from_timestamp(d.as_secs() as i64, 0).unwrap_or_else(Utc::now));

        if let Some(last_indexed) = last_indexed {
            Ok(modified.is_none_or(|m| m > last_indexed))
        } else {
            Ok(true)
        }
    }
}

impl Default for SessionParser {
    fn default() -> Self {
        Self::new(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::create_dir_all;
    use tempfile::TempDir;

    fn create_test_session(dir: &Path, content: &str) -> PathBuf {
        let session_dir = dir.join("test-session");
        create_dir_all(&session_dir).unwrap();
        let index_path = session_dir.join(INDEX_FILE);
        fs::write(&index_path, content).unwrap();
        session_dir
    }

    #[test]
    fn test_parser_new() {
        let parser = SessionParser::new(None);
        assert_eq!(parser.config_dir, dirs::home_dir().unwrap().join(".claude"));
    }

    #[test]
    fn test_parser_default() {
        let parser = SessionParser::default();
        assert!(parser.config_dir.ends_with(".claude"));
    }

    #[test]
    fn test_parse_empty_index() {
        let temp = TempDir::new().unwrap();
        let content = r#"{"title":"Test","projectPath":"/test","createdAt":"2024-01-01T00:00:00Z"}"#;
        let session_dir = create_test_session(temp.path(), content);

        let parser = SessionParser::default();
        let meta = parser.parse_sessions_index(&session_dir).unwrap();

        assert_eq!(meta.title, Some("Test".to_string()));
        assert_eq!(meta.project_path, "/test".to_string());
    }

    #[test]
    fn test_parse_session_with_messages() {
        let temp = TempDir::new().unwrap();
        let content = r#"{"title":"Test","projectPath":"/test","createdAt":"2024-01-01T00:00:00Z"}
{"role":"user","content":"Hello","timestamp":"2024-01-01T00:00:00Z"}
{"role":"assistant","content":"Hi there","timestamp":"2024-01-01T00:00:01Z","model":"claude-3"}"#;
        let session_dir = create_test_session(temp.path(), content);

        let parser = SessionParser::default();
        let parsed = parser.parse_session(&session_dir).unwrap();

        assert_eq!(parsed.session.message_count, 2);
        assert_eq!(parsed.messages.len(), 2);
        assert_eq!(parsed.messages[0].role, Role::User);
        assert_eq!(parsed.messages[0].content, "Hello");
        assert_eq!(parsed.messages[1].role, Role::Assistant);
        assert_eq!(parsed.messages[1].content, "Hi there");
        assert_eq!(parsed.messages[1].model, Some("claude-3".to_string()));
    }

    #[test]
    fn test_parse_malformed_json() {
        let temp = TempDir::new().unwrap();
        let content = r#"{"title":"Test"}
invalid json here
{"role":"user","content":"Hello"}"#;
        let session_dir = create_test_session(temp.path(), content);

        let parser = SessionParser::default();
        let result = parser.parse_session(&session_dir);

        assert!(result.is_err());
    }

    #[test]
    fn test_scan_nonexistent_projects() {
        let temp = TempDir::new().unwrap();
        let parser = SessionParser::new(Some(temp.path().to_path_buf()));

        let projects = parser.scan_claude_projects().unwrap();
        assert!(projects.is_empty());
    }

    #[test]
    fn test_needs_indexing() {
        let temp = TempDir::new().unwrap();
        let content = r#"{"title":"Test"}"#;
        let session_dir = create_test_session(temp.path(), content);

        let parser = SessionParser::default();

        // Never indexed -> needs indexing
        assert!(parser.needs_indexing(&session_dir, None).unwrap());

        // Indexed in the past -> needs indexing
        let past = Utc::now() - chrono::Duration::days(1);
        assert!(parser.needs_indexing(&session_dir, Some(past)).unwrap());
    }
}
