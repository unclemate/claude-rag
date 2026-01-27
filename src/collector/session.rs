//! Session collection from Claude Code history.

use crate::error::{RagError, Result};
use crate::parser::{ParsedSession, SessionParser};
use crate::storage::sled::StorageManager;
use chrono::{DateTime, Utc};
use std::path::{Path, PathBuf};

/// Collection statistics for sessions.
#[derive(Debug, Clone, Default)]
pub struct SessionCollectionStats {
    /// Number of sessions scanned
    pub sessions_scanned: usize,
    /// Number of sessions collected
    pub sessions_collected: usize,
    /// Number of messages collected
    pub messages_collected: usize,
    /// Number of errors
    pub errors: usize,
}

/// Session collector for gathering Claude Code session history.
pub struct SessionCollector {
    /// Session parser
    parser: SessionParser,
    /// Specific project path to collect from (None = all projects)
    project_path: Option<PathBuf>,
}

impl SessionCollector {
    /// Create a new session collector.
    ///
    /// # Arguments
    /// * `config_dir` - Optional custom Claude config directory
    pub fn new(config_dir: Option<PathBuf>) -> Self {
        let parser = SessionParser::new(config_dir);
        Self {
            parser,
            project_path: None,
        }
    }

    /// Set a specific project path to collect from.
    pub fn with_project(mut self, project_path: &Path) -> Self {
        self.project_path = Some(project_path.to_path_buf());
        self
    }

    /// Scan for Claude projects with sessions.
    ///
    /// # Returns
    /// * `Vec<String>` - List of project paths with sessions
    pub fn scan_projects(&self) -> Result<Vec<String>> {
        let projects = self.parser.scan_claude_projects()?;

        let mut project_paths: Vec<String> = projects.keys().cloned().collect();
        project_paths.sort();

        Ok(project_paths)
    }

    /// Collect sessions from all projects or a specific project.
    ///
    /// # Returns
    /// * `Vec<ParsedSession>` - List of parsed sessions
    pub fn collect_sessions(&self) -> Result<Vec<ParsedSession>> {
        if let Some(project_path) = &self.project_path {
            // Collect from specific project
            let sessions = self.parser.parse_project_sessions(project_path)?;
            Ok(sessions)
        } else {
            // Collect from all projects
            let projects = self.parser.scan_claude_projects()?;
            let mut all_sessions = Vec::new();

            for session_dir in projects.values() {
                // Each session_dir is actually a session directory
                match self.parser.parse_session(session_dir) {
                    Ok(session) => all_sessions.push(session),
                    Err(e) => {
                        eprintln!("Warning: Failed to parse session {:?}: {}", session_dir, e);
                    }
                }
            }

            Ok(all_sessions)
        }
    }

    /// Collect sessions incrementally based on storage state.
    ///
    /// # Arguments
    /// * `storage` - Storage manager to check index state
    ///
    /// # Returns
    /// * `Vec<ParsedSession>` - List of sessions that need indexing
    pub fn collect_incremental(&self, storage: &StorageManager) -> Result<Vec<ParsedSession>> {
        let all_sessions = self.collect_sessions()?;
        let mut needs_indexing = Vec::new();

        for parsed in all_sessions {
            // Check if session needs indexing
            let session_id = &parsed.session.id;

            // Check if session exists in storage
            if let Some(existing) = storage.get_session(session_id)? {
                // Check if messages have changed
                let current_msg_count = parsed.messages.len();
                if current_msg_count != existing.message_count {
                    needs_indexing.push(parsed);
                    continue;
                }

                // Check modification time
                let last_indexed = storage.get_index_state(session_id)?;
                let indexed_time = last_indexed
                    .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
                    .map(|dt| dt.with_timezone(&Utc));

                // Get session directory path
                let session_dir = self.get_session_dir(session_id)?;
                if self.parser.needs_indexing(&session_dir, indexed_time)? {
                    needs_indexing.push(parsed);
                }
            } else {
                // New session
                needs_indexing.push(parsed);
            }
        }

        Ok(needs_indexing)
    }

    /// Get the session directory for a session ID.
    fn get_session_dir(&self, session_id: &str) -> Result<PathBuf> {
        let projects = self.parser.scan_claude_projects()?;

        // Find the session directory
        for session_dir in projects.values() {
            if let Some(dir_name) = session_dir.file_name() {
                if dir_name.to_string_lossy() == session_id {
                    return Ok(session_dir.clone());
                }
            }
        }

        Err(RagError::NotFound(format!("Session directory for {}", session_id)))
    }

    /// Store collected sessions to storage.
    ///
    /// # Arguments
    /// * `sessions` - Sessions to store
    /// * `storage` - Storage manager
    ///
    /// # Returns
    /// * `SessionCollectionStats` - Collection statistics
    pub fn store_sessions(
        &self,
        sessions: &[ParsedSession],
        storage: &StorageManager,
    ) -> Result<SessionCollectionStats> {
        let mut stats = SessionCollectionStats {
            sessions_scanned: sessions.len(),
            ..Default::default()
        };

        for parsed in sessions {
            // Store session
            match storage.store_session(&parsed.session) {
                Ok(_) => {
                    stats.sessions_collected += 1;
                }
                Err(e) => {
                    eprintln!("Error storing session {}: {}", parsed.session.id, e);
                    stats.errors += 1;
                    continue;
                }
            }

            // Store messages
            for message in &parsed.messages {
                match storage.store_message(message) {
                    Ok(_) => {
                        stats.messages_collected += 1;
                    }
                    Err(e) => {
                        eprintln!("Error storing message {}: {}", message.id, e);
                        stats.errors += 1;
                    }
                }
            }

            // Mark as indexed
            let _ = storage.set_index_state(&parsed.session.id, &Utc::now().to_rfc3339());
        }

        Ok(stats)
    }

    /// Parse a single session by ID.
    ///
    /// # Arguments
    /// * `session_id` - Session identifier
    ///
    /// # Returns
    /// * `Option<ParsedSession>` - Parsed session if found
    pub fn parse_session_by_id(&self, session_id: &str) -> Result<Option<ParsedSession>> {
        let session_dir = match self.get_session_dir(session_id) {
            Ok(dir) => dir,
            Err(RagError::NotFound(_)) => return Ok(None),
            Err(e) => return Err(e),
        };
        match self.parser.parse_session(&session_dir) {
            Ok(session) => Ok(Some(session)),
            Err(RagError::NotFound(_)) => Ok(None),
            Err(e) => Err(e),
        }
    }
}

impl Default for SessionCollector {
    fn default() -> Self {
        Self::new(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{self, File as StdFile};
    use std::io::Write;
    use tempfile::TempDir;

    /// Create a test session directory
    fn create_test_session(dir: &Path, session_id: &str, content: &str) -> PathBuf {
        let session_dir = dir.join(session_id);
        fs::create_dir_all(&session_dir).unwrap();
        let index_path = session_dir.join("index.jsonl");
        let mut file = StdFile::create(&index_path).unwrap();
        file.write_all(content.as_bytes()).unwrap();
        session_dir
    }

    /// Create a mock Claude sessions directory structure
    fn create_mock_sessions(temp: &Path) {
        let sessions_dir = temp.join("sessions");
        fs::create_dir_all(&sessions_dir).unwrap();

        // Session 1
        let content1 = r#"{"title":"Test Session 1","projectPath":"/test/project1","createdAt":"2024-01-01T00:00:00Z"}
{"role":"user","content":"Hello","timestamp":"2024-01-01T00:00:00Z"}
{"role":"assistant","content":"Hi there","timestamp":"2024-01-01T00:00:01Z","model":"claude-3"}"#;
        create_test_session(&sessions_dir, "session-1", content1);

        // Session 2
        let content2 = r#"{"title":"Test Session 2","projectPath":"/test/project2","createdAt":"2024-01-02T00:00:00Z"}
{"role":"user","content":"How do I","timestamp":"2024-01-02T00:00:00Z"}
{"role":"assistant","content":"To do that","timestamp":"2024-01-02T00:00:01Z","model":"claude-3"}
{"role":"user","content":"Thanks","timestamp":"2024-01-02T00:00:02Z"}"#;
        create_test_session(&sessions_dir, "session-2", content2);
    }

    #[test]
    fn test_collector_new() {
        let collector = SessionCollector::new(None);
        assert!(collector.project_path.is_none());
    }

    #[test]
    fn test_collector_default() {
        let collector = SessionCollector::default();
        assert!(collector.project_path.is_none());
    }

    #[test]
    fn test_collector_with_project() {
        let temp = TempDir::new().unwrap();
        let collector = SessionCollector::new(None).with_project(temp.path());
        assert_eq!(collector.project_path, Some(temp.path().to_path_buf()));
    }

    #[test]
    fn test_scan_claude_projects() {
        let temp = TempDir::new().unwrap();
        create_mock_sessions(temp.path());

        let collector = SessionCollector::new(Some(temp.path().to_path_buf()));
        let projects = collector.scan_projects().unwrap();

        assert_eq!(projects.len(), 2);
    }

    #[test]
    fn test_scan_empty_directory() {
        let temp = TempDir::new().unwrap();

        let collector = SessionCollector::new(Some(temp.path().to_path_buf()));
        let projects = collector.scan_projects().unwrap();

        assert_eq!(projects.len(), 0);
    }

    #[test]
    fn test_collect_sessions() {
        let temp = TempDir::new().unwrap();
        create_mock_sessions(temp.path());

        let collector = SessionCollector::new(Some(temp.path().to_path_buf()));
        let sessions = collector.collect_sessions().unwrap();

        assert_eq!(sessions.len(), 2);

        // Find session with 2 messages
        let session1 = sessions.iter()
            .find(|s| s.messages.len() == 2)
            .expect("Should find session with 2 messages");
        assert_eq!(session1.session.title, Some("Test Session 1".to_string()));

        // Find session with 3 messages
        let session2 = sessions.iter()
            .find(|s| s.messages.len() == 3)
            .expect("Should find session with 3 messages");
        assert_eq!(session2.session.title, Some("Test Session 2".to_string()));
    }

    #[test]
    fn test_collect_sessions_message_content() {
        let temp = TempDir::new().unwrap();
        create_mock_sessions(temp.path());

        let collector = SessionCollector::new(Some(temp.path().to_path_buf()));
        let sessions = collector.collect_sessions().unwrap();

        // Find session with "Hello" message
        let hello_session = sessions.iter()
            .find(|s| s.messages.iter().any(|m| m.content == "Hello"))
            .expect("Should find session with Hello");

        assert_eq!(hello_session.messages[0].role, crate::models::Role::User);
        assert_eq!(hello_session.messages[0].content, "Hello");
        assert_eq!(hello_session.messages[1].role, crate::models::Role::Assistant);
        assert_eq!(hello_session.messages[1].content, "Hi there");
    }

    #[test]
    fn test_storage_integration() {
        let temp = TempDir::new().unwrap();
        create_mock_sessions(temp.path());

        let collector = SessionCollector::new(Some(temp.path().to_path_buf()));
        let storage = StorageManager::open_project_db(temp.path()).unwrap();

        let sessions = collector.collect_sessions().unwrap();
        let stats = collector.store_sessions(&sessions, &storage).unwrap();

        assert_eq!(stats.sessions_scanned, 2);
        assert_eq!(stats.sessions_collected, 2);
        assert_eq!(stats.messages_collected, 5); // 2 + 3 messages
        assert_eq!(stats.errors, 0);

        // Verify sessions were stored
        for session in &sessions {
            let retrieved = storage.get_session(&session.session.id).unwrap();
            assert!(retrieved.is_some());
            assert_eq!(retrieved.unwrap().id, session.session.id);
        }
    }

    #[test]
    fn test_collect_incremental_new_sessions() {
        let temp = TempDir::new().unwrap();
        create_mock_sessions(temp.path());

        let collector = SessionCollector::new(Some(temp.path().to_path_buf()));
        let storage = StorageManager::open_project_db(temp.path()).unwrap();

        // All sessions should be collected (new)
        let sessions = collector.collect_incremental(&storage).unwrap();

        assert_eq!(sessions.len(), 2);
    }

    #[test]
    fn test_collect_incremental_no_changes() {
        let temp = TempDir::new().unwrap();
        create_mock_sessions(temp.path());

        let collector = SessionCollector::new(Some(temp.path().to_path_buf()));
        let storage = StorageManager::open_project_db(temp.path()).unwrap();

        // First, collect and store
        let sessions = collector.collect_sessions().unwrap();
        collector.store_sessions(&sessions, &storage).unwrap();

        // Second collection should find no new sessions
        let sessions2 = collector.collect_incremental(&storage).unwrap();

        assert_eq!(sessions2.len(), 0);
    }

    #[test]
    fn test_parse_session_by_id() {
        let temp = TempDir::new().unwrap();
        create_mock_sessions(temp.path());

        let collector = SessionCollector::new(Some(temp.path().to_path_buf()));

        let session = collector.parse_session_by_id("session-1").unwrap();

        assert!(session.is_some());
        assert_eq!(session.unwrap().session.title, Some("Test Session 1".to_string()));
    }

    #[test]
    fn test_parse_session_by_id_not_found() {
        let temp = TempDir::new().unwrap();
        create_mock_sessions(temp.path());

        let collector = SessionCollector::new(Some(temp.path().to_path_buf()));

        let session = collector.parse_session_by_id("nonexistent").unwrap();

        assert!(session.is_none());
    }

    #[test]
    fn test_collect_from_specific_project() {
        let temp = TempDir::new().unwrap();

        // Create a project with sessions
        let project_dir = temp.path().join("test-project");
        let sessions_dir = project_dir.join(".claude/sessions");
        fs::create_dir_all(&sessions_dir).unwrap();

        let content = r#"{"title":"Project Session","projectPath":"/test-project","createdAt":"2024-01-01T00:00:00Z"}
{"role":"user","content":"Project message","timestamp":"2024-01-01T00:00:00Z"}"#;
        create_test_session(&sessions_dir, "proj-session-1", content);

        // Collect from specific project
        let collector = SessionCollector::new(None).with_project(&project_dir);
        let sessions = collector.collect_sessions().unwrap();

        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].session.title, Some("Project Session".to_string()));
    }
}
