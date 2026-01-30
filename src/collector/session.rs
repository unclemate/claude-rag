//! Session collection from Claude Code history.

use crate::error::{RagError, Result};
use crate::parser::{ParsedSession, SessionParser};
use crate::progress::ProgressReporter;
use crate::storage::sled::StorageManager;
use chrono::{DateTime, Utc};
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

/// Collection statistics for sessions.
#[derive(Debug, Clone, Default)]
pub struct SessionCollectionStats {
    /// Number of sessions scanned
    pub sessions_scanned: usize,
    /// Number of sessions collected
    pub sessions_collected: usize,
    /// Number of messages stored to database
    pub messages_stored: usize,
    /// Number of message chunks indexed to vector store
    pub message_chunks_indexed: usize,
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
        debug!(
            "Creating SessionCollector with config_dir: {:?}",
            config_dir
        );
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
        debug!("Scanning for Claude projects with sessions");
        let projects = self.parser.scan_claude_projects()?;

        let mut project_paths: Vec<String> = projects.keys().cloned().collect();
        project_paths.sort();

        info!("Found {} projects with sessions", project_paths.len());
        Ok(project_paths)
    }

    /// Collect sessions from all projects or a specific project.
    ///
    /// # Returns
    /// * `Vec<ParsedSession>` - List of parsed sessions
    pub fn collect_sessions(&self) -> Result<Vec<ParsedSession>> {
        if let Some(project_path) = &self.project_path {
            // Collect from specific project
            debug!("Collecting sessions from specific project: {}", project_path.display());
            let sessions = self.parser.parse_project_sessions(project_path)?;
            Ok(sessions)
        } else {
            // Collect from all projects
            debug!("Collecting sessions from all projects");
            let projects = self.parser.scan_claude_projects()?;
            let mut all_sessions = Vec::new();

            for session_dir in projects.values() {
                // Each session_dir is actually a session directory
                match self.parser.parse_session(session_dir) {
                    Ok(session) => all_sessions.push(session),
                    Err(e) => {
                        warn!(
                            session_dir = %session_dir.display(),
                            error = %e,
                            "Failed to parse session"
                        );
                    }
                }
            }

            info!("Collected {} sessions from all projects", all_sessions.len());
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
        debug!("Collecting sessions incrementally");
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
                    debug!(
                        "Session {} message count changed: {} -> {}",
                        session_id, existing.message_count, current_msg_count
                    );
                    needs_indexing.push(parsed);
                    continue;
                }

                // Check modification time
                let last_indexed = storage.get_index_state(session_id)?;
                let indexed_time = last_indexed
                    .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
                    .map(|dt| dt.with_timezone(&Utc));

                // Get session file path
                let session_file = self.get_session_file(session_id)?;
                if self.parser.needs_indexing(&session_file, indexed_time)? {
                    debug!("Session {} needs indexing (modified)", session_id);
                    needs_indexing.push(parsed);
                }
            } else {
                // New session
                debug!("Session {} is new, needs indexing", session_id);
                needs_indexing.push(parsed);
            }
        }

        info!(
            "Incremental collection: {} sessions need indexing",
            needs_indexing.len()
        );
        Ok(needs_indexing)
    }

    /// Get the session file path for a session ID (new format).
    fn get_session_file(&self, session_id: &str) -> Result<PathBuf> {
        let projects = self.parser.scan_claude_projects()?;

        // Find the session file
        if let Some(path) = projects.get(session_id) {
            return Ok(path.clone());
        }

        Err(RagError::NotFound(format!("Session file for {}", session_id)))
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
                    warn!(
                        session_id = %parsed.session.id,
                        error = %e,
                        "Error storing session"
                    );
                    stats.errors += 1;
                    continue;
                }
            }

            // Store messages
            for message in &parsed.messages {
                match storage.store_message(message) {
                    Ok(_) => {
                        stats.messages_stored += 1;
                    }
                    Err(e) => {
                        warn!(
                            message_id = %message.id,
                            error = %e,
                            "Error storing message"
                        );
                        stats.errors += 1;
                    }
                }
            }

            // Mark as indexed
            let _ = storage.set_index_state(&parsed.session.id, &Utc::now().to_rfc3339());
        }

        Ok(stats)
    }

    /// Store collected sessions to storage with progress reporting.
    ///
    /// # Arguments
    /// * `sessions` - Sessions to store
    /// * `storage` - Storage manager
    /// * `reporter` - Progress reporter
    ///
    /// # Returns
    /// * `SessionCollectionStats` - Collection statistics
    pub fn store_sessions_with_progress(
        &self,
        sessions: &[ParsedSession],
        storage: &StorageManager,
        reporter: &dyn ProgressReporter,
    ) -> Result<SessionCollectionStats> {
        use crate::progress::ProgressEvent;

        let mut stats = SessionCollectionStats {
            sessions_scanned: sessions.len(),
            ..Default::default()
        };

        // Report phase started
        reporter.report(ProgressEvent::PhaseStarted {
            name: "sessions".to_string(),
            total: sessions.len(),
        });

        for (i, parsed) in sessions.iter().enumerate() {
            // Report progress
            reporter.report(ProgressEvent::ItemProgress {
                current: i + 1,
                total: sessions.len(),
                name: parsed.session.title.clone().unwrap_or_else(|| parsed.session.id.clone()),
            });

            // Store session
            match storage.store_session(&parsed.session) {
                Ok(_) => {
                    stats.sessions_collected += 1;
                    stats.messages_stored += parsed.messages.len();
                    reporter.report(ProgressEvent::ItemCompleted {
                        name: parsed.session.id.clone(),
                        success: true,
                    });
                }
                Err(e) => {
                    stats.errors += 1;
                    warn!(
                        session_id = %parsed.session.id,
                        error = %e,
                        "Error storing session"
                    );
                    reporter.report(ProgressEvent::ItemCompleted {
                        name: parsed.session.id.clone(),
                        success: false,
                    });
                    reporter.report(ProgressEvent::Error {
                        message: format!("Failed to store session {}: {}", parsed.session.id, e),
                    });
                    continue;
                }
            }

            // Store messages
            for message in &parsed.messages {
                match storage.store_message(message) {
                    Ok(_) => {
                        // Message storage success
                    }
                    Err(e) => {
                        stats.errors += 1;
                        warn!(
                            message_id = %message.id,
                            error = %e,
                            "Error storing message"
                        );
                        reporter.report(ProgressEvent::Error {
                            message: format!("Failed to store message {}: {}", message.id, e),
                        });
                    }
                }
            }

            // Mark as indexed
            let _ = storage.set_index_state(&parsed.session.id, &Utc::now().to_rfc3339());
        }

        // Report phase completed
        reporter.report(ProgressEvent::PhaseCompleted {
            name: "sessions".to_string(),
            duration_secs: 0.0,
        });

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
        let session_file = match self.get_session_file(session_id) {
            Ok(file) => file,
            Err(RagError::NotFound(_)) => return Ok(None),
            Err(e) => return Err(e),
        };
        match self.parser.parse_session(&session_file) {
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
    use std::fs;
    use tempfile::TempDir;

    /// Create a test session file in new format
    fn create_test_session_new_format(dir: &Path, session_id: &str, content: &str, title: &str) {
        let session_file = dir.join(format!("{}.jsonl", session_id));
        fs::write(&session_file, content).unwrap();

        // Create or update sessions-index.json
        let index_path = dir.join("sessions-index.json");
        let index_content = if index_path.exists() {
            let existing = fs::read_to_string(&index_path).unwrap();
            let mut json: serde_json::Value = serde_json::from_str(&existing).unwrap();
            if let Some(entries) = json["entries"].as_array_mut() {
                entries.push(serde_json::json!({
                    "sessionId": session_id,
                    "summary": title,
                    "projectPath": "/test/project",
                    "created": "2024-01-01T00:00:00Z"
                }));
            }
            serde_json::to_string_pretty(&json).unwrap()
        } else {
            serde_json::json!({
                "entries": [{
                    "sessionId": session_id,
                    "summary": title,
                    "projectPath": "/test/project",
                    "created": "2024-01-01T00:00:00Z"
                }]
            }).to_string()
        };
        fs::write(&index_path, index_content).unwrap();
    }

    /// Create a mock Claude projects directory structure (new format)
    fn create_mock_projects(temp: &Path) {
        let projects_dir = temp.join("projects");
        let project_dir = projects_dir.join("test-project");
        fs::create_dir_all(&project_dir).unwrap();

        // Session 1
        let content1 = r#"{"type":"user","timestamp":"2024-01-01T00:00:00Z","uuid":"msg-1","sessionId":"session-1","message":{"role":"user","content":"Hello","tokens":10}}
{"type":"assistant","timestamp":"2024-01-01T00:00:01Z","uuid":"msg-2","sessionId":"session-1","message":{"role":"assistant","content":"Hi there","model":"claude-3","tokens":20}}"#;
        create_test_session_new_format(&project_dir, "session-1", content1, "Test Session 1");

        // Session 2
        let content2 = r#"{"type":"user","timestamp":"2024-01-02T00:00:00Z","uuid":"msg-3","sessionId":"session-2","message":{"role":"user","content":"How do I","tokens":10}}
{"type":"assistant","timestamp":"2024-01-02T00:00:01Z","uuid":"msg-4","sessionId":"session-2","message":{"role":"assistant","content":"To do that","model":"claude-3","tokens":20}}
{"type":"user","timestamp":"2024-01-02T00:00:02Z","uuid":"msg-5","sessionId":"session-2","message":{"role":"user","content":"Thanks","tokens":5}}"#;
        create_test_session_new_format(&project_dir, "session-2", content2, "Test Session 2");
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
        create_mock_projects(temp.path());

        let collector = SessionCollector::new(Some(temp.path().to_path_buf()));
        let projects = collector.scan_projects().unwrap();

        // scan_projects returns session IDs in new format
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
        create_mock_projects(temp.path());

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
        create_mock_projects(temp.path());

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
        create_mock_projects(temp.path());

        let collector = SessionCollector::new(Some(temp.path().to_path_buf()));
        let storage = StorageManager::open_project_db(temp.path()).unwrap();

        let sessions = collector.collect_sessions().unwrap();
        let stats = collector.store_sessions(&sessions, &storage).unwrap();

        assert_eq!(stats.sessions_scanned, 2);
        assert_eq!(stats.sessions_collected, 2);
        assert_eq!(stats.messages_stored, 5); // 2 + 3 messages
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
        create_mock_projects(temp.path());

        let collector = SessionCollector::new(Some(temp.path().to_path_buf()));
        let storage = StorageManager::open_project_db(temp.path()).unwrap();

        // All sessions should be collected (new)
        let sessions = collector.collect_incremental(&storage).unwrap();

        assert_eq!(sessions.len(), 2);
    }

    #[test]
    fn test_collect_incremental_no_changes() {
        let temp = TempDir::new().unwrap();
        create_mock_projects(temp.path());

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
        create_mock_projects(temp.path());

        let collector = SessionCollector::new(Some(temp.path().to_path_buf()));

        let session = collector.parse_session_by_id("session-1").unwrap();

        assert!(session.is_some());
        assert_eq!(session.unwrap().session.title, Some("Test Session 1".to_string()));
    }

    #[test]
    fn test_parse_session_by_id_not_found() {
        let temp = TempDir::new().unwrap();
        create_mock_projects(temp.path());

        let collector = SessionCollector::new(Some(temp.path().to_path_buf()));

        let session = collector.parse_session_by_id("nonexistent").unwrap();

        assert!(session.is_none());
    }

    #[test]
    fn test_collect_from_specific_project() {
        let temp = TempDir::new().unwrap();

        // Create a project with sessions (new format)
        let projects_dir = temp.path().join("projects");
        let project_path = temp.path().join("test-project");
        let encoded_path = crate::parser::SessionParser::encode_project_path(&project_path);
        let project_dir = projects_dir.join(&encoded_path);
        fs::create_dir_all(&project_dir).unwrap();

        let content = r#"{"type":"user","timestamp":"2024-01-01T00:00:00Z","uuid":"msg-1","sessionId":"proj-session-1","message":{"role":"user","content":"Project message","tokens":10}}"#;
        create_test_session_new_format(&project_dir, "proj-session-1", content, "Project Session");

        // Collect from specific project using temp as config_dir
        let collector = SessionCollector::new(Some(temp.path().to_path_buf())).with_project(&project_path);
        let sessions = collector.collect_sessions().unwrap();

        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].session.title, Some("Project Session".to_string()));
    }
}
