//! Session JSONL parsing and project scanning.
//!
//! This module handles scanning for Claude Code projects and parsing
//! session files from the new `.claude/projects/<encoded-path>/` directory structure.

use crate::error::{RagError, Result};
use crate::models::{Message, Role, Session};
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

/// Session index file name (new format).
const INDEX_FILE: &str = "sessions-index.json";

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

    /// Encode a project path to the format used in directory names.
    ///
    /// Uses a simple character substitution to avoid path collisions.
    /// Order: 1) Replace original dashes with underscores, 2) Replace slashes with dashes
    ///
    /// Examples:
    /// - `/home/changh/Projects/claude-rag` → `-home-changh-Projects-claude_rag`
    /// - `/a/b` → `-a-b`
    /// - `/a-b` → `-a_b` (distinct from `-a-b`)
    pub fn encode_project_path(project_path: &Path) -> String {
        project_path
            .to_str()
            .map(|s| {
                // First: replace original dashes with underscores
                let with_dash_replaced = s.replace('-', "_");
                // Second: replace path separators (slashes) with dashes
                with_dash_replaced.replace('/', "-")
            })
            .unwrap_or_default()
    }

    /// Try to find the project directory using either new or old encoding format.
    ///
    /// This provides backward compatibility with directories created using the old encoding.
    fn get_project_dir_with_fallback(&self, project_path: &Path) -> Result<PathBuf> {
        // Try new encoding first
        let encoded_new = Self::encode_project_path(project_path);
        let project_dir_new = self.config_dir.join("projects").join(&encoded_new);

        if project_dir_new.exists() {
            return Ok(project_dir_new);
        }

        // Fallback to old encoding (simple slash replacement)
        let encoded_old = project_path
            .to_str()
            .map(|s| s.replace('/', "-"))
            .unwrap_or_default();
        let project_dir_old = self.config_dir.join("projects").join(&encoded_old);

        if project_dir_old.exists() {
            debug!("Using legacy encoding for project path: {}", project_path.display());
            return Ok(project_dir_old);
        }

        Err(RagError::NotFound(format!(
            "Project directory not found for {}. Tried encodings: {} (new), {} (old). Please ensure Claude Code has created sessions for this project.",
            project_path.display(),
            encoded_new,
            encoded_old
        )))
    }

    /// Decode a project path from the encoded directory name format.
    ///
    /// Reverses the encoding: 1) Replace dashes back to slashes, 2) Replace underscores back to dashes
    #[allow(dead_code)]
    fn decode_project_path(encoded: &str) -> String {
        // Reverse the encoding process
        let with_slash = encoded.replace('-', "/");
        let original = with_slash.replace('_', "-");
        original
    }

    /// Get the project directory for a specific project (new format).
    pub fn get_project_dir(&self, project_path: &Path) -> Result<PathBuf> {
        let encoded = Self::encode_project_path(project_path);
        let project_dir = self.config_dir.join("projects").join(&encoded);

        if !project_dir.exists() {
            return Err(RagError::NotFound(format!(
                "Project directory not found: {}. Please ensure Claude Code has created sessions for this project.",
                project_dir.display()
            )));
        }

        Ok(project_dir)
    }

    /// Scan for all Claude projects with sessions (new format only).
    ///
    /// Returns a map of session IDs to their .jsonl file paths.
    ///
    /// # Returns
    /// * `HashMap<String, PathBuf>` - Session ID -> .jsonl file path
    pub fn scan_claude_projects(&self) -> Result<HashMap<String, PathBuf>> {
        let projects_dir = self.config_dir.join("projects");

        // Check if new format exists
        if !projects_dir.exists() {
            // Check for old format and provide helpful error
            let sessions_dir = self.config_dir.join("sessions");
            if sessions_dir.exists() {
                warn!("Old session format detected. Please upgrade Claude Code to use the new session format.");
                return Err(RagError::Unsupported(
                    "Old session format detected. This version of claude-rag only supports the new Claude Code session format. Please upgrade Claude Code.".to_string()
                ));
            }
            return Ok(HashMap::new());
        }

        let mut projects = HashMap::new();
        let mut project_count = 0;

        // Scan all project directories - count during the same pass
        for entry in fs::read_dir(&projects_dir).map_err(RagError::Io)? {
            let entry = entry.map_err(RagError::Io)?;
            let path = entry.path();

            if !path.is_dir() {
                continue;
            }

            project_count += 1;

            // Scan session files in each project directory
            match self.scan_sessions_in_dir(&path) {
                Ok(sessions) => projects.extend(sessions),
                Err(e) => {
                    debug!("Failed to scan sessions in {}: {}", path.display(), e);
                }
            }
        }

        info!("Found {} sessions across {} projects", projects.len(), project_count);
        Ok(projects)
    }

    /// Scan session files in a specific project directory.
    fn scan_sessions_in_dir(&self, project_dir: &Path) -> Result<HashMap<String, PathBuf>> {
        let mut sessions = HashMap::new();

        for entry in fs::read_dir(project_dir).map_err(RagError::Io)? {
            let entry = entry.map_err(RagError::Io)?;
            let path = entry.path();

            // Only process .jsonl files (excluding sessions-index.json)
            if path.extension().and_then(|s| s.to_str()) != Some("jsonl") {
                continue;
            }

            // Skip sessions-index.json
            if path.file_name().and_then(|s| s.to_str()) == Some("sessions-index.json") {
                continue;
            }

            // Extract session ID from filename
            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                sessions.insert(stem.to_string(), path);
            }
        }

        Ok(sessions)
    }

    /// Get session file paths for a specific project.
    pub fn get_project_session_files(&self, project_path: &Path) -> Result<HashMap<String, PathBuf>> {
        let project_dir = self.get_project_dir(project_path)?;
        self.scan_sessions_in_dir(&project_dir)
    }

    /// Parse session metadata from sessions-index.json (new format).
    ///
    /// Reads the sessions-index.json file and extracts metadata for a specific session.
    ///
    /// # Arguments
    /// * `session_path` - Path to the .jsonl session file (used to derive session ID)
    ///
    /// # Returns
    /// * `SessionMeta` - Parsed session metadata including title, project path, and creation time
    pub fn parse_sessions_index(&self, session_path: &Path) -> Result<SessionMeta> {
        // New format: read from sessions-index.json
        let project_dir = session_path
            .parent()
            .ok_or_else(|| RagError::Parse("No parent directory".to_string()))?;

        let index_path = project_dir.join(INDEX_FILE);
        if !index_path.exists() {
            return Err(RagError::NotFound(format!(
                "sessions-index.json not found in {}",
                project_dir.display()
            )));
        }

        let content = fs::read_to_string(&index_path).map_err(RagError::Io)?;

        let index: serde_json::Value = serde_json::from_str(&content)
            .map_err(|e| RagError::Parse(format!("Invalid sessions index: {}", e)))?;

        let session_id = session_path
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or_else(|| RagError::Parse("Invalid session filename".to_string()))?;

        // Find entry matching session_id
        let entries = index["entries"]
            .as_array()
            .ok_or_else(|| RagError::Parse("No entries in index".to_string()))?;

        for entry in entries {
            if entry["sessionId"].as_str() == Some(session_id) {
                return Ok(SessionMeta {
                    id: session_id.to_string(),
                    title: entry["summary"].as_str().map(|s| s.to_string()),
                    project_path: entry["projectPath"].as_str()
                        .unwrap_or("/")
                        .to_string(),
                    created_at: entry["created"].as_str()
                        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                        .map(|dt| dt.with_timezone(&Utc))
                        .unwrap_or_else(Utc::now),
                });
            }
        }

        Err(RagError::NotFound(format!(
            "Session {} not in index",
            session_id
        )))
    }

    /// Parse a JSONL session file into messages (new format).
    ///
    /// # Arguments
    /// * `session_path` - Path to the .jsonl session file
    ///
    /// # Returns
    /// * `Vec<Message>` - Parsed messages
    pub fn parse_jsonl_file(&self, session_path: &Path) -> Result<Vec<Message>> {
        let content = fs::read_to_string(session_path).map_err(RagError::Io)?;
        self.parse_jsonl_new(&content)
    }

    /// Parse new format JSONL content.
    fn parse_jsonl_new(&self, content: &str) -> Result<Vec<Message>> {
        let mut messages = Vec::new();

        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            let json: serde_json::Value = serde_json::from_str(line).map_err(|_| {
                RagError::Parse(format!(
                    "Invalid JSON: {}",
                    &line[..50.min(line.len())]
                ))
            })?;

            // Skip non-message entries (snapshots, metadata, etc.)
            if json["type"].as_str() != Some("user")
                && json["type"].as_str() != Some("assistant")
            {
                continue;
            }

            // Extract nested message
            let msg = &json["message"];
            let role = match msg["role"].as_str() {
                Some("user") => Role::User,
                Some("assistant") => Role::Assistant,
                _ => continue,
            };

            let content = msg["content"].as_str().unwrap_or("").to_string();
            if content.is_empty() {
                continue;
            }

            // Validate required fields - skip if UUID or session_id is empty
            let uuid = json["uuid"].as_str().filter(|s| !s.is_empty());
            let session_id = json["sessionId"].as_str().filter(|s| !s.is_empty());

            if uuid.is_none() || session_id.is_none() {
                debug!("Skipping message with missing UUID or session_id");
                continue;
            }

            let timestamp = json["timestamp"]
                .as_str()
                .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(Utc::now);

            messages.push(Message {
                id: uuid.unwrap().to_string(),
                session_id: session_id.unwrap().to_string(),
                role,
                content,
                timestamp,
                tokens: msg["tokens"].as_u64().map(|t| t as usize),
                model: msg["model"].as_str().map(|s| s.to_string()),
            });
        }

        Ok(messages)
    }

    /// Parse a session with full metadata and messages.
    ///
    /// # Arguments
    /// * `session_path` - Path to the .jsonl session file
    ///
    /// # Returns
    /// * `ParsedSession` - Fully parsed session
    pub fn parse_session(&self, session_path: &Path) -> Result<ParsedSession> {
        let meta = self.parse_sessions_index(session_path)?;
        let messages = self.parse_jsonl_file(session_path)?;

        Ok(ParsedSession {
            session: Session {
                id: meta.id,
                title: meta.title,
                project_path: meta.project_path.clone(),
                started_at: meta.created_at,
                ended_at: None,
                message_count: messages.len(),
                indexed: false,
            },
            messages,
        })
    }

    /// Parse all sessions from a project directory (new format).
    ///
    /// # Arguments
    /// * `project_path` - Path to the project
    ///
    /// # Returns
    /// * `Vec<ParsedSession>` - All parsed sessions
    pub fn parse_project_sessions(&self, project_path: &Path) -> Result<Vec<ParsedSession>> {
        // Get project directory with fallback for legacy encoding
        let project_dir = self.get_project_dir_with_fallback(project_path)?;

        // Get session files
        let session_files = self.scan_sessions_in_dir(&project_dir)?;

        // Parse each session
        let mut sessions = Vec::new();
        for (_id, path) in session_files {
            match self.parse_session(&path) {
                Ok(session) => sessions.push(session),
                Err(e) => {
                    debug!("Failed to parse session {}: {}", path.display(), e);
                }
            }
        }

        info!(
            "Parsed {} sessions from project {}",
            sessions.len(),
            project_path.display()
        );
        Ok(sessions)
    }

    /// Check incremental parsing state (new format).
    ///
    /// # Arguments
    /// * `session_file` - Path to the .jsonl session file
    /// * `last_indexed` - Optional last indexed timestamp
    ///
    /// # Returns
    /// * `bool` - true if session needs re-indexing
    pub fn needs_indexing(
        &self,
        session_file: &Path,
        last_indexed: Option<DateTime<Utc>>,
    ) -> Result<bool> {
        if !session_file.exists() {
            return Ok(true);
        }

        // Check file modification time (session is a .jsonl file, not a directory)
        let metadata = fs::metadata(session_file).map_err(RagError::Io)?;
        let modified = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| {
                DateTime::from_timestamp(d.as_secs() as i64, 0).unwrap_or_else(Utc::now)
            });

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
    use tempfile::TempDir;

    fn create_test_session_new_format(dir: &Path, session_id: &str, content: &str) -> PathBuf {
        let session_file = dir.join(format!("{}.jsonl", session_id));
        fs::write(&session_file, content).unwrap();

        // Create sessions-index.json
        let index_content = serde_json::json!({
            "entries": [{
                "sessionId": session_id,
                "summary": format!("Test Session {}", session_id),
                "projectPath": "/test/project",
                "created": "2024-01-01T00:00:00Z"
            }]
        });
        fs::write(dir.join("sessions-index.json"), index_content.to_string()).unwrap();

        session_file
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
    fn test_encode_project_path() {
        let path = Path::new("/home/changh/Projects/claude-rag");
        let encoded = SessionParser::encode_project_path(path);
        assert_eq!(encoded, "-home-changh-Projects-claude_rag");
    }

    #[test]
    fn test_encode_project_path_no_collision() {
        let path1 = Path::new("/a/b");
        let path2 = Path::new("/a-b");

        let encoded1 = SessionParser::encode_project_path(path1);
        let encoded2 = SessionParser::encode_project_path(path2);

        assert_ne!(encoded1, encoded2, "Path encoding should not collide");
        assert_eq!(encoded1, "-a-b", "Path /a/b should encode to -a-b");
        assert_eq!(encoded2, "-a_b", "Path /a-b should encode to -a_b");
    }

    #[test]
    fn test_decode_project_path() {
        let encoded = "-home-changh-Projects-claude_rag";
        let decoded = SessionParser::decode_project_path(encoded);
        assert_eq!(decoded, "/home/changh/Projects/claude-rag");
    }

    #[test]
    fn test_parse_jsonl_new_format() {
        let content = r#"{"type":"user","timestamp":"2024-01-01T00:00:00Z","uuid":"msg-1","sessionId":"sess-1","message":{"role":"user","content":"Hello","tokens":10}}
{"type":"assistant","timestamp":"2024-01-01T00:00:01Z","uuid":"msg-2","sessionId":"sess-1","message":{"role":"assistant","content":"Hi there","model":"claude-3","tokens":20}}"#;

        let parser = SessionParser::default();
        let messages = parser.parse_jsonl_new(content).unwrap();

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, Role::User);
        assert_eq!(messages[0].content, "Hello");
        assert_eq!(messages[0].id, "msg-1");
        assert_eq!(messages[1].role, Role::Assistant);
        assert_eq!(messages[1].content, "Hi there");
        assert_eq!(messages[1].model, Some("claude-3".to_string()));
    }

    #[test]
    fn test_scan_sessions_in_dir() {
        let temp = TempDir::new().unwrap();
        let project_dir = temp.path();

        // Create session files
        create_test_session_new_format(project_dir, "sess-1", "content1");
        create_test_session_new_format(project_dir, "sess-2", "content2");

        // Create non-jsonl file (should be ignored)
        fs::write(project_dir.join("README.md"), "readme").unwrap();

        let parser = SessionParser::default();
        let sessions = parser.scan_sessions_in_dir(project_dir).unwrap();

        assert_eq!(sessions.len(), 2);
        assert!(sessions.contains_key("sess-1"));
        assert!(sessions.contains_key("sess-2"));
    }

    #[test]
    fn test_parse_sessions_index() {
        let temp = TempDir::new().unwrap();
        let project_dir = temp.path();

        // Create sessions-index.json
        let index_content = serde_json::json!({
            "entries": [{
                "sessionId": "test-session",
                "summary": "Test Summary",
                "projectPath": "/test/project",
                "created": "2024-01-01T00:00:00Z"
            }]
        });
        fs::write(project_dir.join("sessions-index.json"), index_content.to_string()).unwrap();

        // Create session file
        let session_file = project_dir.join("test-session.jsonl");
        fs::write(&session_file, "dummy content").unwrap();

        let parser = SessionParser::default();
        let meta = parser.parse_sessions_index(&session_file).unwrap();

        assert_eq!(meta.id, "test-session");
        assert_eq!(meta.title, Some("Test Summary".to_string()));
        assert_eq!(meta.project_path, "/test/project");
    }

    #[test]
    fn test_parse_session() {
        let temp = TempDir::new().unwrap();
        let content = r#"{"type":"user","timestamp":"2024-01-01T00:00:00Z","uuid":"msg-1","sessionId":"sess-1","message":{"role":"user","content":"Hello","tokens":10}}
{"type":"assistant","timestamp":"2024-01-01T00:00:01Z","uuid":"msg-2","sessionId":"sess-1","message":{"role":"assistant","content":"Hi there","model":"claude-3","tokens":20}}"#;

        let session_file = create_test_session_new_format(temp.path(), "sess-1", content);

        let parser = SessionParser::default();
        let parsed = parser.parse_session(&session_file).unwrap();

        assert_eq!(parsed.session.message_count, 2);
        assert_eq!(parsed.messages.len(), 2);
        assert_eq!(parsed.messages[0].role, Role::User);
        assert_eq!(parsed.messages[0].content, "Hello");
        assert_eq!(parsed.messages[1].role, Role::Assistant);
        assert_eq!(parsed.messages[1].content, "Hi there");
    }

    #[test]
    fn test_needs_indexing() {
        let temp = TempDir::new().unwrap();
        let content = r#"{"type":"user","timestamp":"2024-01-01T00:00:00Z","uuid":"msg-1","sessionId":"sess-1","message":{"role":"user","content":"Hello","tokens":10}}"#;

        let session_file = create_test_session_new_format(temp.path(), "sess-1", content);

        let parser = SessionParser::default();

        // Never indexed -> needs indexing
        assert!(parser.needs_indexing(&session_file, None).unwrap());

        // Indexed in the past -> needs indexing
        let past = Utc::now() - chrono::Duration::days(1);
        assert!(parser.needs_indexing(&session_file, Some(past)).unwrap());
    }

    #[test]
    fn test_parse_message_with_empty_uuid() {
        let content = r#"{"type":"user","timestamp":"2024-01-01T00:00:00Z","uuid":"","sessionId":"sess-1","message":{"role":"user","content":"Hello","tokens":10}}"#;
        let parser = SessionParser::default();
        let messages = parser.parse_jsonl_new(content).unwrap();
        assert_eq!(messages.len(), 0, "Empty UUID should be skipped");
    }

    #[test]
    fn test_parse_message_with_empty_session_id() {
        let content = r#"{"type":"user","timestamp":"2024-01-01T00:00:00Z","uuid":"msg-1","sessionId":"","message":{"role":"user","content":"Hello","tokens":10}}"#;
        let parser = SessionParser::default();
        let messages = parser.parse_jsonl_new(content).unwrap();
        assert_eq!(messages.len(), 0, "Empty session_id should be skipped");
    }
}
