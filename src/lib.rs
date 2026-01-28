//! # Claude RAG
//!
//! A time-aware RAG knowledge base for Claude Code projects.
//!
//! This library provides:
//! - Session record indexing from Claude Code history
//! - Source file indexing with hierarchical granularity
//! - Git history integration for temporal context
//! - Time-aware retrieval with confidence scoring

#![warn(missing_docs)]
#![warn(clippy::all)]

pub mod ast;
pub mod cli;
pub mod collector;
pub mod config;
pub mod daemon;
pub mod document;
pub mod embedding;
pub mod error;
pub mod formatter;
pub mod hook;
pub mod indexer;
pub mod logging;
pub mod mcp;
pub mod results;
pub mod parser;
pub mod retrieval;
pub mod scanner;
pub mod skills;
pub mod storage;
pub mod vector;

pub mod models;
pub mod progress;
pub mod query;

pub use ast::{AstParser, SupportedLanguage};
pub use config::{Config, ConfigManager, EmbeddingConfig};
pub use error::{Result, RagError};
pub use logging::{init_logging, init_logging_default, LoggingOptions, LogLevel, create_dummy_guard};
pub use parser::{ParsedSession, SessionParser};
pub use progress::{CallbackReporter, ProgressBarReporter, ProgressEvent, ProgressReporter, ProgressReporterExt, ProgressStats, ProgressStyle, ProgressStyleType};
pub use query::{
    execute_query, execute_query_with_time_range, QueryExecutor, QueryOptions, QueryStats,
};

// Re-export TimeRange at the crate root for convenience
pub use query::TimeRange;
pub use results::{EnhancedItem, GitInfo, SupersededInfo};
