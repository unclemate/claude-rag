//! Centralized error types for Claude RAG.
//!
//! Uses `thiserror` for structured error definitions and `anyhow` for application context.

use std::path::PathBuf;
use thiserror::Error;

/// Centralized error type for the application.
#[derive(Error, Debug)]
pub enum RagError {
    /// I/O operations failed.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Sled database operation failed.
    #[error("Database error: {0}")]
    Sled(#[from] sled::Error),

    /// Git operation failed.
    #[error("Git error: {0}")]
    Git(String),

    /// Embedding API error.
    #[error("Embedding API error: {0}")]
    Embedding(String),

    /// Configuration error.
    #[error("Configuration error: {0}")]
    Config(String),

    /// Resource not found.
    #[error("Not found: {0}")]
    NotFound(String),

    /// JSON parsing error.
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    /// TOML parsing error.
    #[error("TOML error: {0}")]
    Toml(#[from] toml::de::Error),

    /// HTTP request error.
    #[error("HTTP error: {0}")]
    Http(String),

    /// Parse error.
    #[error("Parse error: {0}")]
    Parse(String),

    /// Validation error.
    #[error("Validation error: {0}")]
    Validation(String),
}

/// Result type alias for RagError.
pub type Result<T> = std::result::Result<T, RagError>;

impl From<git2::Error> for RagError {
    fn from(err: git2::Error) -> Self {
        RagError::Git(err.to_string())
    }
}

impl From<reqwest::Error> for RagError {
    fn from(err: reqwest::Error) -> Self {
        RagError::Http(err.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let err = RagError::NotFound("test".to_string());
        assert_eq!(err.to_string(), "Not found: test");
    }

    #[test]
    fn test_error_from_io() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
        let rag_err: RagError = io_err.into();
        assert!(matches!(rag_err, RagError::Io(_)));
    }
}
