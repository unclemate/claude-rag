//! Sled database wrapper for KV storage.

use sled::Db;
use std::path::{Path, PathBuf};
use tracing::info;

use crate::error::{Result, RagError};
use crate::models::{Commit, File, GitDiff, Message, Session, Symbol};
use crate::storage::HnswIndex;

/// Storage manager for project data.
pub struct StorageManager {
    /// Sled database instance.
    db: Db,
    /// Project path for HNSW index storage.
    project_path: PathBuf,
}

impl StorageManager {
    /// Open or create a project database.
    pub fn open_project_db(project_path: &Path) -> Result<Self> {
        let db_dir = crate::config::ConfigManager::db_dir(project_path);

        // Create directory if it doesn't exist
        std::fs::create_dir_all(&db_dir)
            .map_err(RagError::Io)?;

        info!("Opening project database at {}", db_dir.display());
        let db = sled::open(&db_dir)
            .map_err(RagError::Sled)?;

        info!("Project database opened successfully");
        Ok(Self {
            db,
            project_path: project_path.to_path_buf(),
        })
    }

    /// Get the database instance.
    pub fn db(&self) -> &Db {
        &self.db
    }

    // ==================== Primary Key Operations ====================

    /// Store a session.
    pub fn store_session(&self, session: &Session) -> Result<()> {
        let key = format!("session:{}", session.id);
        let value = serde_json::to_vec(session)?;
        self.db.insert(key, value)?;
        Ok(())
    }

    /// Retrieve a session.
    pub fn get_session(&self, id: &str) -> Result<Option<Session>> {
        let key = format!("session:{}", id);
        if let Some(value) = self.db.get(key)? {
            let session = serde_json::from_slice(&value)?;
            Ok(Some(session))
        } else {
            Ok(None)
        }
    }

    /// Get all sessions from storage.
    pub fn get_all_sessions(&self) -> Result<Vec<Session>> {
        let mut sessions = Vec::new();
        let prefix = "session:";

        // Use sled's prefix iteration
        for item in self.db.scan_prefix(prefix) {
            let (_key, value) = item.map_err(RagError::Sled)?;
            if let Ok(session) = serde_json::from_slice::<Session>(&value) {
                sessions.push(session);
            }
        }

        Ok(sessions)
    }

    /// Iterate over all sessions with a callback function.
    ///
    /// This is memory-efficient for large datasets as it doesn't load all items at once.
    pub fn iter_sessions<F>(&self, mut callback: F) -> Result<()>
    where
        F: FnMut(Session) -> Result<()>,
    {
        let prefix = "session:";

        for item in self.db.scan_prefix(prefix) {
            let (_key, value) = item.map_err(RagError::Sled)?;
            if let Ok(session) = serde_json::from_slice::<Session>(&value) {
                callback(session)?;
            }
        }

        Ok(())
    }

    /// Store a message.
    pub fn store_message(&self, message: &Message) -> Result<()> {
        let key = format!("message:{}", message.id);
        let value = serde_json::to_vec(message)?;
        self.db.insert(key, value)?;

        // Update session_messages index
        self.add_to_session_messages(&message.session_id, &message.id)?;

        Ok(())
    }

    /// Retrieve a message.
    pub fn get_message(&self, id: &str) -> Result<Option<Message>> {
        let key = format!("message:{}", id);
        if let Some(value) = self.db.get(key)? {
            let message = serde_json::from_slice(&value)?;
            Ok(Some(message))
        } else {
            Ok(None)
        }
    }

    /// Store a file.
    pub fn store_file(&self, file: &File) -> Result<()> {
        let key = format!("file:{}", file.id);
        let value = serde_json::to_vec(file)?;
        self.db.insert(key, value)?;

        // Update project_files index
        self.add_to_project_files(&file.project_path, &file.id)?;

        Ok(())
    }

    /// Retrieve a file.
    pub fn get_file(&self, id: &str) -> Result<Option<File>> {
        let key = format!("file:{}", id);
        if let Some(value) = self.db.get(key)? {
            let file = serde_json::from_slice(&value)?;
            Ok(Some(file))
        } else {
            Ok(None)
        }
    }

    /// Store a symbol.
    pub fn store_symbol(&self, symbol: &Symbol) -> Result<()> {
        let key = format!("symbol:{}", symbol.id);
        let value = serde_json::to_vec(symbol)?;
        self.db.insert(key, value)?;

        // Update file_symbols index
        self.add_to_file_symbols(&symbol.file_id, &symbol.id)?;

        Ok(())
    }

    /// Retrieve a symbol.
    pub fn get_symbol(&self, id: &str) -> Result<Option<Symbol>> {
        let key = format!("symbol:{}", id);
        if let Some(value) = self.db.get(key)? {
            let symbol = serde_json::from_slice(&value)?;
            Ok(Some(symbol))
        } else {
            Ok(None)
        }
    }

    /// Store a commit.
    pub fn store_commit(&self, commit: &Commit) -> Result<()> {
        let key = format!("commit:{}", commit.id);
        let value = serde_json::to_vec(commit)?;
        self.db.insert(key, value)?;
        Ok(())
    }

    /// Retrieve a commit.
    pub fn get_commit(&self, id: &str) -> Result<Option<Commit>> {
        let key = format!("commit:{}", id);
        if let Some(value) = self.db.get(key)? {
            let commit = serde_json::from_slice(&value)?;
            Ok(Some(commit))
        } else {
            Ok(None)
        }
    }

    /// Get all commits from storage.
    pub fn get_all_commits(&self) -> Result<Vec<Commit>> {
        let mut commits = Vec::new();
        let prefix = "commit:";

        // Use sled's prefix iteration
        for item in self.db.scan_prefix(prefix) {
            let (_key, value) = item.map_err(RagError::Sled)?;
            if let Ok(commit) = serde_json::from_slice::<Commit>(&value) {
                commits.push(commit);
            }
        }

        Ok(commits)
    }

    /// Iterate over all commits with a callback function.
    ///
    /// This is memory-efficient for large datasets as it doesn't load all items at once.
    pub fn iter_commits<F>(&self, mut callback: F) -> Result<()>
    where
        F: FnMut(Commit) -> Result<()>,
    {
        let prefix = "commit:";

        for item in self.db.scan_prefix(prefix) {
            let (_key, value) = item.map_err(RagError::Sled)?;
            if let Ok(commit) = serde_json::from_slice::<Commit>(&value) {
                callback(commit)?;
            }
        }

        Ok(())
    }

    /// Store a git diff.
    pub fn store_diff(&self, diff: &GitDiff) -> Result<()> {
        let key = format!("diff:{}", diff.id);
        let value = serde_json::to_vec(diff)?;
        self.db.insert(key, value)?;

        // Update file_commits index
        self.add_to_file_commits(&diff.project_path, &diff.file_path, &diff.commit_id)?;

        Ok(())
    }

    /// Retrieve a git diff.
    pub fn get_diff(&self, id: &str) -> Result<Option<GitDiff>> {
        let key = format!("diff:{}", id);
        if let Some(value) = self.db.get(key)? {
            let diff = serde_json::from_slice(&value)?;
            Ok(Some(diff))
        } else {
            Ok(None)
        }
    }

    // ==================== Auxiliary Index Operations ====================

    /// Add message ID to session_messages index.
    fn add_to_session_messages(&self, session_id: &str, message_id: &str) -> Result<()> {
        let key = format!("session_messages:{}", session_id);
        let mut ids = self.get_id_list(&key)?;
        if !ids.contains(&message_id.to_string()) {
            ids.push(message_id.to_string());
            self.set_id_list(&key, &ids)?;
        }
        Ok(())
    }

    /// Get all message IDs for a session.
    pub fn get_session_messages(&self, session_id: &str) -> Result<Vec<String>> {
        let key = format!("session_messages:{}", session_id);
        self.get_id_list(&key)
    }

    /// Add file ID to project_files index.
    fn add_to_project_files(&self, project_path: &str, file_id: &str) -> Result<()> {
        let key = format!("project_files:{}", project_path);
        let mut ids = self.get_id_list(&key)?;
        if !ids.contains(&file_id.to_string()) {
            ids.push(file_id.to_string());
            self.set_id_list(&key, &ids)?;
        }
        Ok(())
    }

    /// Get all file IDs for a project.
    pub fn get_project_files(&self, project_path: &str) -> Result<Vec<String>> {
        let key = format!("project_files:{}", project_path);
        self.get_id_list(&key)
    }

    /// Add symbol ID to file_symbols index.
    fn add_to_file_symbols(&self, file_id: &str, symbol_id: &str) -> Result<()> {
        let key = format!("file_symbols:{}", file_id);
        let mut ids = self.get_id_list(&key)?;
        if !ids.contains(&symbol_id.to_string()) {
            ids.push(symbol_id.to_string());
            self.set_id_list(&key, &ids)?;
        }
        Ok(())
    }

    /// Get all symbol IDs for a file.
    pub fn get_file_symbols(&self, file_id: &str) -> Result<Vec<String>> {
        let key = format!("file_symbols:{}", file_id);
        self.get_id_list(&key)
    }

    /// Add commit ID to file_commits index.
    fn add_to_file_commits(&self, project_path: &str, file_path: &str, commit_id: &str) -> Result<()> {
        let key = format!("file_commits:{}:{}", project_path, file_path);
        let mut ids = self.get_id_list(&key)?;
        if !ids.contains(&commit_id.to_string()) {
            ids.push(commit_id.to_string());
            self.set_id_list(&key, &ids)?;
        }
        Ok(())
    }

    /// Get all commit IDs for a file.
    pub fn get_file_commits(&self, project_path: &str, file_path: &str) -> Result<Vec<String>> {
        let key = format!("file_commits:{}:{}", project_path, file_path);
        self.get_id_list(&key)
    }

    /// Get ID list from an index key.
    fn get_id_list(&self, key: &str) -> Result<Vec<String>> {
        if let Some(value) = self.db.get(key)? {
            let ids: Vec<String> = serde_json::from_slice(&value)?;
            Ok(ids)
        } else {
            Ok(Vec::new())
        }
    }

    /// Set ID list for an index key.
    fn set_id_list(&self, key: &str, ids: &[String]) -> Result<()> {
        let value = serde_json::to_vec(ids)?;
        self.db.insert(key, value)?;
        Ok(())
    }

    // ==================== Index State Operations ====================

    /// Get index state for an item.
    pub fn get_index_state(&self, id: &str) -> Result<Option<String>> {
        let key = format!("index_state:{}", id);
        if let Some(value) = self.db.get(key)? {
            Ok(Some(String::from_utf8_lossy(&value).to_string()))
        } else {
            Ok(None)
        }
    }

    /// Set index state for an item.
    pub fn set_index_state(&self, id: &str, state: &str) -> Result<()> {
        let key = format!("index_state:{}", id);
        self.db.insert(key, state)?;
        Ok(())
    }

    // ==================== Database Operations ====================

    /// Flush database to disk.
    pub fn flush(&self) -> Result<()> {
        self.db.flush()?;
        Ok(())
    }

    // ==================== HNSW Index Operations ====================

    /// Get the HNSW index file path for this project.
    pub fn hnsw_path(&self) -> PathBuf {
        crate::config::ConfigManager::rag_dir(&self.project_path).join("hnsw.bin")
    }

    /// Save HNSW index to disk.
    pub fn save_hnsw(&self, index: &HnswIndex) -> Result<()> {
        let path = self.hnsw_path();
        index.save(&path)
    }

    /// Load HNSW index from disk.
    ///
    /// Returns `Ok(None)` if the index file doesn't exist.
    pub fn load_hnsw(&self) -> Result<Option<HnswIndex>> {
        let path = self.hnsw_path();
        if !path.exists() {
            return Ok(None);
        }
        HnswIndex::load(&path).map(Some)
    }

    /// Check if HNSW index exists on disk.
    pub fn has_hnsw_index(&self) -> bool {
        self.hnsw_path().exists()
    }

    /// Get database size in bytes.
    pub fn size_on_disk(&self) -> Result<u64> {
        self.db.size_on_disk().map_err(RagError::Sled)
    }

    /// Check if database is empty.
    pub fn is_empty(&self) -> bool {
        self.db.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use chrono::Utc;
    use crate::models::diff::ChangeType;

    #[test]
    fn test_storage_manager_open() {
        let temp_dir = TempDir::new().unwrap();
        let project_path = temp_dir.path();

        let storage = StorageManager::open_project_db(project_path);
        assert!(storage.is_ok());
    }

    #[test]
    fn test_session_store_retrieve() {
        let temp_dir = TempDir::new().unwrap();
        let project_path = temp_dir.path();
        let storage = StorageManager::open_project_db(project_path).unwrap();

        let session = Session {
            id: "test-session".to_string(),
            title: Some("Test".to_string()),
            project_path: project_path.display().to_string(),
            started_at: Utc::now(),
            ended_at: None,
            message_count: 0,
            indexed: false,
        };

        storage.store_session(&session).unwrap();
        let retrieved = storage.get_session("test-session").unwrap();

        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().id, "test-session");
    }

    #[test]
    fn test_message_with_session_index() {
        let temp_dir = TempDir::new().unwrap();
        let project_path = temp_dir.path();
        let storage = StorageManager::open_project_db(project_path).unwrap();

        let message = Message {
            id: "msg-1".to_string(),
            session_id: "session-1".to_string(),
            role: crate::models::Role::User,
            content: "Hello".to_string(),
            timestamp: Utc::now(),
            tokens: None,
            model: None,
        };

        storage.store_message(&message).unwrap();
        let retrieved = storage.get_message("msg-1").unwrap();

        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().id, "msg-1");

        // Check session_messages index
        let msg_ids = storage.get_session_messages("session-1").unwrap();
        assert_eq!(msg_ids, vec!["msg-1".to_string()]);
    }

    #[test]
    fn test_file_with_project_index() {
        let temp_dir = TempDir::new().unwrap();
        let project_path = temp_dir.path();
        let storage = StorageManager::open_project_db(project_path).unwrap();

        let file = File {
            id: "file-1".to_string(),
            project_path: project_path.display().to_string(),
            file_path: "src/main.rs".to_string(),
            language: Some("rust".to_string()),
            kind: crate::models::file::FileKind::Source,
            modified_at: Utc::now(),
            size: 1024,
            content_hash: "abc123".to_string(),
            indexed: false,
        };

        storage.store_file(&file).unwrap();
        let retrieved = storage.get_file("file-1").unwrap();

        assert!(retrieved.is_some());

        // Check project_files index
        let file_ids = storage.get_project_files(&project_path.display().to_string()).unwrap();
        assert_eq!(file_ids, vec!["file-1".to_string()]);
    }

    #[test]
    fn test_symbol_with_file_index() {
        let temp_dir = TempDir::new().unwrap();
        let project_path = temp_dir.path();
        let storage = StorageManager::open_project_db(project_path).unwrap();

        let symbol = Symbol {
            id: "sym-1".to_string(),
            file_id: "file-1".to_string(),
            name: "test_func".to_string(),
            kind: crate::models::SymbolKind::Function,
            start_line: 10,
            end_line: 20,
            doc_comment: None,
            code: "fn test_func() {}".to_string(),
            parent_id: None,
        };

        storage.store_symbol(&symbol).unwrap();

        // Check file_symbols index
        let symbol_ids = storage.get_file_symbols("file-1").unwrap();
        assert_eq!(symbol_ids, vec!["sym-1".to_string()]);
    }

    #[test]
    fn test_index_state() {
        let temp_dir = TempDir::new().unwrap();
        let project_path = temp_dir.path();
        let storage = StorageManager::open_project_db(project_path).unwrap();

        // Initially no state
        assert!(storage.get_index_state("test-id").unwrap().is_none());

        // Set state
        storage.set_index_state("test-id", "hashed").unwrap();
        assert_eq!(storage.get_index_state("test-id").unwrap(), Some("hashed".to_string()));
    }

    #[test]
    fn test_file_commits_index() {
        let temp_dir = TempDir::new().unwrap();
        let project_path = temp_dir.path();
        let storage = StorageManager::open_project_db(project_path).unwrap();

        let diff = GitDiff {
            id: "diff-1".to_string(),
            commit_id: "commit-1".to_string(),
            project_path: project_path.display().to_string(),
            file_path: "src/main.rs".to_string(),
            old_oid: None,
            new_oid: None,
            change_type: ChangeType::Modified,
            diff_content: "".to_string(),
            diff_summary: "".to_string(),
            added_lines: 0,
            removed_lines: 0,
            insertions: 0,
            deletions: 0,
            timestamp: Utc::now(),
        };

        storage.store_diff(&diff).unwrap();

        // Check file_commits index
        let commit_ids = storage.get_file_commits(&project_path.display().to_string(), "src/main.rs").unwrap();
        assert_eq!(commit_ids, vec!["commit-1".to_string()]);
    }

    #[test]
    fn test_hnsw_path() {
        let temp_dir = TempDir::new().unwrap();
        let project_path = temp_dir.path();
        let storage = StorageManager::open_project_db(project_path).unwrap();

        let hnsw_path = storage.hnsw_path();
        assert!(hnsw_path.ends_with(".rag/hnsw.bin"));
        assert!(hnsw_path.starts_with(project_path));
    }

    #[test]
    fn test_hnsw_save_load() {
        use crate::models::ContentType;

        let temp_dir = TempDir::new().unwrap();
        let project_path = temp_dir.path();
        let storage = StorageManager::open_project_db(project_path).unwrap();

        // Create and save HNSW index
        let mut index = HnswIndex::new(16, 200, 50);
        index.insert(
            "test-id".to_string(),
            ContentType::Message,
            vec![0.1, 0.2, 0.3],
        ).unwrap();

        storage.save_hnsw(&index).unwrap();
        assert!(storage.has_hnsw_index());

        // Load HNSW index
        let loaded = storage.load_hnsw().unwrap();
        assert!(loaded.is_some());
        let loaded_index = loaded.unwrap();
        assert_eq!(loaded_index.len(), 1);
    }

    #[test]
    fn test_hnsw_load_not_exists() {
        let temp_dir = TempDir::new().unwrap();
        let project_path = temp_dir.path();
        let storage = StorageManager::open_project_db(project_path).unwrap();

        // HNSW index doesn't exist
        assert!(!storage.has_hnsw_index());
        let loaded = storage.load_hnsw().unwrap();
        assert!(loaded.is_none());
    }
}
