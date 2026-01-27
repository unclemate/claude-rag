//! Git history collection.

use crate::error::Result;

/// Git collector.
pub struct GitCollector;

impl GitCollector {
    /// Create a new git collector.
    pub fn new() -> Self {
        Self
    }
}

impl Default for GitCollector {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_collector_new() {
        let _collector = GitCollector::new();
    }
}
