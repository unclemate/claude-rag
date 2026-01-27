//! Session collection from Claude Code history.

use crate::error::Result;

/// Session collector.
pub struct SessionCollector;

impl SessionCollector {
    /// Create a new session collector.
    pub fn new() -> Self {
        Self
    }

    /// Scan for Claude projects.
    pub fn scan_projects(&self) -> Result<Vec<String>> {
        // TODO: Implement scanning
        Ok(Vec::new())
    }
}

impl Default for SessionCollector {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_collector_new() {
        let collector = SessionCollector::new();
        let projects = collector.scan_projects();
        assert!(projects.is_ok());
    }
}
