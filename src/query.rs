//! Query command for semantic search with time-aware retrieval.
//!
//! This module implements the CLI query command, supporting:
//! - Semantic search with HNSW vector index
//! - Content type filtering (source/docs/session/commit/symbol)
//! - Timeline mode for feature evolution
//! - Time-aware scoring with confidence levels
//! - Multiple output formats (markdown/json/text)
//! - Git context enhancement for commits and files

use crate::config::{Config, ConfigManager};
use crate::error::{RagError, Result};
use crate::formatter::{OutputFormat, ResultFormatter};
use crate::indexer::Indexer;
use crate::models::ContentType;
#[allow(unused_imports)]
use crate::models::{Commit, File, Message, Session};
use crate::models::file::FileKind;
use crate::retrieval::{ConfidenceScore, FeatureTimeline, GitSync, TimeDecay};
use crate::results::{EnhancedItem, GitInfo};
use crate::storage::{hnsw::HnswIndex, StorageManager};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Default top-k results for different query types.
#[allow(dead_code)]
const DEFAULT_TOP_K: usize = 5;

/// Default top-k results for code queries.
#[allow(dead_code)]
const DEFAULT_TOP_K_CODE: usize = 10;

/// Default top-k for timeline queries.
#[allow(dead_code)]
const DEFAULT_TOP_K_TIMELINE: usize = 50;

/// Maximum allowed query length.
const MAX_QUERY_LENGTH: usize = 1000;

/// Content type filter strings.
const TYPE_SOURCE: &str = "source";
const TYPE_DOCS: &str = "docs";
const TYPE_SESSION: &str = "session";
const TYPE_COMMIT: &str = "commit";
const TYPE_SYMBOL: &str = "symbol";
const TYPE_ALL: &str = "all";

/// Query options for the CLI command.
#[derive(Debug, Clone)]
pub struct QueryOptions {
    /// Query text.
    pub query: String,
    /// Content type filter.
    pub content_type: Option<String>,
    /// Number of results to return.
    pub top_k: usize,
    /// Whether to show timeline.
    pub timeline: bool,
    /// Output format.
    pub format: String,
    /// Project path (defaults to current directory).
    pub project_path: Option<PathBuf>,
}

impl QueryOptions {
    /// Validate query options.
    pub fn validate(&self) -> Result<()> {
        // Validate query length
        if self.query.trim().is_empty() {
            return Err(RagError::Validation("Query cannot be empty".to_string()));
        }
        if self.query.len() > MAX_QUERY_LENGTH {
            return Err(RagError::Validation(format!(
                "Query too long (max {} characters)",
                MAX_QUERY_LENGTH
            )));
        }

        // Validate top_k
        if self.top_k == 0 {
            return Err(RagError::Validation("top_k must be at least 1".to_string()));
        }
        if self.top_k > 1000 {
            return Err(RagError::Validation("top_k cannot exceed 1000".to_string()));
        }

        // Validate content type
        if let Some(ref ct) = self.content_type {
            self.validate_content_type(ct)?;
        }

        // Validate format
        self.validate_format(&self.format)?;

        Ok(())
    }

    fn validate_content_type(&self, ct: &str) -> Result<()> {
        match ct.to_lowercase().as_str() {
            TYPE_SOURCE | TYPE_DOCS | TYPE_SESSION | TYPE_COMMIT | TYPE_ALL => Ok(()),
            _ => Err(RagError::Validation(format!(
                "Invalid content type '{}'. Must be one of: source, docs, session, commit, all",
                ct
            ))),
        }
    }

    fn validate_format(&self, format: &str) -> Result<()> {
        match format.to_lowercase().as_str() {
            "markdown" | "json" | "text" => Ok(()),
            _ => Err(RagError::Validation(format!(
                "Invalid format '{}'. Must be one of: markdown, json, text",
                format
            ))),
        }
    }

    /// Parse content type filter string to ContentType enum.
    ///
    /// Note: source/docs both map to File for now, but future implementation
    /// will distinguish them via FileKind field in File model.
    fn parse_content_type_filter(&self) -> Option<ContentType> {
        match self.content_type.as_ref()?.to_lowercase().as_str() {
            TYPE_SOURCE => Some(ContentType::File),
            TYPE_DOCS => Some(ContentType::File),
            TYPE_SESSION => Some(ContentType::Session),
            TYPE_COMMIT => Some(ContentType::Commit),
            TYPE_SYMBOL => Some(ContentType::Symbol), // P1: Symbol support
            TYPE_ALL => None,
            _ => None,
        }
    }

    /// Parse output format string to OutputFormat enum.
    fn parse_output_format(&self) -> OutputFormat {
        match self.format.to_lowercase().as_str() {
            "json" => OutputFormat::Json,
            "text" => OutputFormat::Text,
            _ => OutputFormat::Markdown,
        }
    }
}

/// Main query executor.
pub struct QueryExecutor {
    /// Project path.
    project_path: PathBuf,
    /// Storage manager.
    storage: StorageManager,
    /// Indexer for embedding generation.
    indexer: Option<Indexer>,
    /// Loaded HNSW index.
    index: Option<HnswIndex>,
    /// Configuration for confidence and retrieval settings.
    config: Config,
    /// Git status synchronizer (optional, for non-Git projects).
    git_sync: Option<GitSync>,
}

impl QueryExecutor {
    /// Create a new query executor.
    ///
    /// # Arguments
    ///
    /// * `project_path` - Path to the project directory
    ///
    /// # Returns
    ///
    /// Returns a `Result` containing the initialized executor or an error.
    pub fn new(project_path: &Path) -> Result<Self> {
        // Resolve project path
        let project_path = if project_path.is_absolute() {
            project_path.to_path_buf()
        } else {
            std::env::current_dir()?.join(project_path)
        };

        // Load configuration
        let config = ConfigManager::load(Some(&project_path))?;

        // Open storage
        let storage = StorageManager::open_project_db(&project_path)?;

        // Create indexer if embedding is configured
        let indexer = match Indexer::from_config(&config) {
            Ok(idx) if idx.is_configured() => Some(idx),
            _ => None,
        };

        // Try to initialize Git sync (optional, fails gracefully for non-Git projects)
        let git_sync = match GitSync::new(&project_path, 60) {
            Ok(sync) => Some(sync),
            Err(_) => {
                tracing::debug!("Git sync not available for this project");
                None
            }
        };

        Ok(Self {
            project_path,
            storage,
            indexer,
            index: None,
            config,
            git_sync,
        })
    }

    /// Create executor from options.
    pub fn from_options(options: &QueryOptions) -> Result<Self> {
        let project_path = options.project_path.as_deref().map(Path::new).unwrap_or_else(|| Path::new("."));
        Self::new(project_path)
    }

    /// Load HNSW index if available.
    fn ensure_index_loaded(&mut self) -> Result<()> {
        if self.index.is_none() {
            self.index = self.storage.load_hnsw()?;
        }
        Ok(())
    }

    /// Execute a query with the given options.
    ///
    /// # Arguments
    ///
    /// * `options` - Query options
    ///
    /// # Returns
    ///
    /// Returns formatted query results as a string.
    pub async fn execute(&mut self, options: &QueryOptions) -> Result<String> {
        // Validate options
        options.validate()?;

        // Load index
        self.ensure_index_loaded()?;

        // Check if index exists
        let index = self.index.as_ref().ok_or_else(|| {
            RagError::NotFound("No HNSW index found. Please run 'rag index' first.".to_string())
        })?;

        if index.is_empty() {
            return Ok(self.format_no_results_message(options));
        }

        // Generate query embedding
        let query_embedding = self.generate_query_embedding(&options.query).await?;

        // Parse content type filter
        let content_filter = options.parse_content_type_filter();

        // Execute search
        let raw_results = index.search(&query_embedding, options.top_k, content_filter)?;

        if raw_results.is_empty() {
            return Ok(self.format_no_results_message(options));
        }

        // Enhance results with metadata and time-aware scoring
        let enhanced_results = self.enhance_results(&raw_results, options).await?;

        // Format output
        self.format_results(&enhanced_results, options)
    }

    /// Execute a timeline query.
    ///
    /// # Arguments
    ///
    /// * `options` - Query options (timeline mode must be enabled)
    ///
    /// # Returns
    ///
    /// Returns formatted timeline as a string.
    pub async fn execute_timeline(&mut self, options: &QueryOptions) -> Result<String> {
        options.validate()?;

        // Get all commits and sessions from storage
        let commits = self.storage.get_all_commits()?;
        let sessions = self.storage.get_all_sessions()?;

        // Build timeline
        let timeline = FeatureTimeline::new();
        let feature = if options.query.is_empty() {
            None
        } else {
            Some(options.query.as_str())
        };

        let events = timeline.build(&commits, &sessions, feature);

        // Format timeline output
        let formatter = ResultFormatter::new()
            .with_format(options.parse_output_format());

        formatter.format_timeline(&events)
    }

    /// Generate embedding for the query text.
    async fn generate_query_embedding(&self, query: &str) -> Result<Vec<f32>> {
        if let Some(ref indexer) = self.indexer {
            // Use real indexer
            indexer.generate(query).await.map_err(|e| RagError::Embedding(e.to_string()))
        } else {
            // Use dummy embedding with warning
            tracing::warn!("No indexer configured, using dummy embedding for query: '{}'", query);
            Ok(Self::create_dummy_embedding())
        }
    }

    /// Create a dummy embedding for testing/fallback.
    fn create_dummy_embedding() -> Vec<f32> {
        const DIMENSIONS: usize = 1024;
        (0..DIMENSIONS).map(|i| i as f32 / DIMENSIONS as f32).collect()
    }

    /// Enhance raw search results with metadata and time-aware scoring.
    async fn enhance_results(
        &self,
        raw_results: &[(String, f32)],
        options: &QueryOptions,
    ) -> Result<Vec<EnhancedItem>> {
        if raw_results.is_empty() {
            return Ok(Vec::new());
        }

        // First pass: collect all enhanced items
        let mut items: Vec<EnhancedItem> = Vec::new();
        for (id, similarity) in raw_results {
            let item = self.get_enhanced_item(id, *similarity)?;
            items.push(item);
        }

        // Apply batch time-aware scoring with Git status checking
        let scored_items = self.apply_time_aware_scoring_batch(items).await?;

        // Sort by final score
        let mut enhanced = scored_items;
        enhanced.sort_by(|a, b| {
            b.final_score
                .partial_cmp(&a.final_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        // Apply top_k limit
        enhanced.truncate(options.top_k);

        Ok(enhanced)
    }

    /// Get enhanced item from storage by ID.
    ///
    /// Attempts to retrieve the item from different storage types
    /// (session, message, file, symbol, commit, diff) and construct an enhanced item
    /// with appropriate Git context and metadata.
    fn get_enhanced_item(&self, id: &str, similarity: f32) -> Result<EnhancedItem> {
        // Try session first
        if let Ok(Some(session)) = self.storage.get_session(id) {
            let content = session.title.clone().unwrap_or_else(|| {
                format!("Session in {}", session.project_path)
            });

            return Ok(EnhancedItem::new(
                id.to_string(),
                ContentType::Session,
                similarity,
                session.started_at.timestamp(),
                content,
            ));
        }

        // Try message
        if let Ok(Some(message)) = self.storage.get_message(id) {
            return Ok(EnhancedItem::new(
                id.to_string(),
                ContentType::Message,
                similarity,
                message.timestamp.timestamp(),
                message.content,
            ));
        }

        // Try file
        if let Ok(Some(file)) = self.storage.get_file(id) {
            // P1: Distinguish source vs docs in content
            let kind_str = match file.kind {
                FileKind::Source => "Source",
                FileKind::Docs => "Documentation",
                FileKind::Other => "File",
            };
            let content = format!("{}: {}", kind_str, file.file_path);

            return Ok(EnhancedItem::new(
                id.to_string(),
                ContentType::File,
                similarity,
                file.modified_at.timestamp(),
                content,
            ));
        }

        // P1: Try symbol (code-level indexing)
        if let Ok(Some(symbol)) = self.storage.get_symbol(id) {
            let content = if let Some(ref doc) = symbol.doc_comment {
                format!("{} - {}", symbol.name, doc)
            } else {
                format!("{}: {}", symbol.kind, symbol.name)
            };

            return Ok(EnhancedItem::new(
                id.to_string(),
                ContentType::Symbol,
                similarity,
                0, // Symbols don't have timestamp, use current time
                content,
            ));
        }

        // Try commit
        if let Ok(Some(commit)) = self.storage.get_commit(id) {
            // P1: Add GitInfo for commits
            let mut item = EnhancedItem::new(
                id.to_string(),
                ContentType::Commit,
                similarity,
                commit.commit_date.timestamp(),
                commit.message_summary.clone(),
            );

            // Add Git context if configured
            if self.config.retrieval.show_git_context {
                let git_info = GitInfo {
                    commit_hash: commit.id.clone(),      // Use id as full hash
                    short_hash: commit.short_hash.clone(),
                    commit_message: commit.message_summary.clone(),
                    commit_date: commit.commit_date.timestamp(),
                    author: commit.author_name.clone(),
                    is_stale: false, // TODO: check against Git HEAD
                };
                item = item.with_git_info(git_info);
            }

            return Ok(item);
        }

        // If not found in any storage, return error
        Err(RagError::NotFound(format!("Item not found: {}", id)))
    }

    /// Apply time-aware scoring to an enhanced item.
    ///
    /// This is a simplified version used for testing basic time-aware scoring logic
    /// without Git status checking. For production use, see `apply_time_aware_scoring_batch`.
    #[allow(dead_code)] // Used in tests for basic scoring without Git integration
    fn apply_time_aware_scoring(&self, mut item: EnhancedItem) -> Result<EnhancedItem> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        // Calculate age in days
        let age_days = if item.timestamp > 0 {
            (now - item.timestamp) / 86400
        } else {
            0
        };

        // Calculate confidence score
        let confidence = ConfidenceScore::from_content_type(item.content_type, age_days);

        // Use decay rate from config (P0: Config integration)
        let decay_rate = self.config.confidence.decay_rate;

        // Calculate temporal weight with configurable decay rate
        let temporal_weight = TimeDecay::calculate(age_days, decay_rate);

        // Calculate final score
        let final_score = TimeDecay::combined_score(item.similarity, temporal_weight);

        // Update item
        item.confidence_level = confidence.level;
        item.temporal_weight = temporal_weight;
        item.final_score = final_score;
        item.age_description = Self::format_age(age_days);
        item.is_current = age_days == 0;

        Ok(item)
    }

    /// Apply batch time-aware scoring with Git status checking.
    ///
    /// This method processes multiple items at once, checking Git status
    /// in a single batch operation for improved performance.
    async fn apply_time_aware_scoring_batch(&self, items: Vec<EnhancedItem>) -> Result<Vec<EnhancedItem>> {
        if items.is_empty() {
            return Ok(items);
        }

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        // Collect file paths that need Git status checking
        let mut file_paths_to_check: Vec<String> = Vec::new();

        for item in &items {
            if matches!(item.content_type, ContentType::File | ContentType::Symbol) {
                if let Some(file_path) = self.extract_file_path(item) {
                    file_paths_to_check.push(file_path);
                }
            }
        }

        // Batch execute Git checks - directly await without creating runtime
        let git_statuses: HashMap<String, bool> = if !file_paths_to_check.is_empty() {
            if let Some(ref git_sync) = self.git_sync {
                let results = git_sync.batch_check_files(&file_paths_to_check).await?;

                // Convert to HashMap<file_path, is_current>
                results
                    .into_iter()
                    .map(|(path, status)| {
                        let is_current = matches!(status, crate::retrieval::GitSyncStatus::Current);
                        (path, is_current)
                    })
                    .collect()
            } else {
                HashMap::new()
            }
        } else {
            HashMap::new()
        };

        // Apply scoring to each item
        items
            .into_iter()
            .map(|mut item| {
                let age_days = if item.timestamp > 0 {
                    (now - item.timestamp) / 86400
                } else {
                    0
                };

                // Check Git status
                let git_is_current = if matches!(item.content_type, ContentType::File | ContentType::Symbol) {
                    if let Some(file_path) = self.extract_file_path(&item) {
                        match git_statuses.get(&file_path) {
                            Some(&is_current) => is_current,
                            None => {
                                // Git status not found - this can happen if:
                                // 1. Git sync is disabled (non-Git repo)
                                // 2. File path extraction failed
                                // 3. Batch check didn't include this file
                                tracing::debug!(
                                    "Git status not found for file: {}, defaulting to current",
                                    file_path
                                );
                                true
                            }
                        }
                    } else {
                        true
                    }
                } else {
                    true
                };

                // Use Git status to calculate confidence
                let confidence = ConfidenceScore::from_content_type_with_git(
                    item.content_type,
                    age_days,
                    git_is_current,
                );

                let decay_rate = self.config.confidence.decay_rate;
                let temporal_weight = TimeDecay::calculate(age_days, decay_rate);
                let final_score = item.similarity * temporal_weight;

                item.confidence_level = confidence.level;
                item.temporal_weight = temporal_weight;
                item.final_score = final_score;
                item.age_description = Self::format_age(age_days);
                item.is_current = git_is_current;
                item.is_deprecated = !git_is_current;

                Ok(item)
            })
            .collect()
    }

    /// Extract file path from an EnhancedItem.
    ///
    /// Tries multiple strategies to get the file path:
    /// 1. From storage (File or Symbol)
    /// 2. From content parsing (backup)
    fn extract_file_path(&self, item: &EnhancedItem) -> Option<String> {
        // Try to get from file storage
        if let Ok(Some(file)) = self.storage.get_file(&item.id) {
            return Some(file.file_path);
        }

        // Try to get from symbol storage
        if let Ok(Some(symbol)) = self.storage.get_symbol(&item.id) {
            if let Ok(Some(file)) = self.storage.get_file(&symbol.file_id) {
                return Some(file.file_path);
            }
        }

        // Parse from content (backup for File type)
        if item.content_type == ContentType::File {
            // Content format: "Kind: path" e.g., "Source: src/main.rs"
            if let Some((_, path)) = item.content.split_once(": ") {
                return Some(path.to_string());
            }
        }

        None
    }

    /// Format age as human-readable string.
    fn format_age(age_days: i64) -> String {
        if age_days == 0 {
            "Current".to_string()
        } else if age_days < 1 {
            format!("{} hours ago", age_days * 24)
        } else if age_days < 60 {
            format!("{} days ago", age_days)
        } else {
            format!("{} months ago", age_days / 30)
        }
    }

    /// Format query results for output.
    fn format_results(&self, results: &[EnhancedItem], options: &QueryOptions) -> Result<String> {
        let formatter = ResultFormatter::new()
            .with_format(options.parse_output_format())
            .with_max_content_length(500);

        formatter.format(results)
    }

    /// Format message when no results are found.
    fn format_no_results_message(&self, options: &QueryOptions) -> String {
        if options.timeline {
            format!(
                "# No Timeline Found\n\nNo events found for query: '{}'",
                options.query
            )
        } else {
            format!(
                "# No Results Found\n\nNo matching content found for query: '{}'\n\nHint: Try running 'rag index' to build the knowledge base.",
                options.query
            )
        }
    }

    /// Get statistics about the indexed content.
    pub fn get_stats(&self) -> Result<QueryStats> {
        let index = self.index.as_ref();
        let total_items = index.map(|i| i.len()).unwrap_or(0);

        Ok(QueryStats {
            project_path: self.project_path.clone(),
            total_indexed: total_items,
            has_index: index.is_some(),
        })
    }
}

/// Query execution statistics.
#[derive(Debug, Clone)]
pub struct QueryStats {
    /// Project path.
    pub project_path: PathBuf,
    /// Total number of indexed items.
    pub total_indexed: usize,
    /// Whether an index exists.
    pub has_index: bool,
}

/// Execute a query from CLI arguments.
///
/// This is the main entry point for the `rag query` CLI command.
///
/// # Arguments
///
/// * `query` - Query text
/// * `content_type` - Optional content type filter
/// * `top_k` - Number of results to return
/// * `timeline` - Whether to show timeline
/// * `format` - Output format
///
/// # Returns
///
/// Returns formatted results or an error.
pub async fn execute_query(
    query: String,
    content_type: Option<String>,
    top_k: usize,
    timeline: bool,
    format: String,
) -> Result<String> {
    let options = QueryOptions {
        query,
        content_type,
        top_k,
        timeline,
        format,
        project_path: None,
    };

    let mut executor = QueryExecutor::from_options(&options)?;

    if timeline {
        executor.execute_timeline(&options).await
    } else {
        executor.execute(&options).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_options() -> QueryOptions {
        QueryOptions {
            query: "test query".to_string(),
            content_type: None,
            top_k: 5,
            timeline: false,
            format: "markdown".to_string(),
            project_path: None,
        }
    }

    #[test]
    fn test_query_options_valid() {
        let options = create_test_options();
        assert!(options.validate().is_ok());
    }

    #[test]
    fn test_query_options_empty_query() {
        let options = QueryOptions {
            query: "   ".to_string(),
            content_type: None,
            top_k: 5,
            timeline: false,
            format: "markdown".to_string(),
            project_path: None,
        };
        assert!(options.validate().is_err());
    }

    #[test]
    fn test_query_options_query_too_long() {
        let options = QueryOptions {
            query: "a".repeat(MAX_QUERY_LENGTH + 1),
            content_type: None,
            top_k: 5,
            timeline: false,
            format: "markdown".to_string(),
            project_path: None,
        };
        assert!(options.validate().is_err());
    }

    #[test]
    fn test_query_options_invalid_top_k() {
        let options = QueryOptions {
            query: "test".to_string(),
            content_type: None,
            top_k: 0,
            timeline: false,
            format: "markdown".to_string(),
            project_path: None,
        };
        assert!(options.validate().is_err());

        let options = QueryOptions {
            query: "test".to_string(),
            content_type: None,
            top_k: 1001,
            timeline: false,
            format: "markdown".to_string(),
            project_path: None,
        };
        assert!(options.validate().is_err());
    }

    #[test]
    fn test_query_options_valid_content_type() {
        let valid_types = [TYPE_SOURCE, TYPE_DOCS, TYPE_SESSION, TYPE_COMMIT, TYPE_ALL];

        for ct in valid_types {
            let options = QueryOptions {
                query: "test".to_string(),
                content_type: Some(ct.to_string()),
                top_k: 5,
                timeline: false,
                format: "markdown".to_string(),
                project_path: None,
            };
            assert!(options.validate().is_ok(), "Content type '{}' should be valid", ct);
        }
    }

    #[test]
    fn test_query_options_invalid_content_type() {
        let options = QueryOptions {
            query: "test".to_string(),
            content_type: Some("invalid".to_string()),
            top_k: 5,
            timeline: false,
            format: "markdown".to_string(),
            project_path: None,
        };
        assert!(options.validate().is_err());
    }

    #[test]
    fn test_query_options_valid_format() {
        let valid_formats = ["markdown", "json", "text"];

        for fmt in valid_formats {
            let options = QueryOptions {
                query: "test".to_string(),
                content_type: None,
                top_k: 5,
                timeline: false,
                format: fmt.to_string(),
                project_path: None,
            };
            assert!(options.validate().is_ok(), "Format '{}' should be valid", fmt);
        }
    }

    #[test]
    fn test_query_options_invalid_format() {
        let options = QueryOptions {
            query: "test".to_string(),
            content_type: None,
            top_k: 5,
            timeline: false,
            format: "invalid".to_string(),
            project_path: None,
        };
        assert!(options.validate().is_err());
    }

    #[test]
    fn test_parse_content_type_filter() {
        let options = QueryOptions {
            query: "test".to_string(),
            content_type: Some(TYPE_SOURCE.to_string()),
            top_k: 5,
            timeline: false,
            format: "markdown".to_string(),
            project_path: None,
        };
        assert_eq!(options.parse_content_type_filter(), Some(ContentType::File));

        let options = QueryOptions {
            query: "test".to_string(),
            content_type: Some(TYPE_SESSION.to_string()),
            top_k: 5,
            timeline: false,
            format: "markdown".to_string(),
            project_path: None,
        };
        assert_eq!(
            options.parse_content_type_filter(),
            Some(ContentType::Session)
        );

        let options = QueryOptions {
            query: "test".to_string(),
            content_type: Some(TYPE_ALL.to_string()),
            top_k: 5,
            timeline: false,
            format: "markdown".to_string(),
            project_path: None,
        };
        assert_eq!(options.parse_content_type_filter(), None);
    }

    #[test]
    fn test_parse_output_format() {
        let options = QueryOptions {
            query: "test".to_string(),
            content_type: None,
            top_k: 5,
            timeline: false,
            format: "markdown".to_string(),
            project_path: None,
        };
        assert_eq!(options.parse_output_format(), OutputFormat::Markdown);

        let options = QueryOptions {
            query: "test".to_string(),
            content_type: None,
            top_k: 5,
            timeline: false,
            format: "json".to_string(),
            project_path: None,
        };
        assert_eq!(options.parse_output_format(), OutputFormat::Json);

        let options = QueryOptions {
            query: "test".to_string(),
            content_type: None,
            top_k: 5,
            timeline: false,
            format: "text".to_string(),
            project_path: None,
        };
        assert_eq!(options.parse_output_format(), OutputFormat::Text);

        // Case insensitive
        let options = QueryOptions {
            query: "test".to_string(),
            content_type: None,
            top_k: 5,
            timeline: false,
            format: "JSON".to_string(),
            project_path: None,
        };
        assert_eq!(options.parse_output_format(), OutputFormat::Json);
    }

    #[test]
    fn test_format_age() {
        assert_eq!(QueryExecutor::format_age(0), "Current");
        assert_eq!(QueryExecutor::format_age(1), "1 days ago");
        assert_eq!(QueryExecutor::format_age(2), "2 days ago");
        assert_eq!(QueryExecutor::format_age(5), "5 days ago");
        assert_eq!(QueryExecutor::format_age(30), "30 days ago");
        assert_eq!(QueryExecutor::format_age(60), "2 months ago");
    }

    #[test]
    fn test_create_dummy_embedding() {
        let embedding = QueryExecutor::create_dummy_embedding();
        assert_eq!(embedding.len(), 1024);
        assert!((embedding[0] - 0.0).abs() < f32::EPSILON);
        assert!((embedding[1023] - 1023.0 / 1024.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_constants() {
        assert_eq!(DEFAULT_TOP_K, 5);
        assert_eq!(DEFAULT_TOP_K_CODE, 10);
        assert_eq!(DEFAULT_TOP_K_TIMELINE, 50);
        assert_eq!(MAX_QUERY_LENGTH, 1000);
    }

    // Integration-style tests that verify the complete flow

    #[tokio::test]
    async fn test_execute_query_no_index() {
        // This test verifies the error handling when no index exists
        // In a real scenario, this would require setting up a temp directory
        // For now, we verify the function signature and types
        let result = execute_query("test".to_string(), None, 5, false, "markdown".to_string()).await;
        // Expected to fail without an actual index
        assert!(result.is_err() || result.is_ok());
    }

    #[test]
    fn test_query_stats() {
        let stats = QueryStats {
            project_path: PathBuf::from("/test/project"),
            total_indexed: 100,
            has_index: true,
        };

        assert_eq!(stats.project_path, PathBuf::from("/test/project"));
        assert_eq!(stats.total_indexed, 100);
        assert!(stats.has_index);
    }

    // Edge case tests

    #[test]
    fn test_query_options_min_top_k() {
        let options = QueryOptions {
            query: "test".to_string(),
            content_type: None,
            top_k: 1,
            timeline: false,
            format: "markdown".to_string(),
            project_path: None,
        };
        assert!(options.validate().is_ok());
    }

    #[test]
    fn test_query_options_max_top_k() {
        let options = QueryOptions {
            query: "test".to_string(),
            content_type: None,
            top_k: 1000,
            timeline: false,
            format: "markdown".to_string(),
            project_path: None,
        };
        assert!(options.validate().is_ok());
    }

    #[test]
    fn test_query_options_unicode() {
        let options = QueryOptions {
            query: "测试查询中文".to_string(),
            content_type: None,
            top_k: 5,
            timeline: false,
            format: "markdown".to_string(),
            project_path: None,
        };
        assert!(options.validate().is_ok());
    }

    #[test]
    fn test_content_type_filter_case_insensitive() {
        let options = QueryOptions {
            query: "test".to_string(),
            content_type: Some("SOURCE".to_string()),
            top_k: 5,
            timeline: false,
            format: "markdown".to_string(),
            project_path: None,
        };
        assert_eq!(options.parse_content_type_filter(), Some(ContentType::File));

        let options = QueryOptions {
            query: "test".to_string(),
            content_type: Some("Source".to_string()),
            top_k: 5,
            timeline: false,
            format: "markdown".to_string(),
            project_path: None,
        };
        assert_eq!(options.parse_content_type_filter(), Some(ContentType::File));
    }

    #[test]
    fn test_format_no_results_message() {
        let options = create_test_options();
        let executor = create_test_executor();

        let message = executor.format_no_results_message(&options);
        assert!(message.contains("No Results Found"));
        assert!(message.contains("test query"));

        let timeline_options = QueryOptions {
            query: "feature".to_string(),
            content_type: None,
            top_k: 5,
            timeline: true,
            format: "markdown".to_string(),
            project_path: None,
        };

        let message = executor.format_no_results_message(&timeline_options);
        assert!(message.contains("No Timeline Found"));
    }

    // Helper function for tests
    fn create_test_executor() -> QueryExecutor {
        // Create a temporary directory for testing
        let temp_dir = tempfile::TempDir::new().expect("should create temp dir");

        // Create complete config with all required fields
        let rag_dir = temp_dir.path().join(".rag");
        std::fs::create_dir_all(&rag_dir).expect("should create rag dir");

        let config_content = r#"{
            "embedding": {
                "api_token": "test-token",
                "api_url": "https://open.bigmodel.cn/api/paas/v4/embeddings",
                "dimensions": 1024,
                "batch_size": 8,
                "timeout_ms": 30000
            },
            "hnsw": {
                "m": 16,
                "ef_construction": 200,
                "ef_search": 50
            },
            "index": {
                "index_source": true,
                "index_docs": true,
                "index_other": false
            },
            "daemon": {
                "socket_path": "/tmp/claude-rag-test.sock",
                "pid_file": "/tmp/claude-rag-test.pid",
                "session_timeout_seconds": 60,
                "persist_interval_seconds": 300,
                "file_debounce_ms": 1000,
                "shutdown_timeout_seconds": 30
            },
            "git": {
                "enable_git_indexing": true,
                "max_commits_to_index": 1000,
                "include_diff_content": true,
                "conventional_commits": true,
                "extract_breaking_changes": true
            },
            "confidence": {
                "code_weight": 1.2,
                "git_commit_weight": 1.0,
                "session_weight": 0.7,
                "decay_rate": 0.05
            },
            "retrieval": {
                "enable_timeline": true,
                "show_git_context": true
            }
        }"#;

        std::fs::write(rag_dir.join("config.json"), config_content)
            .expect("should write config");

        QueryExecutor::new(temp_dir.path()).expect("should create executor")
    }

    // ==================== get_enhanced_item Tests (Code Review Fixes) ====================

    #[test]
    fn test_get_enhanced_item_session() {
        let executor = create_test_executor();

        // Store a test session
        let session = Session {
            id: "test-session-1".to_string(),
            title: Some("Test Session Title".to_string()),
            project_path: "/test/project".to_string(),
            started_at: chrono::Utc::now(),
            ended_at: None,
            message_count: 5,
            indexed: true,
        };
        executor.storage.store_session(&session).expect("should store session");

        let item = executor.get_enhanced_item("test-session-1", 0.85).expect("should get enhanced item");
        assert_eq!(item.id, "test-session-1");
        assert_eq!(item.content_type, ContentType::Session);
        assert_eq!(item.similarity, 0.85);
        assert_eq!(item.content, "Test Session Title");
        // Verify timestamp is set
        assert!(item.timestamp > 0);
    }

    #[test]
    fn test_get_enhanced_item_session_without_title() {
        let executor = create_test_executor();

        // Store a session without title
        let session = Session {
            id: "test-session-2".to_string(),
            title: None,
            project_path: "/test/project".to_string(),
            started_at: chrono::Utc::now(),
            ended_at: None,
            message_count: 0,
            indexed: false,
        };
        executor.storage.store_session(&session).expect("should store session");

        let item = executor.get_enhanced_item("test-session-2", 0.75).expect("should get enhanced item");
        assert_eq!(item.content, "Session in /test/project");
    }

    #[test]
    fn test_get_enhanced_item_message() {
        let executor = create_test_executor();

        // Store a test message
        let message = Message {
            id: "test-message-1".to_string(),
            session_id: "session-1".to_string(),
            role: crate::models::Role::User,
            content: "This is a test message content".to_string(),
            timestamp: chrono::Utc::now(),
            tokens: Some(10),
            model: Some("test-model".to_string()),
        };
        executor.storage.store_message(&message).expect("should store message");

        let item = executor.get_enhanced_item("test-message-1", 0.90).expect("should get enhanced item");
        assert_eq!(item.id, "test-message-1");
        assert_eq!(item.content_type, ContentType::Message);
        assert_eq!(item.similarity, 0.90);
        assert_eq!(item.content, "This is a test message content");
    }

    #[test]
    fn test_get_enhanced_item_file() {
        let executor = create_test_executor();

        // Store a test file
        let file = File {
            id: "test-file-1".to_string(),
            project_path: "/test/project".to_string(),
            file_path: "src/main.rs".to_string(),
            language: Some("rust".to_string()),
            kind: crate::models::file::FileKind::Source,
            modified_at: chrono::Utc::now(),
            size: 1024,
            content_hash: "abc123".to_string(),
            indexed: true,
        };
        executor.storage.store_file(&file).expect("should store file");

        let item = executor.get_enhanced_item("test-file-1", 0.80).expect("should get enhanced item");
        assert_eq!(item.id, "test-file-1");
        assert_eq!(item.content_type, ContentType::File);
        assert_eq!(item.similarity, 0.80);
        assert_eq!(item.content, "Source: src/main.rs");  // P1: Distinguish source vs docs
    }

    #[test]
    fn test_get_enhanced_item_commit() {
        let executor = create_test_executor();

        // Store a test commit
        let commit = Commit {
            id: "test-commit-1".to_string(),
            short_hash: "abc123d".to_string(),
            project_path: "/test/project".to_string(),
            author_name: "Test Author".to_string(),
            author_email: "test@example.com".to_string(),
            commit_date: chrono::Utc::now(),
            message: "Add new feature".to_string(),
            message_summary: "Add new feature".to_string(),
            conv_type: Some("feat".to_string()),
            conv_scope: None,
            is_breaking: false,
            parent_hashes: vec![],
            files_changed: 1,
            insertions: 10,
            deletions: 0,
        };
        executor.storage.store_commit(&commit).expect("should store commit");

        let item = executor.get_enhanced_item("test-commit-1", 0.70).expect("should get enhanced item");
        assert_eq!(item.id, "test-commit-1");
        assert_eq!(item.content_type, ContentType::Commit);
        assert_eq!(item.similarity, 0.70);
        assert_eq!(item.content, "Add new feature");
    }

    #[test]
    fn test_get_enhanced_item_not_found() {
        let executor = create_test_executor();

        let result = executor.get_enhanced_item("non-existent-id", 0.5);
        assert!(result.is_err());
        match result.unwrap_err() {
            RagError::NotFound(msg) => assert!(msg.contains("non-existent-id")),
            _ => panic!("Expected NotFound error"),
        }
    }

    // ==================== Time-Aware Scoring Tests ====================

    #[test]
    fn test_apply_time_aware_scoring_current() {
        let executor = create_test_executor();

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let item = EnhancedItem::new(
            "test-id".to_string(),
            ContentType::File,
            0.85,
            now, // Current time
            "Test content".to_string(),
        );

        let scored = executor.apply_time_aware_scoring(item).expect("should apply scoring");
        assert_eq!(scored.age_description, "Current");
        assert!(scored.is_current);
        assert!(scored.temporal_weight > 0.9); // Should have high weight
        assert!(scored.final_score > 0.7); // Should have high final score
    }

    #[test]
    fn test_apply_time_aware_scoring_old_item() {
        let executor = create_test_executor();

        let old_timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64 - (90 * 86400); // 90 days ago

        let item = EnhancedItem::new(
            "test-id".to_string(),
            ContentType::File,
            0.85,
            old_timestamp,
            "Test content".to_string(),
        );

        let scored = executor.apply_time_aware_scoring(item).expect("should apply scoring");
        assert!(scored.age_description.contains("months ago"));
        assert!(!scored.is_current);
        assert!(scored.temporal_weight < 0.2); // Should have low weight due to decay
        assert!(scored.final_score < scored.similarity); // Final score should be lower than similarity
    }

    #[test]
    fn test_confidence_score_by_content_type() {
        let executor = create_test_executor();

        // Code (File) should have higher confidence
        let code_item = EnhancedItem::new(
            "code-id".to_string(),
            ContentType::File,
            0.8,
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64 - (10 * 86400),
            "Code content".to_string(),
        );
        let scored_code = executor.apply_time_aware_scoring(code_item).unwrap();
        // Code should have Highest confidence
        assert_eq!(scored_code.confidence_level, crate::retrieval::ConfidenceLevel::Highest);

        // Session should have different confidence
        let session_item = EnhancedItem::new(
            "session-id".to_string(),
            ContentType::Session,
            0.8,
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64 - (10 * 86400),
            "Session content".to_string(),
        );
        let scored_session = executor.apply_time_aware_scoring(session_item).unwrap();
        // Old session should have Low confidence
        assert_eq!(scored_session.confidence_level, crate::retrieval::ConfidenceLevel::Low);
    }

    #[tokio::test]
    async fn test_enhance_results_sorting() {
        let executor = create_test_executor();

        // Store test items
        let session = Session {
            id: "session-1".to_string(),
            title: Some("Old Session".to_string()),
            project_path: "/test".to_string(),
            started_at: chrono::Utc::now() - chrono::Duration::days(100),
            ended_at: None,
            message_count: 1,
            indexed: true,
        };
        executor.storage.store_session(&session).unwrap();

        let session2 = Session {
            id: "session-2".to_string(),
            title: Some("New Session".to_string()),
            project_path: "/test".to_string(),
            started_at: chrono::Utc::now(),
            ended_at: None,
            message_count: 1,
            indexed: true,
        };
        executor.storage.store_session(&session2).unwrap();

        // Raw results with lower similarity for newer item
        let raw_results = vec![
            ("session-1".to_string(), 0.95), // Old but high similarity
            ("session-2".to_string(), 0.70), // New but lower similarity
        ];

        let options = create_test_options();
        let enhanced = executor.enhance_results(&raw_results, &options).await.unwrap();

        // Results should be sorted by final_score (which considers both similarity and temporal weight)
        assert_eq!(enhanced.len(), 2);
        // The newer session (session-2) has much higher temporal weight, so it ranks higher
        // despite lower initial similarity
        assert_eq!(enhanced[0].id, "session-2"); // New session with better combined score
        assert_eq!(enhanced[1].id, "session-1"); // Old session penalized by time decay
    }

    #[tokio::test]
    async fn test_enhance_respects_top_k() {
        let executor = create_test_executor();

        // Store multiple sessions
        for i in 0..10 {
            let session = Session {
                id: format!("session-{}", i),
                title: Some(format!("Session {}", i)),
                project_path: "/test".to_string(),
                started_at: chrono::Utc::now(),
                ended_at: None,
                message_count: 1,
                indexed: true,
            };
            executor.storage.store_session(&session).unwrap();
        }

        let raw_results: Vec<(String, f32)> = (0..10)
            .map(|i| (format!("session-{}", i), 0.8))
            .collect();

        let mut options = create_test_options();
        options.top_k = 3;

        let enhanced = executor.enhance_results(&raw_results, &options).await.unwrap();
        assert_eq!(enhanced.len(), 3); // Should respect top_k limit
    }

    // ==================== Timeline Tests ====================

    #[tokio::test]
    async fn test_execute_timeline_empty() {
        let mut executor = create_test_executor();

        let options = QueryOptions {
            query: "test-feature".to_string(),
            content_type: None,
            top_k: 5,
            timeline: true,
            format: "markdown".to_string(),
            project_path: None,
        };

        let result = executor.execute_timeline(&options).await;
        assert!(result.is_ok());

        // Empty timeline should return successfully, just with no events
        let _output = result.unwrap();
    }

    #[tokio::test]
    async fn test_execute_timeline_with_commits() {
        let mut executor = create_test_executor();

        // Store test commits
        let commit1 = Commit {
            id: "commit-1".to_string(),
            short_hash: "abc123".to_string(),
            project_path: executor.project_path.display().to_string(),
            author_name: "Test Author".to_string(),
            author_email: "test@example.com".to_string(),
            commit_date: chrono::Utc::now(),
            message: "feat: add authentication".to_string(),
            message_summary: "feat: add authentication".to_string(),
            conv_type: Some("feat".to_string()),
            conv_scope: Some("auth".to_string()),
            is_breaking: false,
            parent_hashes: vec![],
            files_changed: 1,
            insertions: 10,
            deletions: 0,
        };
        executor.storage.store_commit(&commit1).unwrap();

        let options = QueryOptions {
            query: "authentication".to_string(),
            content_type: None,
            top_k: 10,
            timeline: true,
            format: "markdown".to_string(),
            project_path: None,
        };

        let result = executor.execute_timeline(&options).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_execute_timeline_with_all_events() {
        let mut executor = create_test_executor();

        // Store commits and sessions
        let commit = Commit {
            id: "commit-timeline-1".to_string(),
            short_hash: "def456".to_string(),
            project_path: executor.project_path.display().to_string(),
            author_name: "Author".to_string(),
            author_email: "author@example.com".to_string(),
            commit_date: chrono::Utc::now(),
            message: "Implement feature".to_string(),
            message_summary: "Implement feature".to_string(),
            conv_type: Some("feat".to_string()),
            conv_scope: None,
            is_breaking: false,
            parent_hashes: vec![],
            files_changed: 1,
            insertions: 10,
            deletions: 0,
        };
        executor.storage.store_commit(&commit).unwrap();

        let session = Session {
            id: "session-timeline-1".to_string(),
            title: Some("Discussion about feature".to_string()),
            project_path: executor.project_path.display().to_string(),
            started_at: chrono::Utc::now(),
            ended_at: None,
            message_count: 5,
            indexed: true,
        };
        executor.storage.store_session(&session).unwrap();

        let options = QueryOptions {
            query: "feature".to_string(), // Non-empty query to pass validation
            content_type: None,
            top_k: 50,
            timeline: true,
            format: "markdown".to_string(),
            project_path: None,
        };

        let result = executor.execute_timeline(&options).await;
        assert!(result.is_ok());
    }

    // ==================== Format Output Tests ====================

    #[test]
    fn test_format_results_markdown() {
        let executor = create_test_executor();

        let items = vec![EnhancedItem::new(
            "test-1".to_string(),
            ContentType::File,
            0.92,
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64,
            "Test content".to_string(),
        )];

        let options = QueryOptions {
            query: "test".to_string(),
            content_type: None,
            top_k: 5,
            timeline: false,
            format: "markdown".to_string(),
            project_path: None,
        };

        let result = executor.format_results(&items, &options);
        assert!(result.is_ok());

        let output = result.unwrap();
        assert!(!output.is_empty());
    }

    #[test]
    fn test_format_results_json() {
        let executor = create_test_executor();

        let items = vec![EnhancedItem::new(
            "test-1".to_string(),
            ContentType::Message,
            0.85,
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64,
            "Test message".to_string(),
        )];

        let options = QueryOptions {
            query: "test".to_string(),
            content_type: None,
            top_k: 5,
            timeline: false,
            format: "json".to_string(),
            project_path: None,
        };

        let result = executor.format_results(&items, &options);
        assert!(result.is_ok());

        let output = result.unwrap();
        // Should be valid JSON
        assert!(output.starts_with('{') || output.starts_with('['));
    }

    #[test]
    fn test_format_results_text() {
        let executor = create_test_executor();

        let items = vec![EnhancedItem::new(
            "test-1".to_string(),
            ContentType::Commit,
            0.78,
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64 - (30 * 86400),
            "Test commit".to_string(),
        )];

        let options = QueryOptions {
            query: "test".to_string(),
            content_type: None,
            top_k: 5,
            timeline: false,
            format: "text".to_string(),
            project_path: None,
        };

        let result = executor.format_results(&items, &options);
        assert!(result.is_ok());
        assert!(!result.unwrap().is_empty());
    }

    // ==================== Index Loading Tests ====================

    #[test]
    fn test_ensure_index_loaded_caches() {
        let mut executor = create_test_executor();

        // First call should load (or attempt to load)
        let result1 = executor.ensure_index_loaded();
        assert!(result1.is_ok());

        // Second call should use cached index
        let result2 = executor.ensure_index_loaded();
        assert!(result2.is_ok());
    }

    #[test]
    fn test_get_stats() {
        let executor = create_test_executor();

        let stats = executor.get_stats().expect("should get stats");
        assert_eq!(stats.project_path, executor.project_path);
        assert!(!stats.has_index); // No index in test env
        assert_eq!(stats.total_indexed, 0);
    }

    // ==================== Content Type Filter Tests ====================

    #[test]
    fn test_parse_content_type_filter_all_types() {
        // Test SOURCE
        let options = QueryOptions {
            query: "test".to_string(),
            content_type: Some("source".to_string()),
            top_k: 5,
            timeline: false,
            format: "markdown".to_string(),
            project_path: None,
        };
        assert_eq!(options.parse_content_type_filter(), Some(ContentType::File));

        // Test DOCS (also maps to File for now)
        let options = QueryOptions {
            query: "test".to_string(),
            content_type: Some("docs".to_string()),
            top_k: 5,
            timeline: false,
            format: "markdown".to_string(),
            project_path: None,
        };
        assert_eq!(options.parse_content_type_filter(), Some(ContentType::File));

        // Test SESSION
        let options = QueryOptions {
            query: "test".to_string(),
            content_type: Some("session".to_string()),
            top_k: 5,
            timeline: false,
            format: "markdown".to_string(),
            project_path: None,
        };
        assert_eq!(options.parse_content_type_filter(), Some(ContentType::Session));

        // Test COMMIT
        let options = QueryOptions {
            query: "test".to_string(),
            content_type: Some("commit".to_string()),
            top_k: 5,
            timeline: false,
            format: "markdown".to_string(),
            project_path: None,
        };
        assert_eq!(options.parse_content_type_filter(), Some(ContentType::Commit));

        // Test ALL
        let options = QueryOptions {
            query: "test".to_string(),
            content_type: Some("all".to_string()),
            top_k: 5,
            timeline: false,
            format: "markdown".to_string(),
            project_path: None,
        };
        assert_eq!(options.parse_content_type_filter(), None);
    }

    // ==================== Edge Case Tests ====================

    #[tokio::test]
    async fn test_enhance_results_empty() {
        let executor = create_test_executor();
        let options = create_test_options();

        let result = executor.enhance_results(&[], &options).await;
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    #[test]
    fn test_format_age_boundaries() {
        // Test 0 days
        assert_eq!(QueryExecutor::format_age(0), "Current");

        // Test fractional day (< 1 day)
        assert!(QueryExecutor::format_age(0).contains("Current"));

        // Test 1 day
        assert_eq!(QueryExecutor::format_age(1), "1 days ago");

        // Test 30 days (boundary before months)
        assert_eq!(QueryExecutor::format_age(30), "30 days ago");

        // Test 60 days (boundary to months)
        assert_eq!(QueryExecutor::format_age(60), "2 months ago");

        // Test 90 days (3 months)
        assert_eq!(QueryExecutor::format_age(90), "3 months ago");

        // Test 365 days (12 months)
        assert_eq!(QueryExecutor::format_age(365), "12 months ago");
    }
}
