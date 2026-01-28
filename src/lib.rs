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
pub mod mcp;
pub mod results;
pub mod parser;
pub mod retrieval;
pub mod scanner;
pub mod skills;
pub mod storage;
pub mod vector;

pub mod models;
pub mod query;

pub use ast::{AstParser, SupportedLanguage};
pub use config::EmbeddingConfig;
pub use error::{Result, RagError};
pub use parser::{ParsedSession, SessionParser};
pub use query::{execute_query, QueryExecutor, QueryOptions, QueryStats};
pub use results::{EnhancedItem, GitInfo, SupersededInfo};
