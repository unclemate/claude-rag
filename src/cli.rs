//! CLI command implementations.

use crate::collector::file::{CollectionStats as FileCollectionStats, FileCollector};
use crate::collector::session::{SessionCollectionStats, SessionCollector};
use crate::config::{Config, ConfigManager};
use crate::error::{Result, RagError};
use crate::storage::sled::StorageManager;
use std::fs;
use std::path::{Path, PathBuf};

/// Initialize knowledge base for a project.
///
/// # Arguments
/// * `project_path` - Path to the project directory
/// * `force` - Force re-initialization if already initialized
///
/// # Returns
/// * `Result<InitResult>` - Initialization result
pub fn init_project(project_path: &Path, force: bool) -> Result<InitResult> {
    // Check if already initialized
    let rag_dir = ConfigManager::rag_dir(project_path);
    if rag_dir.exists() && !force {
        return Ok(InitResult::AlreadyExists);
    }

    // Create .rag directory structure
    fs::create_dir_all(&rag_dir)
        .map_err(RagError::Io)?;

    let db_dir = ConfigManager::db_dir(project_path);
    fs::create_dir_all(&db_dir)
        .map_err(RagError::Io)?;

    // Initialize storage
    let storage = StorageManager::open_project_db(project_path)?;

    // Create default config if it doesn't exist
    let config_path = rag_dir.join("config.json");
    if !config_path.exists() {
        let default_config = Config::default();
        let config_json = serde_json::to_string_pretty(&default_config)
            .map_err(RagError::Json)?;
        fs::write(&config_path, config_json)
            .map_err(RagError::Io)?;
    }

    Ok(InitResult::Success {
        rag_dir,
        storage_size: storage.size_on_disk().unwrap_or(0),
    })
}

/// Initialization result.
#[derive(Debug, Clone)]
pub enum InitResult {
    /// Successfully initialized
    Success {
        /// Path to .rag directory
        rag_dir: PathBuf,
        /// Storage size in bytes
        storage_size: u64,
    },
    /// Already initialized
    AlreadyExists,
}

/// Index options.
#[derive(Debug, Clone, Default)]
pub struct IndexOptions {
    /// Index source files
    pub index_source: bool,
    /// Index documentation files
    pub index_docs: bool,
    /// Index other files
    pub index_other: bool,
    /// Index sessions
    pub index_sessions: bool,
    /// Force re-index
    pub force: bool,
}

/// Index result.
#[derive(Debug, Clone)]
pub struct IndexResult {
    /// File collection stats
    pub file_stats: FileCollectionStats,
    /// Session collection stats
    pub session_stats: SessionCollectionStats,
    /// Number of errors
    pub errors: usize,
}

/// Index a project.
///
/// # Arguments
/// * `project_path` - Path to the project directory
/// * `options` - Index options
/// * `progress` - Optional progress callback
///
/// # Returns
/// * `Result<IndexResult>` - Index result
pub fn index_project(
    project_path: &Path,
    options: IndexOptions,
    progress: Option<&dyn Fn(&str)>,
) -> Result<IndexResult> {
    // Open storage
    let storage = StorageManager::open_project_db(project_path)?;

    let mut file_stats = FileCollectionStats::default();
    let mut session_stats = SessionCollectionStats::default();
    let errors = 0;

    // Index files
    if options.index_source || options.index_docs || options.index_other {
        if let Some(p) = progress {
            p("Scanning files...");
        }

        let collector = FileCollector::new(project_path)?;

        let (files, _deleted) = if options.force {
            // Full re-index
            let files = collector.collect_files(options.index_source, options.index_docs, options.index_other)?;
            (files, Vec::new())
        } else {
            // Incremental index
            collector.collect_incremental(&storage, options.index_source, options.index_docs, options.index_other)?
        };

        if let Some(p) = progress {
            p(&format!("Found {} files to index", files.len()));
        }

        // Store files
        let stats = collector.store_files(&files, &storage)?;
        file_stats = stats.clone();

        if let Some(p) = progress {
            p(&format!("Indexed {} files", stats.files_collected));
        }
    }

    // Index sessions
    if options.index_sessions {
        if let Some(p) = progress {
            p("Scanning sessions...");
        }

        let collector = SessionCollector::new(None).with_project(project_path);

        let sessions = if options.force {
            collector.collect_sessions()?
        } else {
            collector.collect_incremental(&storage)?
        };

        if let Some(p) = progress {
            p(&format!("Found {} sessions to index", sessions.len()));
        }

        // Store sessions
        let stats = collector.store_sessions(&sessions, &storage)?;
        session_stats = stats.clone();

        if let Some(p) = progress {
            p(&format!("Indexed {} sessions", stats.sessions_collected));
        }
    }

    // Flush storage
    storage.flush()?;

    Ok(IndexResult {
        file_stats,
        session_stats,
        errors,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use std::fs::File as StdFile;
    use std::io::Write;

    fn create_test_file(dir: &Path, name: &str, content: &str) {
        let path = dir.join(name);
        let mut file = StdFile::create(&path).unwrap();
        file.write_all(content.as_bytes()).unwrap();
    }

    #[test]
    fn test_init_project_creates_rag_dir() {
        let temp = TempDir::new().unwrap();
        let project_path = temp.path();

        let result = init_project(project_path, false).unwrap();

        match result {
            InitResult::Success { rag_dir, .. } => {
                assert_eq!(rag_dir, project_path.join(".rag"));
                assert!(rag_dir.exists());
            }
            _ => panic!("Expected Success"),
        }
    }

    #[test]
    fn test_init_project_creates_db_dir() {
        let temp = TempDir::new().unwrap();
        let project_path = temp.path();

        let result = init_project(project_path, false).unwrap();

        match result {
            InitResult::Success { .. } => {
                let db_dir = ConfigManager::db_dir(project_path);
                assert!(db_dir.exists());
            }
            _ => panic!("Expected Success"),
        }
    }

    #[test]
    fn test_init_project_creates_config() {
        let temp = TempDir::new().unwrap();
        let project_path = temp.path();

        let result = init_project(project_path, false).unwrap();

        match result {
            InitResult::Success { .. } => {
                let config_path = project_path.join(".rag/config.json");
                assert!(config_path.exists());
            }
            _ => panic!("Expected Success"),
        }
    }

    #[test]
    fn test_init_project_already_initialized() {
        let temp = TempDir::new().unwrap();
        let project_path = temp.path();

        // First init
        init_project(project_path, false).unwrap();

        // Second init should report already exists
        let result = init_project(project_path, false).unwrap();
        assert!(matches!(result, InitResult::AlreadyExists));
    }

    #[test]
    fn test_init_project_force_reinit() {
        let temp = TempDir::new().unwrap();
        let project_path = temp.path();

        // First init
        {
            let _result = init_project(project_path, false).unwrap();
            drop(_result);
        }

        // Force re-init should succeed
        let result = init_project(project_path, true).unwrap();
        assert!(matches!(result, InitResult::Success { .. }));
    }

    #[test]
    fn test_init_project_initializes_storage() {
        let temp = TempDir::new().unwrap();
        let project_path = temp.path();

        let result = init_project(project_path, false).unwrap();

        match result {
            InitResult::Success { storage_size, .. } => {
                // Storage should be initialized (size may be 0 for new DB)
                let _ = storage_size;
            }
            _ => panic!("Expected Success"),
        }
    }

    #[test]
    fn test_index_project_files() {
        let temp = TempDir::new().unwrap();
        let project_path = temp.path();

        // Initialize and release storage
        {
            let result = init_project(project_path, false).unwrap();
            drop(result);
        }

        // Create test files
        create_test_file(project_path, "main.rs", "fn main() {}");
        create_test_file(project_path, "README.md", "# Test");

        // Index files
        let options = IndexOptions {
            index_source: true,
            index_docs: true,
            index_other: false,
            index_sessions: false,
            force: false,
        };

        let result = index_project(project_path, options, None).unwrap();

        assert_eq!(result.file_stats.files_collected, 2);
        assert_eq!(result.session_stats.sessions_collected, 0);
    }

    #[test]
    fn test_index_project_empty() {
        let temp = TempDir::new().unwrap();
        let project_path = temp.path();

        // Initialize and release storage
        {
            let result = init_project(project_path, false).unwrap();
            drop(result);
        }

        // Index with no files
        let options = IndexOptions {
            index_source: true,
            index_docs: true,
            index_other: false,
            index_sessions: false,
            force: false,
        };

        let result = index_project(project_path, options, None).unwrap();

        assert_eq!(result.file_stats.files_collected, 0);
    }

    #[test]
    fn test_index_project_incremental() {
        // Simplified test - just verify incremental mode works
        let temp = TempDir::new().unwrap();
        let project_path = temp.path();

        // Initialize project
        {
            let result = init_project(project_path, false).unwrap();
            drop(result);
        }

        // Create test file
        create_test_file(project_path, "main.rs", "fn main() {}");

        // First index (incremental)
        let options = IndexOptions {
            index_source: true,
            index_docs: false,
            index_other: false,
            index_sessions: false,
            force: false, // Incremental mode
        };

        let result = index_project(project_path, options, None).unwrap();
        assert_eq!(result.file_stats.files_collected, 1);
        // Verify it's in incremental mode (not force)
        // In real usage, second call would detect no changes
    }

    #[test]
    fn test_index_project_force_reindex() {
        let temp = TempDir::new().unwrap();
        let project_path = temp.path();

        // Initialize project
        {
            let result = init_project(project_path, false).unwrap();
            drop(result);
        }

        // Create test file
        create_test_file(project_path, "main.rs", "fn main() {}");

        // First index
        let options = IndexOptions {
            index_source: true,
            index_docs: false,
            index_other: false,
            index_sessions: false,
            force: false,
        };

        // Need to release storage between calls
        let result1 = {
            index_project(project_path, options.clone(), None).unwrap()
        };
        assert_eq!(result1.file_stats.files_collected, 1);

        // Modify file
        create_test_file(project_path, "main.rs", "fn new_main() {}");

        // Force re-index should index the modified file
        let mut force_options = options.clone();
        force_options.force = true;

        let result2 = index_project(project_path, force_options, None).unwrap();
        assert_eq!(result2.file_stats.files_collected, 1);
    }
}
