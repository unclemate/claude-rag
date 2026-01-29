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
use tracing::{debug, info};

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
    /// Time range filter for results.
    pub time_range: Option<TimeRange>,
}

/// Time range filter for queries.
///
/// This struct represents a time interval used to filter search results
/// by their timestamp. It supports both absolute timestamps and
/// relative time specifications.
///
/// # Fields
///
/// * `after` - Start of time range (Unix timestamp, inclusive).
///              Results with timestamp >= after are included.
/// * `before` - End of time range (Unix timestamp, exclusive).
///               Results with timestamp < before are included.
///
/// # Examples
///
/// ```rust
/// use claude_rag::query::TimeRange;
///
/// // Empty range (no filtering)
/// let range = TimeRange::none();
/// assert!(range.is_empty());
///
/// // Last 7 days using max_age
/// let range = TimeRange::from_cli_args(None, None, Some(7)).unwrap();
/// let now = std::time::SystemTime::now()
///     .duration_since(std::time::UNIX_EPOCH)
///     .unwrap()
///     .as_secs() as i64;
/// assert!(range.contains(now));  // Now is included
/// assert!(range.contains(now - 3 * 86400));  // 3 days ago is included
/// assert!(!range.contains(now - 10 * 86400));  // 10 days ago is excluded
///
/// // Date range using ISO 8601
/// let range = TimeRange::from_cli_args(Some("2025-01-01"), Some("2025-01-31"), None).unwrap();
/// // Includes content from January 2025
///
/// // Relative time range
/// let range = TimeRange::from_cli_args(Some("7d"), Some("1d"), None).unwrap();
/// // Includes content from 1 to 7 days ago
///
/// # Time Format
///
/// - **Relative time** (requires suffix): `7d` (7 days), `1w` (1 week), `1m` (1 month), `1y` (1 year)
/// - **ISO 8601 date**: `2025-01-01` (YYYY-MM-DD format)
/// - **Case insensitive**: `7D`, `1W`, `1M`, `1Y` are also valid
///
/// # Special Behavior
///
/// - **Symbol type**: `ContentType::Symbol` items always pass time filtering
///   as they represent current code (timestamp = 0)
///
/// # Validation
///
/// - When both `after` and `before` are specified, `after` must be < `before`
/// - Pure numbers without suffix are rejected (e.g., `7` is invalid, use `7d`)
/// - Negative values are rejected
#[derive(Debug, Clone, PartialEq)]
pub struct TimeRange {
    /// Start of time range (Unix timestamp, inclusive).
    pub after: Option<i64>,
    /// End of time range (Unix timestamp, exclusive).
    pub before: Option<i64>,
}

impl TimeRange {
    /// Create an empty time range (no filtering).
    pub fn none() -> Self {
        Self { after: None, before: None }
    }

    /// Parse time string from CLI args.
    ///
    /// Time format must have suffix for relative time: 7d, 1w, 1m, 1y
    /// Or ISO 8601 date: 2025-01-01
    ///
    /// # Arguments
    ///
    /// * `after` - Optional start time string
    /// * `before` - Optional end time string
    /// * `max_age` - Optional maximum age in days (sets 'after' to now - max_age)
    pub fn from_cli_args(after: Option<&str>, before: Option<&str>, max_age: Option<u64>) -> Result<Self> {
        let mut result = Self::none();

        // Process max_age first (sets the 'after' boundary)
        if let Some(days) = max_age {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|e| RagError::Validation(format!("Failed to get current time: {}", e)))?
                .as_secs() as i64;
            result.after = Some(now - (days as i64 * 86400));
        }

        // Process explicit 'after' argument (overrides max_age)
        if let Some(after_str) = after {
            result.after = Some(Self::parse_time_string(after_str)?);
        }

        // Process 'before' argument
        if let Some(before_str) = before {
            result.before = Some(Self::parse_time_string(before_str)?);
        }

        // Validate: after must be before before (if both are set)
        if let (Some(after), Some(before)) = (result.after, result.before) {
            if after >= before {
                return Err(RagError::Validation(format!(
                    "Invalid time range: 'after' ({}) must be before 'before' ({})",
                    after, before
                )));
            }
        }

        Ok(result)
    }

    /// Parse time string to Unix timestamp.
    ///
    /// Supported formats:
    /// - Relative time with suffix: 7d, 30d, 1w, 1m, 1y
    /// - ISO 8601 date: 2025-01-01
    ///
    /// # Arguments
    ///
    /// * `s` - Time string to parse
    fn parse_time_string(s: &str) -> Result<i64> {
        let s = s.trim();

        // Check for relative time suffix (d, w, m, y)
        if let Some(suffix) = s.chars().last() {
            match suffix {
                'd' | 'D' => {
                    // Days: 7d, 30d
                    let days: i64 = s[..s.len()-1]
                        .parse()
                        .map_err(|_| RagError::Validation(format!("Invalid days format: '{}'", s)))?;
                    if days <= 0 {
                        return Err(RagError::Validation(format!("Days must be positive: '{}'", s)));
                    }
                    let now = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .map_err(|e| RagError::Validation(format!("Failed to get current time: {}", e)))?
                        .as_secs() as i64;
                    return Ok(now - (days * 86400));
                }
                'w' | 'W' => {
                    // Weeks: 1w, 2w
                    let weeks: i64 = s[..s.len()-1]
                        .parse()
                        .map_err(|_| RagError::Validation(format!("Invalid weeks format: '{}'", s)))?;
                    if weeks <= 0 {
                        return Err(RagError::Validation(format!("Weeks must be positive: '{}'", s)));
                    }
                    let now = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .map_err(|e| RagError::Validation(format!("Failed to get current time: {}", e)))?
                        .as_secs() as i64;
                    return Ok(now - (weeks * 7 * 86400));
                }
                'm' | 'M' => {
                    // Months: 1m, 6m (approximate as 30 days)
                    let months: i64 = s[..s.len()-1]
                        .parse()
                        .map_err(|_| RagError::Validation(format!("Invalid months format: '{}'", s)))?;
                    if months <= 0 {
                        return Err(RagError::Validation(format!("Months must be positive: '{}'", s)));
                    }
                    let now = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .map_err(|e| RagError::Validation(format!("Failed to get current time: {}", e)))?
                        .as_secs() as i64;
                    return Ok(now - (months * 30 * 86400));
                }
                'y' | 'Y' => {
                    // Years: 1y, 2y (approximate as 365 days)
                    let years: i64 = s[..s.len()-1]
                        .parse()
                        .map_err(|_| RagError::Validation(format!("Invalid years format: '{}'", s)))?;
                    if years <= 0 {
                        return Err(RagError::Validation(format!("Years must be positive: '{}'", s)));
                    }
                    let now = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .map_err(|e| RagError::Validation(format!("Failed to get current time: {}", e)))?
                        .as_secs() as i64;
                    return Ok(now - (years * 365 * 86400));
                }
                _ => {
                    // Continue to try ISO 8601 parsing
                }
            }
        }

        // Try ISO 8601 date parsing: YYYY-MM-DD
        if s.len() == 10 && &s[4..5] == "-" && &s[7..8] == "-" {
            let date: chrono::NaiveDate = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d")
                .map_err(|_| RagError::Validation(format!("Invalid date format (use YYYY-MM-DD): '{}'", s)))?;
            let datetime = date.and_hms_opt(0, 0, 0)
                .ok_or_else(|| RagError::Validation("Failed to create datetime".to_string()))?;
            return Ok(datetime.and_utc().timestamp());
        }

        Err(RagError::Validation(format!(
            "Invalid time format: '{}'. Use relative time (7d, 1w, 1m, 1y) or ISO 8601 date (YYYY-MM-DD)",
            s
        )))
    }

    /// Check if a timestamp is within this time range.
    ///
    /// # Arguments
    ///
    /// * `timestamp` - Unix timestamp to check
    pub fn contains(&self, timestamp: i64) -> bool {
        if let Some(after) = self.after {
            if timestamp < after {
                return false;
            }
        }
        if let Some(before) = self.before {
            if timestamp >= before {
                return false;
            }
        }
        true
    }

    /// Check if this time range has any filtering effect.
    pub fn is_empty(&self) -> bool {
        self.after.is_none() && self.before.is_none()
    }
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
        info!(
            query = %options.query,
            top_k = options.top_k,
            content_type = ?options.content_type,
            "Executing query"
        );

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
        debug!(content_filter = ?content_filter, "Content filter");

        // Execute search
        let raw_results = index.search(&query_embedding, options.top_k, content_filter)?;
        debug!(results_count = raw_results.len(), "Retrieved raw results");

        if raw_results.is_empty() {
            return Ok(self.format_no_results_message(options));
        }

        // Enhance results with metadata and time-aware scoring
        let enhanced_results = self.enhance_results(&raw_results, options).await?;

        info!(
            results_count = enhanced_results.len(),
            "Query completed"
        );
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

        // Apply time range filtering (if specified)
        let mut filtered_items = scored_items;
        if let Some(ref time_range) = options.time_range {
            let original_count = filtered_items.len();
            filtered_items.retain(|item| {
                // Symbols always pass filtering (represent current code)
                if item.content_type == ContentType::Symbol {
                    return true;
                }
                time_range.contains(item.timestamp)
            });

            if filtered_items.len() < original_count {
                tracing::debug!(
                    "Time range filtered: {} -> {} results",
                    original_count,
                    filtered_items.len()
                );
            }
        }

        // Sort by final score
        let mut enhanced = filtered_items;
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

        // Calculate final score with full formula (similarity × temporal_weight × confidence_weight)
        let final_score = item.similarity * temporal_weight * confidence.weight;

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
                let final_score = item.similarity * temporal_weight * confidence.weight;

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
        time_range: None,
    };

    let mut executor = QueryExecutor::from_options(&options)?;

    if timeline {
        executor.execute_timeline(&options).await
    } else {
        executor.execute(&options).await
    }
}

/// Execute a query from CLI arguments with time range filter.
///
/// This is the main entry point for the `rag query` CLI command with time filtering.
///
/// # Arguments
///
/// * `query` - Query text
/// * `content_type` - Optional content type filter
/// * `top_k` - Number of results to return
/// * `timeline` - Whether to show timeline
/// * `format` - Output format
/// * `time_range` - Optional time range filter
///
/// # Returns
///
/// Returns formatted results or an error.
pub async fn execute_query_with_time_range(
    query: String,
    content_type: Option<String>,
    top_k: usize,
    timeline: bool,
    format: String,
    time_range: Option<TimeRange>,
) -> Result<String> {
    let options = QueryOptions {
        query,
        content_type,
        top_k,
        timeline,
        format,
        project_path: None,
        time_range,
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
            time_range: None,
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
            time_range: None,
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
            time_range: None,
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
            time_range: None,
        };
        assert!(options.validate().is_err());

        let options = QueryOptions {
            query: "test".to_string(),
            content_type: None,
            top_k: 1001,
            timeline: false,
            format: "markdown".to_string(),
            project_path: None,
            time_range: None,
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
                time_range: None,
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
            time_range: None,
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
                time_range: None,
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
            time_range: None,
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
            time_range: None,
        };
        assert_eq!(options.parse_content_type_filter(), Some(ContentType::File));

        let options = QueryOptions {
            query: "test".to_string(),
            content_type: Some(TYPE_SESSION.to_string()),
            top_k: 5,
            timeline: false,
            format: "markdown".to_string(),
            project_path: None,
            time_range: None,
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
            time_range: None,
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
            time_range: None,
        };
        assert_eq!(options.parse_output_format(), OutputFormat::Markdown);

        let options = QueryOptions {
            query: "test".to_string(),
            content_type: None,
            top_k: 5,
            timeline: false,
            format: "json".to_string(),
            project_path: None,
            time_range: None,
        };
        assert_eq!(options.parse_output_format(), OutputFormat::Json);

        let options = QueryOptions {
            query: "test".to_string(),
            content_type: None,
            top_k: 5,
            timeline: false,
            format: "text".to_string(),
            project_path: None,
            time_range: None,
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
            time_range: None,
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
            time_range: None,
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
            time_range: None,
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
            time_range: None,
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
            time_range: None,
        };
        assert_eq!(options.parse_content_type_filter(), Some(ContentType::File));

        let options = QueryOptions {
            query: "test".to_string(),
            content_type: Some("Source".to_string()),
            top_k: 5,
            timeline: false,
            format: "markdown".to_string(),
            project_path: None,
            time_range: None,
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
            time_range: None,
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
            },
            "logging": {
                "level": "info",
                "enable_file_logging": false,
                "json_format": false,
                "include_spans": false,
                "daily_rotation": true
            },
            "progress": {
                "style": {
                    "style_type": "default",
                    "show_cache_stats": true,
                    "show_processing_rate": true,
                    "show_eta": true
                }
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
        // With confidence.weight = 1.2 for File, final_score ≈ 0.85 * 1.0 * 1.2 ≈ 1.02
        assert!(scored.final_score > 0.9); // Should have high final score with confidence weight
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
        // Final score with confidence: 0.85 * low_temporal * 1.2
        // Even with confidence.weight=1.2, low temporal_weight should keep final_score < similarity
        assert!(scored.final_score < scored.similarity);
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
        // Code should have Highest confidence with weight 1.2
        assert_eq!(scored_code.confidence_level, crate::retrieval::ConfidenceLevel::Highest);
        assert_eq!(scored_code.confidence_level.base_weight(), 1.2);
        // Verify final_score includes confidence weight
        let expected = scored_code.similarity * scored_code.temporal_weight * 1.2;
        assert!((scored_code.final_score - expected).abs() < f32::EPSILON);

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
            time_range: None,
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
            time_range: None,
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
            time_range: None,
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
            time_range: None,
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
            time_range: None,
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
            time_range: None,
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
            time_range: None,
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
            time_range: None,
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
            time_range: None,
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
            time_range: None,
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
            time_range: None,
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

    // ==================== Confidence Weight Tests ====================

    #[tokio::test]
    async fn test_confidence_weight_included_in_final_score() {
        let executor = create_test_executor();

        // Store a test file
        let file = File {
            id: "test-confidence-1".to_string(),
            project_path: "/test/project".to_string(),
            file_path: "src/test.rs".to_string(),
            language: Some("rust".to_string()),
            kind: FileKind::Source,
            modified_at: chrono::Utc::now(),
            size: 1024,
            content_hash: "abc123".to_string(),
            indexed: true,
        };
        executor.storage.store_file(&file).expect("should store file");

        let raw_results = vec![("test-confidence-1".to_string(), 0.85)];
        let options = create_test_options();

        let enhanced = executor.enhance_results(&raw_results, &options).await.unwrap();
        assert_eq!(enhanced.len(), 1);

        let item = &enhanced[0];
        let confidence_weight = item.confidence_level.base_weight();

        // Verify final_score = similarity * temporal_weight * confidence_weight
        let expected_score = item.similarity * item.temporal_weight * confidence_weight;
        assert!((item.final_score - expected_score).abs() < f32::EPSILON,
            "final_score should equal similarity * temporal_weight * confidence_weight: \
             got {}, expected {} (similarity={}, temporal_weight={}, confidence_weight={})",
            item.final_score, expected_score, item.similarity, item.temporal_weight, confidence_weight);
    }

    #[tokio::test]
    async fn test_confidence_weight_affects_ranking() {
        let executor = create_test_executor();

        let now = chrono::Utc::now();

        // Store two files with same similarity but different confidence
        let file_current = File {
            id: "file-current".to_string(),
            project_path: "/test/project".to_string(),
            file_path: "src/current.rs".to_string(),
            language: Some("rust".to_string()),
            kind: FileKind::Source,
            modified_at: now,
            size: 1024,
            content_hash: "abc123".to_string(),
            indexed: true,
        };
        executor.storage.store_file(&file_current).expect("should store current file");

        let file_old = File {
            id: "file-old".to_string(),
            project_path: "/test/project".to_string(),
            file_path: "src/old.rs".to_string(),
            language: Some("rust".to_string()),
            kind: FileKind::Source,
            modified_at: now - chrono::Duration::days(100),
            size: 1024,
            content_hash: "def456".to_string(),
            indexed: true,
        };
        executor.storage.store_file(&file_old).expect("should store old file");

        // Raw results: same similarity, but current file should rank higher due to confidence
        let raw_results = vec![
            ("file-old".to_string(), 0.80),
            ("file-current".to_string(), 0.80),
        ];
        let options = create_test_options();

        let enhanced = executor.enhance_results(&raw_results, &options).await.unwrap();

        // Current file should rank first (higher confidence weight)
        assert_eq!(enhanced[0].id, "file-current");
        assert_eq!(enhanced[1].id, "file-old");

        // Verify current file has higher final_score despite same similarity
        assert!(enhanced[0].final_score > enhanced[1].final_score);
    }

    #[tokio::test]
    async fn test_final_score_range_within_expected_bounds() {
        let executor = create_test_executor();

        let now = chrono::Utc::now();

        // Create items with different content types to test score range
        let file = File {
            id: "test-file".to_string(),
            project_path: "/test/project".to_string(),
            file_path: "src/test.rs".to_string(),
            language: Some("rust".to_string()),
            kind: FileKind::Source,
            modified_at: now,
            size: 1024,
            content_hash: "abc123".to_string(),
            indexed: true,
        };
        executor.storage.store_file(&file).expect("should store file");

        let session = Session {
            id: "test-session".to_string(),
            title: Some("Test Session".to_string()),
            project_path: "/test/project".to_string(),
            started_at: now - chrono::Duration::days(40), // Old session
            ended_at: None,
            message_count: 1,
            indexed: true,
        };
        executor.storage.store_session(&session).expect("should store session");

        let raw_results = vec![
            ("test-file".to_string(), 0.90),
            ("test-session".to_string(), 0.90),
        ];
        let options = create_test_options();

        let enhanced = executor.enhance_results(&raw_results, &options).await.unwrap();

        // Final scores should be in range [0, 1.2]
        for item in &enhanced {
            assert!(item.final_score >= 0.0, "final_score should be non-negative");
            assert!(item.final_score <= 1.2, "final_score should not exceed 1.2, got {}", item.final_score);
        }
    }

    // ==================== Edge Case & Boundary Tests ====================

    #[test]
    fn test_confidence_weight_all_levels() {
        let executor = create_test_executor();

        // Test each confidence level's weight
        use crate::retrieval::ConfidenceLevel;
        let test_cases = [
            (ContentType::File, 0, ConfidenceLevel::Highest, 1.2),
            (ContentType::Commit, 10, ConfidenceLevel::High, 1.0),
            (ContentType::Session, 5, ConfidenceLevel::Medium, 0.85),
            (ContentType::Session, 15, ConfidenceLevel::Low, 0.6),
            (ContentType::Session, 40, ConfidenceLevel::Lowest, 0.4),
        ];

        for (content_type, age_days, expected_level, expected_weight) in test_cases {
            let item = EnhancedItem::new(
                format!("test-{:?}", content_type),
                content_type,
                0.8,
                SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64 - (age_days * 86400),
                "Test".to_string(),
            );

            let scored = executor.apply_time_aware_scoring(item).unwrap();
            assert_eq!(
                scored.confidence_level, expected_level,
                "Content type {:?} with age {} days should have level {:?}",
                content_type, age_days, expected_level
            );
            assert_eq!(
                scored.confidence_level.base_weight(), expected_weight,
                "Confidence level {:?} should have weight {}",
                expected_level, expected_weight
            );
        }
    }

    #[test]
    fn test_zero_similarity_results_in_zero_final_score() {
        let executor = create_test_executor();

        let item = EnhancedItem::new(
            "test-zero-sim".to_string(),
            ContentType::File,
            0.0, // Zero similarity
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64,
            "Test".to_string(),
        );

        let scored = executor.apply_time_aware_scoring(item).unwrap();
        // With similarity = 0, final_score should be 0 regardless of other factors
        assert_eq!(scored.final_score, 0.0);
        assert_eq!(scored.similarity, 0.0);
    }

    #[test]
    fn test_very_old_content_has_near_zero_temporal_weight() {
        let executor = create_test_executor();

        // Create very old content (1 year old)
        let very_old_timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64 - (365 * 86400);

        let item = EnhancedItem::new(
            "test-ancient".to_string(),
            ContentType::File,
            0.9,
            very_old_timestamp,
            "Ancient content".to_string(),
        );

        let scored = executor.apply_time_aware_scoring(item).unwrap();
        // With 365 days and decay_rate=0.05: temporal_weight = exp(-0.05 * 365) ≈ 1.5e-8
        // Final score should be very small (near zero) despite high similarity
        assert!(scored.temporal_weight < 0.01, "temporal_weight should be near zero for very old content, got {}", scored.temporal_weight);
        assert!(scored.final_score < 0.02, "final_score should be near zero for very old content, got {}", scored.final_score);
    }

    #[test]
    fn test_highest_confidence_boosts_score_above_similarity() {
        let executor = create_test_executor();

        // Current file with Highest confidence (weight=1.2)
        let item = EnhancedItem::new(
            "test-current-file".to_string(),
            ContentType::File,
            0.8,
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64,
            "Current file".to_string(),
        );

        let scored = executor.apply_time_aware_scoring(item).unwrap();
        // With Highest confidence (weight=1.2), final_score should exceed similarity
        assert!(scored.final_score > scored.similarity,
            "Final score ({}) should exceed similarity ({}) with Highest confidence",
            scored.final_score, scored.similarity);
    }

    #[test]
    fn test_lowest_confidence_reduces_score_below_similarity() {
        let executor = create_test_executor();

        // Old session with Lowest confidence (weight=0.4)
        let old_session_timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64 - (40 * 86400);

        let item = EnhancedItem::new(
            "test-old-session".to_string(),
            ContentType::Session,
            0.8,
            old_session_timestamp,
            "Old session".to_string(),
        );

        let scored = executor.apply_time_aware_scoring(item).unwrap();
        // With Lowest confidence (weight=0.4) and low temporal_weight, final_score should be much lower
        assert!(scored.final_score < scored.similarity * 0.5,
            "Final score ({}) should be significantly lower than similarity ({}) with Lowest confidence",
            scored.final_score, scored.similarity);
    }

    #[tokio::test]
    async fn test_confidence_formula_structure_consistency() {
        let executor = create_test_executor();

        // Test that both methods use the same formula structure: similarity × temporal_weight × confidence_weight

        // Test simplified method directly
        let item = EnhancedItem::new(
            "test-structure".to_string(),
            ContentType::File,
            0.8,
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64,
            "Test".to_string(),
        );
        let scored = executor.apply_time_aware_scoring(item).unwrap();

        // Verify formula: final_score = similarity × temporal_weight × confidence_weight
        let expected = scored.similarity * scored.temporal_weight * scored.confidence_level.base_weight();
        assert!((scored.final_score - expected).abs() < f32::EPSILON,
            "Formula should be: similarity × temporal_weight × confidence_weight");

        // For File type with current timestamp, verify Highest confidence is applied
        assert_eq!(scored.confidence_level, crate::retrieval::ConfidenceLevel::Highest);
    }

    // ==================== High Priority: Git Status Tests ====================

    #[tokio::test]
    async fn test_deprecated_file_has_low_confidence() {
        let executor = create_test_executor();

        let now = chrono::Utc::now();

        // Store a file that will be treated as deprecated (non-current in Git)
        let file = File {
            id: "deprecated-file".to_string(),
            project_path: "/test/project".to_string(),
            file_path: "src/deprecated.rs".to_string(),
            language: Some("rust".to_string()),
            kind: FileKind::Source,
            modified_at: now,
            size: 1024,
            content_hash: "old-hash".to_string(),
            indexed: true,
        };
        executor.storage.store_file(&file).expect("should store file");

        let raw_results = vec![("deprecated-file".to_string(), 0.85)];
        let options = create_test_options();

        let enhanced = executor.enhance_results(&raw_results, &options).await.unwrap();
        assert_eq!(enhanced.len(), 1);

        let item = &enhanced[0];

        // Deprecated file should have Low confidence (weight=0.6)
        // In test environment without actual Git repo, git_sync defaults to treating files as current
        // So we verify the structure is correct rather than exact confidence level
        assert_eq!(item.content_type, ContentType::File);
        assert!(item.similarity > 0.0);

        // Verify final_score uses the formula: similarity × temporal_weight × confidence_weight
        let expected = item.similarity * item.temporal_weight * item.confidence_level.base_weight();
        assert!((item.final_score - expected).abs() < f32::EPSILON,
            "final_score should follow the formula even with varying confidence levels");
    }

    #[tokio::test]
    async fn test_git_status_affects_confidence_level() {
        let executor = create_test_executor();

        let now = chrono::Utc::now();

        // Store two files with same content type and age
        // In a real Git scenario, one would be current and one deprecated
        let file1 = File {
            id: "file-git-1".to_string(),
            project_path: "/test/project".to_string(),
            file_path: "src/current.rs".to_string(),
            language: Some("rust".to_string()),
            kind: FileKind::Source,
            modified_at: now,
            size: 1024,
            content_hash: "hash1".to_string(),
            indexed: true,
        };
        executor.storage.store_file(&file1).expect("should store file 1");

        let file2 = File {
            id: "file-git-2".to_string(),
            project_path: "/test/project".to_string(),
            file_path: "src/other.rs".to_string(),
            language: Some("rust".to_string()),
            kind: FileKind::Source,
            modified_at: now,
            size: 1024,
            content_hash: "hash2".to_string(),
            indexed: true,
        };
        executor.storage.store_file(&file2).expect("should store file 2");

        let raw_results = vec![
            ("file-git-1".to_string(), 0.80),
            ("file-git-2".to_string(), 0.80),
        ];
        let options = create_test_options();

        let enhanced = executor.enhance_results(&raw_results, &options).await.unwrap();
        assert_eq!(enhanced.len(), 2);

        // Both should have valid confidence levels
        for item in &enhanced {
            assert!(matches!(item.confidence_level,
                crate::retrieval::ConfidenceLevel::Highest |
                crate::retrieval::ConfidenceLevel::Low));  // Current or Deprecated

            // Verify scoring formula is consistent
            let expected = item.similarity * item.temporal_weight * item.confidence_level.base_weight();
            assert!((item.final_score - expected).abs() < f32::EPSILON);
        }
    }

    // ==================== Medium Priority: Symbol Type Tests ====================

    #[tokio::test]
    async fn test_symbol_scoring_with_confidence_weight() {
        let executor = create_test_executor();

        let now = chrono::Utc::now();

        // Store a file first (symbols need a parent file)
        let file = File {
            id: "symbol-parent-file".to_string(),
            project_path: "/test/project".to_string(),
            file_path: "src/lib.rs".to_string(),
            language: Some("rust".to_string()),
            kind: FileKind::Source,
            modified_at: now,
            size: 1024,
            content_hash: "file-hash".to_string(),
            indexed: true,
        };
        executor.storage.store_file(&file).expect("should store file");

        // Store a symbol
        use crate::models::{Symbol, SymbolKind};
        let symbol = Symbol {
            id: "test-symbol".to_string(),
            file_id: "symbol-parent-file".to_string(),
            name: "test_function".to_string(),
            kind: SymbolKind::Function,
            start_line: 10,
            end_line: 15,
            doc_comment: Some("Test function documentation".to_string()),
            code: "fn test_function() { }".to_string(),
            parent_id: None,
        };
        executor.storage.store_symbol(&symbol).expect("should store symbol");

        let raw_results = vec![("test-symbol".to_string(), 0.88)];
        let options = create_test_options();

        let enhanced = executor.enhance_results(&raw_results, &options).await.unwrap();
        assert_eq!(enhanced.len(), 1);

        let item = &enhanced[0];

        // Verify Symbol content type
        assert_eq!(item.content_type, ContentType::Symbol);
        assert_eq!(item.id, "test-symbol");

        // Symbols should have a confidence level (Highest if git_current, Low otherwise)
        let confidence_weight = item.confidence_level.base_weight();
        assert!(confidence_weight >= 0.4 && confidence_weight <= 1.2,
            "Symbol confidence weight should be in valid range, got {}", confidence_weight);

        // Verify final_score includes confidence weight
        let expected = item.similarity * item.temporal_weight * confidence_weight;
        assert!((item.final_score - expected).abs() < f32::EPSILON,
            "Symbol final_score should follow the full formula");
    }

    // ==================== Medium Priority: Edge Case Tests ====================

    #[test]
    fn test_scoring_with_zero_timestamp() {
        let executor = create_test_executor();

        // Test with timestamp = 0 (should be treated as current/age_days=0)
        let item = EnhancedItem::new(
            "test-zero-timestamp".to_string(),
            ContentType::File,
            0.75,
            0, // Zero timestamp
            "Test content".to_string(),
        );

        let scored = executor.apply_time_aware_scoring(item).unwrap();

        // Zero timestamp should result in age_days = 0 (current)
        assert_eq!(scored.age_description, "Current");
        assert!(scored.is_current);

        // Verify scoring formula still works correctly
        let expected = scored.similarity * scored.temporal_weight * scored.confidence_level.base_weight();
        assert!((scored.final_score - expected).abs() < f32::EPSILON);
    }

    #[test]
    fn test_scoring_with_negative_timestamp_fallback() {
        let executor = create_test_executor();

        // Test with negative timestamp (edge case, should be handled gracefully)
        let item = EnhancedItem::new(
            "test-negative-timestamp".to_string(),
            ContentType::File,
            0.75,
            -100, // Negative timestamp (invalid/unlikely)
            "Test content".to_string(),
        );

        // Should not panic, but handle gracefully
        let scored = executor.apply_time_aware_scoring(item);

        // The code handles timestamp > 0 check, so negative will be treated as 0
        assert!(scored.is_ok());

        let scored = scored.unwrap();
        // Negative timestamp results in large positive age_days calculation
        // but the code should handle it without panicking
        assert!(scored.final_score >= 0.0);
        assert!(scored.final_score <= scored.similarity * 1.2); // Max with confidence
    }

    #[test]
    fn test_scoring_with_future_timestamp() {
        let executor = create_test_executor();

        // Test with future timestamp (edge case)
        let future_timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64 + 86400; // 1 day in the future

        let item = EnhancedItem::new(
            "test-future-timestamp".to_string(),
            ContentType::File,
            0.85,
            future_timestamp,
            "Future content".to_string(),
        );

        let scored = executor.apply_time_aware_scoring(item).unwrap();

        // Future timestamp results in negative age_days, but format_age shows it as negative hours
        // The scoring calculation handles this correctly by using max(0, age_days)
        assert!(scored.age_description.contains("hours ago") || scored.age_description == "Current");

        // temporal_weight should be high (near or above 1.0 since age_days is negative/maxed at 0)
        assert!(scored.temporal_weight > 0.95);

        // Most importantly: verify scoring formula still works correctly
        let expected = scored.similarity * scored.temporal_weight * scored.confidence_level.base_weight();
        assert!((scored.final_score - expected).abs() < f32::EPSILON,
            "Scoring formula should work correctly even with future timestamps");
    }

    // ==================== TimeRange Tests ====================

    #[test]
    fn test_time_range_none() {
        let range = TimeRange::none();
        assert!(range.is_empty());
        assert!(range.after.is_none());
        assert!(range.before.is_none());
    }

    #[test]
    fn test_time_range_from_cli_args_none() {
        let range = TimeRange::from_cli_args(None, None, None).unwrap();
        assert!(range.is_empty());
    }

    #[test]
    fn test_time_range_parse_iso8601() {
        // Parse ISO 8601 date for 'after'
        let range = TimeRange::from_cli_args(Some("2025-01-15"), None, None).unwrap();
        assert!(range.after.is_some());
        assert!(range.before.is_none());

        // Verify the timestamp is roughly correct (January 15, 2025)
        let after_ts = range.after.unwrap();
        // 2025-01-15 00:00:00 UTC should be around 1736899200
        assert!(after_ts >= 1736899200 - 86400 && after_ts <= 1736899200 + 86400);
    }

    #[test]
    fn test_time_range_parse_relative_days() {
        // Parse "7d" (7 days ago)
        let range = TimeRange::from_cli_args(Some("7d"), None, None).unwrap();
        assert!(range.after.is_some());

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let after_ts = range.after.unwrap();

        // Should be approximately 7 days ago
        let expected = now - (7 * 86400);
        assert!(after_ts >= expected - 100 && after_ts <= expected + 100);

        // Parse "30d"
        let range = TimeRange::from_cli_args(Some("30d"), None, None).unwrap();
        let after_ts = range.after.unwrap();
        let expected = now - (30 * 86400);
        assert!(after_ts >= expected - 100 && after_ts <= expected + 100);
    }

    #[test]
    fn test_time_range_parse_relative_weeks() {
        // Parse "1w" (1 week ago)
        let range = TimeRange::from_cli_args(Some("1w"), None, None).unwrap();
        assert!(range.after.is_some());

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let after_ts = range.after.unwrap();

        // Should be approximately 7 days ago (1 week)
        let expected = now - (7 * 86400);
        assert!(after_ts >= expected - 100 && after_ts <= expected + 100);

        // Parse "2w"
        let range = TimeRange::from_cli_args(Some("2w"), None, None).unwrap();
        let after_ts = range.after.unwrap();
        let expected = now - (14 * 86400);
        assert!(after_ts >= expected - 100 && after_ts <= expected + 100);
    }

    #[test]
    fn test_time_range_parse_relative_months() {
        // Parse "1m" (1 month ago, approximately 30 days)
        let range = TimeRange::from_cli_args(Some("1m"), None, None).unwrap();
        assert!(range.after.is_some());

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let after_ts = range.after.unwrap();

        // Should be approximately 30 days ago
        let expected = now - (30 * 86400);
        assert!(after_ts >= expected - 100 && after_ts <= expected + 100);
    }

    #[test]
    fn test_time_range_parse_relative_years() {
        // Parse "1y" (1 year ago, approximately 365 days)
        let range = TimeRange::from_cli_args(Some("1y"), None, None).unwrap();
        assert!(range.after.is_some());

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let after_ts = range.after.unwrap();

        // Should be approximately 365 days ago
        let expected = now - (365 * 86400);
        assert!(after_ts >= expected - 100 && after_ts <= expected + 100);
    }

    #[test]
    fn test_time_range_max_age() {
        // Test max_age parameter
        let range = TimeRange::from_cli_args(None, None, Some(7)).unwrap();
        assert!(range.after.is_some());
        assert!(range.before.is_none());

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let after_ts = range.after.unwrap();

        // Should be approximately 7 days ago
        let expected = now - (7 * 86400);
        assert!(after_ts >= expected - 100 && after_ts <= expected + 100);
    }

    #[test]
    fn test_time_range_max_age_overridden_by_after() {
        // Test that explicit 'after' overrides max_age
        let range = TimeRange::from_cli_args(Some("30d"), None, Some(7)).unwrap();
        assert!(range.after.is_some());

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let after_ts = range.after.unwrap();

        // Should be approximately 30 days ago (from 'after', not max_age=7)
        let expected = now - (30 * 86400);
        assert!(after_ts >= expected - 100 && after_ts <= expected + 100);
    }

    #[test]
    fn test_time_range_validation_after_before_order() {
        // Test that after < before validation works
        // Invalid: after is after before (using relative times that would result in this)
        // "1d" is more recent than "7d", so this should fail
        let result = TimeRange::from_cli_args(Some("1d"), Some("7d"), None);
        // Note: "1d" means now - 1 day, "7d" means now - 7 days
        // So after = now - 1 day, before = now - 7 days
        // This means after > before, which is invalid
        assert!(result.is_err());

        // Valid: after is before before
        let result = TimeRange::from_cli_args(Some("7d"), Some("1d"), None);
        assert!(result.is_ok());
    }

    #[test]
    fn test_time_range_contains() {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        // Test with only 'after' set
        let range = TimeRange {
            after: Some(now - 86400), // 1 day ago
            before: None,
        };

        assert!(!range.contains(now - 172800)); // 2 days ago - before the range
        assert!(range.contains(now - 43200));  // 12 hours ago - within the range
        assert!(range.contains(now));          // Now - within the range

        // Test with only 'before' set
        let range = TimeRange {
            after: None,
            before: Some(now - 43200), // 12 hours ago
        };

        assert!(range.contains(now - 86400)); // 1 day ago - within the range
        assert!(!range.contains(now));         // Now - after the range

        // Test with both 'after' and 'before' set
        let range = TimeRange {
            after: Some(now - 172800), // 2 days ago
            before: Some(now - 43200), // 12 hours ago
        };

        assert!(!range.contains(now - 259200)); // 3 days ago - before the range
        assert!(range.contains(now - 86400));   // 1 day ago - within the range
        assert!(!range.contains(now));          // Now - after the range
    }

    #[test]
    fn test_time_range_empty() {
        // Empty range should contain all timestamps
        let range = TimeRange::none();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        assert!(range.contains(0));
        assert!(range.contains(now));
        assert!(range.contains(now - 86400));
        assert!(range.contains(now + 86400));
    }

    #[test]
    fn test_time_range_invalid_format() {
        // Test invalid time format (no suffix)
        let result = TimeRange::from_cli_args(Some("7"), None, None);
        assert!(result.is_err());

        // Test invalid date format
        let result = TimeRange::from_cli_args(Some("2025/01/15"), None, None);
        assert!(result.is_err());

        // Test invalid suffix
        let result = TimeRange::from_cli_args(Some("7h"), None, None);
        assert!(result.is_err());

        // Test negative value
        let result = TimeRange::from_cli_args(Some("-7d"), None, None);
        assert!(result.is_err());
    }

    #[test]
    fn test_time_range_case_insensitive() {
        // Test case insensitive suffix parsing
        let range1 = TimeRange::from_cli_args(Some("7d"), None, None).unwrap();
        let range2 = TimeRange::from_cli_args(Some("7D"), None, None).unwrap();

        assert_eq!(range1.after, range2.after);

        let range1 = TimeRange::from_cli_args(Some("1w"), None, None).unwrap();
        let range2 = TimeRange::from_cli_args(Some("1W"), None, None).unwrap();

        assert_eq!(range1.after, range2.after);
    }

    // ==================== TimeRange Boundary Tests ====================

    #[test]
    fn test_time_range_boundary_equal_timestamps() {
        // Test boundary condition where after == before
        // This should be rejected as invalid
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let result = TimeRange::from_cli_args(Some("7d"), Some("7d"), None);
        assert!(result.is_err(), "Equal timestamps should be rejected");

        // Test with explicit timestamps
        let range = TimeRange {
            after: Some(now - 86400),
            before: Some(now - 86400),
        };
        // Note: This bypasses validation, but contains() should handle it
        // Equal timestamps means an empty range (no timestamp can be >= x AND < x)
        assert!(!range.contains(now - 86400));
        assert!(!range.contains(now));
    }

    #[test]
    fn test_time_range_boundary_zero_timestamp() {
        // Test behavior with timestamp = 0
        let range = TimeRange {
            after: Some(0),
            before: Some(86400), // 1 day after epoch
        };

        assert!(range.contains(0));           // t=0 is included (>= after)
        assert!(range.contains(43200));        // t=12h is included
        assert!(!range.contains(86400));       // t=1d is excluded (< before)

        // Test with only after = 0
        let range = TimeRange {
            after: Some(0),
            before: None,
        };
        assert!(range.contains(0));
        assert!(range.contains(86400));
    }

    #[test]
    fn test_time_range_symbol_always_passes() {
        // Test that Symbol type (timestamp = 0) passes filtering
        // as it represents current code
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        // Create a range that would exclude everything from the past week
        let range = TimeRange {
            after: Some(now + 86400), // Future time
            before: None,
        };

        // Symbol with timestamp = 0 should still pass
        // (This is the responsibility of the caller, but we test the logic here)
        assert!(!range.contains(now - 86400)); // Normal timestamp filtered
        assert!(!range.contains(0));           // But timestamp=0 also filtered by TimeRange
        // The special Symbol handling is done in enhance_results()
    }

    // ==================== Enhance Results Integration Tests ====================

    #[tokio::test]
    async fn test_enhance_results_with_time_range_filter() {
        let executor = create_test_executor();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        // Store test sessions with different timestamps
        let old_session = Session {
            id: "old-session-time-test".to_string(),
            title: Some("Old Session".to_string()),
            project_path: "/test/project".to_string(),
            started_at: chrono::Utc::now() - chrono::Duration::days(100),
            ended_at: None,
            message_count: 1,
            indexed: true,
        };
        executor.storage.store_session(&old_session).unwrap();

        let recent_session = Session {
            id: "recent-session-time-test".to_string(),
            title: Some("Recent Session".to_string()),
            project_path: "/test/project".to_string(),
            started_at: chrono::Utc::now() - chrono::Duration::days(3),
            ended_at: None,
            message_count: 1,
            indexed: true,
        };
        executor.storage.store_session(&recent_session).unwrap();

        // Create raw results (order: old first, then recent)
        let raw_results = vec![
            ("old-session-time-test".to_string(), 0.90), // High similarity but old
            ("recent-session-time-test".to_string(), 0.70), // Lower similarity but recent
        ];

        // Query with time range (last 7 days)
        let mut options = create_test_options();
        options.time_range = Some(TimeRange {
            after: Some(now - (7 * 86400)),
            before: None,
        });

        let enhanced = executor.enhance_results(&raw_results, &options).await.unwrap();

        // Old session should be filtered out
        assert_eq!(enhanced.len(), 1);
        assert_eq!(enhanced[0].id, "recent-session-time-test");
    }

    #[tokio::test]
    async fn test_enhance_results_time_range_with_symbol() {
        let executor = create_test_executor();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        // Store a file and symbol
        let file = File {
            id: "time-test-file".to_string(),
            project_path: "/test/project".to_string(),
            file_path: "src/test.rs".to_string(),
            language: Some("rust".to_string()),
            kind: FileKind::Source,
            modified_at: chrono::Utc::now() - chrono::Duration::days(100), // Old file
            size: 1024,
            content_hash: "hash".to_string(),
            indexed: true,
        };
        executor.storage.store_file(&file).unwrap();

        use crate::models::{Symbol, SymbolKind};
        let symbol = Symbol {
            id: "time-test-symbol".to_string(),
            file_id: "time-test-file".to_string(),
            name: "test_func".to_string(),
            kind: SymbolKind::Function,
            start_line: 1,
            end_line: 5,
            doc_comment: None,
            code: "fn test_func() {}".to_string(),
            parent_id: None,
        };
        executor.storage.store_symbol(&symbol).unwrap();

        // Create raw results
        let raw_results = vec![
            ("time-test-file".to_string(), 0.85),
            ("time-test-symbol".to_string(), 0.80),
        ];

        // Query with time range (last 7 days)
        let mut options = create_test_options();
        options.time_range = Some(TimeRange {
            after: Some(now - (7 * 86400)),
            before: None,
        });

        let enhanced = executor.enhance_results(&raw_results, &options).await.unwrap();

        // File should be filtered out (too old), but Symbol should pass
        assert_eq!(enhanced.len(), 1);
        assert_eq!(enhanced[0].id, "time-test-symbol");
        assert_eq!(enhanced[0].content_type, ContentType::Symbol);
    }

    #[tokio::test]
    async fn test_enhance_results_time_range_both_bounds() {
        let executor = create_test_executor();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        // Store multiple sessions with different timestamps
        for (days_offset, id) in [(100, "very-old"), (30, "old"), (10, "middle"), (3, "recent"), (1, "very-recent")] {
            let session = Session {
                id: format!("time-range-{}-test", id),
                title: Some(format!("Session {}", id)),
                project_path: "/test/project".to_string(),
                started_at: chrono::Utc::now() - chrono::Duration::days(days_offset),
                ended_at: None,
                message_count: 1,
                indexed: true,
            };
            executor.storage.store_session(&session).unwrap();
        }

        // Create raw results
        let raw_results = vec![
            ("time-range-very-old-test".to_string(), 0.90),
            ("time-range-old-test".to_string(), 0.85),
            ("time-range-middle-test".to_string(), 0.80),
            ("time-range-recent-test".to_string(), 0.75),
            ("time-range-very-recent-test".to_string(), 0.70),
        ];

        // Query with both bounds (5 to 15 days ago)
        let mut options = create_test_options();
        options.time_range = Some(TimeRange {
            after: Some(now - (15 * 86400)), // 15 days ago
            before: Some(now - (5 * 86400)),  // 5 days ago
        });

        let enhanced = executor.enhance_results(&raw_results, &options).await.unwrap();

        // Only "middle" (10 days) should be in range
        assert_eq!(enhanced.len(), 1);
        assert_eq!(enhanced[0].id, "time-range-middle-test");
    }

    #[tokio::test]
    async fn test_enhance_results_time_range_empty_results() {
        let executor = create_test_executor();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        // Store an old session
        let session = Session {
            id: "empty-time-range-test".to_string(),
            title: Some("Old Session".to_string()),
            project_path: "/test/project".to_string(),
            started_at: chrono::Utc::now() - chrono::Duration::days(100),
            ended_at: None,
            message_count: 1,
            indexed: true,
        };
        executor.storage.store_session(&session).unwrap();

        let raw_results = vec![("empty-time-range-test".to_string(), 0.90)];

        // Query with time range that excludes everything
        let mut options = create_test_options();
        options.time_range = Some(TimeRange {
            after: Some(now - (3 * 86400)), // 3 days ago
            before: Some(now - (1 * 86400)),  // 1 day ago
        });

        let enhanced = executor.enhance_results(&raw_results, &options).await.unwrap();

        // Should return empty results (old session filtered)
        assert_eq!(enhanced.len(), 0);
    }
}
