//! File collection and scanning.

/// File collector.
pub struct FileCollector;

impl FileCollector {
    /// Create a new file collector.
    pub fn new() -> Self {
        Self
    }
}

impl Default for FileCollector {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_collector_new() {
        let _collector = FileCollector::new();
    }
}
