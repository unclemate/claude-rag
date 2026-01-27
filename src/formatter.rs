//! Result formatting for query output.

use crate::error::Result;

/// Result formatter.
pub struct ResultFormatter;

impl ResultFormatter {
    /// Create a new formatter.
    pub fn new() -> Self {
        Self
    }

    /// Format results as markdown.
    pub fn format_markdown(&self, _results: &[QueryResult]) -> Result<String> {
        // TODO: Implement markdown formatting
        Ok(String::new())
    }

    /// Format results as JSON.
    pub fn format_json(&self, _results: &[QueryResult]) -> Result<String> {
        // TODO: Implement JSON formatting
        Ok(String::new())
    }
}

impl Default for ResultFormatter {
    fn default() -> Self {
        Self::new()
    }
}

/// A query result.
#[derive(Debug, Clone)]
pub struct QueryResult {
    /// Result ID.
    pub id: String,
    /// Content type.
    pub content_type: crate::models::ContentType,
    /// Similarity score.
    pub similarity: f32,
    /// Temporal weight.
    pub temporal_weight: f32,
    /// Final score.
    pub final_score: f32,
    /// Content snippet.
    pub content: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_formatter_new() {
        let _formatter = ResultFormatter::new();
    }
}
