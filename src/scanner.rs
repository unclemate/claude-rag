//! File scanning with .gitignore support.

use crate::error::Result;

/// File scanner.
pub struct FileScanner;

impl FileScanner {
    /// Create a new file scanner.
    pub fn new() -> Self {
        Self
    }

    /// Scan a project directory.
    pub fn scan(&self, _project_path: &std::path::Path) -> Result<Vec<std::path::PathBuf>> {
        // TODO: Implement file scanning
        Ok(Vec::new())
    }
}

impl Default for FileScanner {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scanner_new() {
        let _scanner = FileScanner::new();
    }
}
