//! Configuration management for Claude RAG.
//!
//! Handles global configuration from `~/.claude/rag/config.toml` and
//! project-specific configuration from `{projectPath}/.rag/config.json`.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::error::{Result, RagError};

/// Global configuration path.
const GLOBAL_CONFIG_PATH: &str = ".claude/rag/config.toml";

/// Project configuration filename.
const PROJECT_CONFIG_NAME: &str = ".rag/config.json";

/// Main configuration structure.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    /// Embedding API configuration.
    pub embedding: EmbeddingConfig,
    /// HNSW index parameters.
    pub hnsw: HnswConfig,
    /// Indexing options.
    pub index: IndexConfig,
    /// Daemon options.
    pub daemon: DaemonConfig,
    /// Git integration settings.
    pub git: GitConfig,
    /// Confidence/temporal weights.
    pub confidence: ConfidenceConfig,
    /// Retrieval options.
    pub retrieval: RetrievalConfig,
}

/// Embedding API configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingConfig {
    /// Zhipu AI API token.
    pub api_token: String,
    /// API URL.
    pub api_url: String,
    /// Vector dimensions.
    pub dimensions: usize,
    /// Batch request size.
    pub batch_size: usize,
    /// Request timeout in milliseconds.
    pub timeout_ms: u64,
}

impl Default for EmbeddingConfig {
    fn default() -> Self {
        Self {
            api_token: String::new(),
            api_url: "https://open.bigmodel.cn/api/paas/v4/embeddings".to_string(),
            dimensions: 1024,
            batch_size: 8,
            timeout_ms: 30000,
        }
    }
}

/// HNSW index parameters.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HnswConfig {
    /// Max connections per layer.
    pub m: usize,
    /// Search width during build.
    pub ef_construction: usize,
    /// Search width during query.
    pub ef_search: usize,
}

impl Default for HnswConfig {
    fn default() -> Self {
        Self {
            m: 16,
            ef_construction: 200,
            ef_search: 50,
        }
    }
}

/// Indexing options.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexConfig {
    /// Whether to index source files.
    pub index_source: bool,
    /// Whether to index documentation.
    pub index_docs: bool,
    /// Whether to index other files.
    pub index_other: bool,
}

impl Default for IndexConfig {
    fn default() -> Self {
        Self {
            index_source: true,
            index_docs: true,
            index_other: false,
        }
    }
}

/// Daemon options.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonConfig {
    /// Session timeout detection in seconds.
    pub session_timeout_seconds: u64,
    /// HNSW persist interval in seconds.
    pub persist_interval_seconds: u64,
    /// Socket path for daemon communication.
    pub socket_path: Option<String>,
    /// PID file path.
    pub pid_file: Option<String>,
    /// File event debounce delay in milliseconds.
    pub file_debounce_ms: u64,
    /// Graceful shutdown timeout in seconds.
    pub shutdown_timeout_seconds: u64,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            session_timeout_seconds: 60,
            persist_interval_seconds: 300,
            socket_path: None,
            pid_file: None,
            file_debounce_ms: 500,
            shutdown_timeout_seconds: 5,
        }
    }
}

/// Git integration settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitConfig {
    /// Enable Git history indexing.
    pub enable_git_indexing: bool,
    /// Maximum commits to index.
    pub max_commits_to_index: usize,
    /// Include diff content.
    pub include_diff_content: bool,
    /// Parse Conventional Commits.
    pub conventional_commits: bool,
    /// Extract BREAKING CHANGE.
    pub extract_breaking_changes: bool,
}

impl Default for GitConfig {
    fn default() -> Self {
        Self {
            enable_git_indexing: true,
            max_commits_to_index: 1000,
            include_diff_content: true,
            conventional_commits: true,
            extract_breaking_changes: true,
        }
    }
}

/// Confidence/temporal weights.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfidenceConfig {
    /// Current code weight.
    pub code_weight: f32,
    /// Git commit weight.
    pub git_commit_weight: f32,
    /// Session weight.
    pub session_weight: f32,
    /// Temporal decay rate per day (default: 0.05).
    pub decay_rate: f32,
}

impl Default for ConfidenceConfig {
    fn default() -> Self {
        Self {
            code_weight: 1.2,
            git_commit_weight: 1.0,
            session_weight: 0.7,
            decay_rate: 0.05,
        }
    }
}

/// Retrieval options.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetrievalConfig {
    /// Enable timeline building.
    pub enable_timeline: bool,
    /// Show Git context.
    pub show_git_context: bool,
}

impl Default for RetrievalConfig {
    fn default() -> Self {
        Self {
            enable_timeline: true,
            show_git_context: true,
        }
    }
}

/// Configuration manager.
pub struct ConfigManager;

impl ConfigManager {
    /// Load configuration with priority: project > global > default.
    pub fn load(project_path: Option<&Path>) -> Result<Config> {
        let mut config = Config::default();

        // Try to load global config
        if let Some(global_config) = Self::load_global_config()? {
            config = global_config;
        }

        // Override with project config if available
        if let Some(path) = project_path {
            if let Some(project_config) = Self::load_project_config(path)? {
                Self::merge_config(&mut config, project_config);
            }
        }

        // Validate required fields
        Self::validate(&config)?;

        Ok(config)
    }

    /// Load global configuration from `~/.claude/rag/config.toml`.
    fn load_global_config() -> Result<Option<Config>> {
        let home = dirs::home_dir()
            .ok_or_else(|| RagError::Config("Cannot determine home directory".to_string()))?;

        let config_path = home.join(GLOBAL_CONFIG_PATH);

        if !config_path.exists() {
            return Ok(None);
        }

        let content = std::fs::read_to_string(&config_path)
            .map_err(|e| RagError::Config(format!("Failed to read config: {} - {}", config_path.display(), e)))?;

        let config: Config = toml::from_str(&content)
            .map_err(|e| RagError::Config(format!("Invalid TOML: {}", e)))?;

        Ok(Some(config))
    }

    /// Load project configuration from `{projectPath}/.rag/config.json`.
    fn load_project_config(project_path: &Path) -> Result<Option<Config>> {
        let config_path = project_path.join(PROJECT_CONFIG_NAME);

        if !config_path.exists() {
            return Ok(None);
        }

        let content = std::fs::read_to_string(&config_path)
            .map_err(|e| RagError::Config(format!("Failed to read config: {} - {}", config_path.display(), e)))?;

        let config: Config = serde_json::from_str(&content)
            .map_err(|e| RagError::Config(format!("Invalid JSON: {}", e)))?;

        Ok(Some(config))
    }

    /// Merge project config into global config (project takes priority).
    fn merge_config(global: &mut Config, project: Config) {
        if !project.embedding.api_token.is_empty() {
            global.embedding = project.embedding;
        }
        global.hnsw = project.hnsw;
        global.index = project.index;
        global.daemon = project.daemon;
        global.git = project.git;
        global.confidence = project.confidence;
        global.retrieval = project.retrieval;
    }

    /// Validate required configuration fields.
    fn validate(config: &Config) -> Result<()> {
        if config.embedding.api_token.is_empty() {
            return Err(RagError::Config(
                "API token is required. Set it in ~/.claude/rag/config.toml or .rag/config.json".to_string(),
            ));
        }
        Ok(())
    }

    /// Get the project's .rag directory path.
    pub fn rag_dir(project_path: &Path) -> PathBuf {
        project_path.join(".rag")
    }

    /// Get the project's database directory path.
    pub fn db_dir(project_path: &Path) -> PathBuf {
        Self::rag_dir(project_path).join("db")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_default() {
        let config = Config::default();
        assert_eq!(config.embedding.dimensions, 1024);
        assert_eq!(config.hnsw.m, 16);
        assert!(config.index.index_source);
    }

    #[test]
    fn test_rag_dir() {
        let project = Path::new("/test/project");
        let rag_dir = ConfigManager::rag_dir(project);
        assert_eq!(rag_dir, PathBuf::from("/test/project/.rag"));
    }

    #[test]
    fn test_db_dir() {
        let project = Path::new("/test/project");
        let db_dir = ConfigManager::db_dir(project);
        assert_eq!(db_dir, PathBuf::from("/test/project/.rag/db"));
    }

    #[test]
    fn test_validate_missing_token() {
        let config = Config::default();
        let result = ConfigManager::validate(&config);
        assert!(matches!(result, Err(RagError::Config(_))));
    }

    #[test]
    fn test_validate_with_token() {
        let mut config = Config::default();
        config.embedding.api_token = "test-token".to_string();
        assert!(ConfigManager::validate(&config).is_ok());
    }
}
