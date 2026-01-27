//! Sled database wrapper for KV storage.

use sled::Db;
use std::path::Path;

use crate::error::{Result, RagError};
use crate::models::{Commit, File, GitDiff, Message, Session, Symbol};

/// Storage manager for project data.
pub struct StorageManager {
    /// Sled database instance.
    db: Db,
}

impl StorageManager {
    /// Open or create a project database.
    pub fn open_project_db(project_path: &Path) -> Result<Self> {
        let db_dir = crate::config::ConfigManager::db_dir(project_path);

        // Create directory if it doesn't exist
        std::fs::create_dir_all(&db_dir)
            .map_err(|e| RagError::Io(e))?;

        let db = sled::open(&db_dir)
            .map_err(|e| RagError::Sled(e))?;

        Ok(Self { db })
    }

    /// Get the database instance.
    pub fn db(&self) -> &Db {
        &self.db
    }

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

    /// Store a message.
    pub fn store_message(&self, message: &Message) -> Result<()> {
        let key = format!("message:{}", message.id);
        let value = serde_json::to_vec(message)?;
        self.db.insert(key, value)?;
        Ok(())
    }

    /// Store a file.
    pub fn store_file(&self, file: &File) -> Result<()> {
        let key = format!("file:{}", file.id);
        let value = serde_json::to_vec(file)?;
        self.db.insert(key, value)?;
        Ok(())
    }

    /// Store a symbol.
    pub fn store_symbol(&self, symbol: &Symbol) -> Result<()> {
        let key = format!("symbol:{}", symbol.id);
        let value = serde_json::to_vec(symbol)?;
        self.db.insert(key, value)?;
        Ok(())
    }

    /// Store a commit.
    pub fn store_commit(&self, commit: &Commit) -> Result<()> {
        let key = format!("commit:{}", commit.id);
        let value = serde_json::to_vec(commit)?;
        self.db.insert(key, value)?;
        Ok(())
    }

    /// Store a git diff.
    pub fn store_diff(&self, diff: &GitDiff) -> Result<()> {
        let key = format!("diff:{}", diff.id);
        let value = serde_json::to_vec(diff)?;
        self.db.insert(key, value)?;
        Ok(())
    }

    /// Get or create index state for an item.
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

    /// Flush database to disk.
    pub fn flush(&self) -> Result<()> {
        self.db.flush()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

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
            started_at: chrono::Utc::now(),
            ended_at: None,
            message_count: 0,
            indexed: false,
        };

        storage.store_session(&session).unwrap();
        let retrieved = storage.get_session("test-session").unwrap();

        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().id, "test-session");
    }
}
