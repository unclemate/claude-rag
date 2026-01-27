//! Session JSONL parsing.

use crate::error::Result;

/// Session parser.
pub struct SessionParser;

impl SessionParser {
    /// Create a new parser.
    pub fn new() -> Self {
        Self
    }

    /// Parse a JSONL session file.
    pub fn parse_jsonl(&self, _path: &std::path::Path) -> Result<Vec<serde_json::Value>> {
        // TODO: Implement JSONL parsing
        Ok(Vec::new())
    }
}

impl Default for SessionParser {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parser_new() {
        let _parser = SessionParser::new();
    }
}
