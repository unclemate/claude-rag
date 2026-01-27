//! File scanning with .gitignore support.
//!
//! This module provides file scanning capabilities that respect .gitignore,
//! classify files by type (source/doc/other), and detect changes for incremental
//! indexing.

use crate::error::{RagError, Result};
use crate::models::File;
use chrono::{DateTime, Utc};
use ignore::{gitignore::Gitignore, WalkBuilder};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// File classification type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileType {
    /// Source code files
    Source,
    /// Documentation files
    Documentation,
    /// Other files
    Other,
}

/// File scan result with metadata.
#[derive(Debug, Clone)]
pub struct ScannedFile {
    /// Absolute path to the file
    pub path: PathBuf,
    /// File type classification
    pub file_type: FileType,
    /// File modification time
    pub modified_at: DateTime<Utc>,
    /// File size in bytes
    pub size: u64,
    /// SHA-256 hash of file content
    pub content_hash: String,
    /// Detected language (for source files)
    pub language: Option<String>,
}

/// File scanner with .gitignore support.
pub struct FileScanner {
    /// Project root directory
    project_root: PathBuf,
    /// Gitignore matcher
    gitignore: Option<Gitignore>,
    /// File type patterns
    source_extensions: Vec<String>,
    doc_extensions: Vec<String>,
}

impl FileScanner {
    /// Create a new file scanner for a project.
    ///
    /// # Arguments
    /// * `project_path` - Path to the project root
    pub fn new(project_path: &Path) -> Result<Self> {
        let project_root = fs::canonicalize(project_path)
            .map_err(RagError::Io)?;

        // Load .gitignore if present
        let gitignore_path = project_root.join(".gitignore");
        let gitignore = if gitignore_path.exists() {
            let (gitignore, _err) = Gitignore::new(&gitignore_path);
            Some(gitignore)
        } else {
            None
        };

        // Define source file extensions
        let source_extensions = vec![
            "rs".to_string(),
            "py".to_string(),
            "js".to_string(),
            "ts".to_string(),
            "jsx".to_string(),
            "tsx".to_string(),
            "go".to_string(),
            "java".to_string(),
            "c".to_string(),
            "cpp".to_string(),
            "cc".to_string(),
            "h".to_string(),
            "hpp".to_string(),
            "cs".to_string(),
            "rb".to_string(),
            "php".to_string(),
            "swift".to_string(),
            "kt".to_string(),
            "scala".to_string(),
            "sh".to_string(),
            "bash".to_string(),
            "zsh".to_string(),
            "fish".to_string(),
            "sql".to_string(),
            "json".to_string(),
            "yaml".to_string(),
            "yml".to_string(),
            "toml".to_string(),
            "xml".to_string(),
        ];

        // Define documentation file extensions
        let doc_extensions = vec![
            "md".to_string(),
            "markdown".to_string(),
            "rst".to_string(),
            "txt".to_string(),
            "adoc".to_string(),
            "asciidoc".to_string(),
        ];

        Ok(Self {
            project_root,
            gitignore,
            source_extensions,
            doc_extensions,
        })
    }

    /// Scan the project directory for files.
    ///
    /// # Arguments
    /// * `index_source` - Whether to include source files
    /// * `index_docs` - Whether to include documentation files
    /// * `index_other` - Whether to include other files
    ///
    /// # Returns
    /// * `Vec<ScannedFile>` - List of scanned files
    pub fn scan(
        &self,
        index_source: bool,
        index_docs: bool,
        index_other: bool,
    ) -> Result<Vec<ScannedFile>> {
        let mut files = Vec::new();

        // Use WalkBuilder for efficient directory traversal
        let walker = WalkBuilder::new(&self.project_root)
            .hidden(false)
            .parents(false)
            .git_ignore(false) // We handle .gitignore ourselves
            .build();

        for entry in walker {
            let entry = entry.map_err(|e| RagError::Parse(format!("Walk error: {e}")))?;
            let path = entry.path();

            // Skip directories
            if path.is_dir() {
                continue;
            }

            // Check .gitignore
            if let Some(gi) = &self.gitignore {
                if gi.matched(path, false).is_ignore() {
                    continue;
                }
            }

            // Get relative path from project root
            let relative_path = path
                .strip_prefix(&self.project_root)
                .map_err(|e| RagError::Parse(format!("Invalid path: {e}")))?;

            // Skip files in .rag directory
            if relative_path.starts_with(".rag") {
                continue;
            }

            // Skip common non-project directories
            if let Some(first) = relative_path.iter().next() {
                let first = first.to_string_lossy();
                if matches!(first.as_ref(), "node_modules" | "target" | "vendor" | ".git" | "dist" | "build") {
                    continue;
                }
            }

            // Get file metadata
            let metadata = fs::metadata(path).map_err(RagError::Io)?;
            let modified_at = metadata
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| DateTime::from_timestamp(d.as_secs() as i64, 0).unwrap_or_else(Utc::now))
                .unwrap_or_else(Utc::now);

            let size = metadata.len();

            // Calculate content hash
            let content_hash = self.hash_file(path)?;

            // Classify file type
            let file_type = self.classify_file(path);

            // Skip based on index settings
            match file_type {
                FileType::Source if !index_source => continue,
                FileType::Documentation if !index_docs => continue,
                FileType::Other if !index_other => continue,
                _ => {}
            }

            // Detect language for source files
            let language = if file_type == FileType::Source {
                self.detect_language(path)
            } else {
                None
            };

            files.push(ScannedFile {
                path: path.to_path_buf(),
                file_type,
                modified_at,
                size,
                content_hash,
                language,
            });
        }

        Ok(files)
    }

    /// Classify a file by its extension.
    ///
    /// # Arguments
    /// * `path` - Path to the file
    ///
    /// # Returns
    /// * `FileType` - The classified file type
    pub fn classify_file(&self, path: &Path) -> FileType {
        path.extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| {
                let ext_lower = ext.to_lowercase();
                if self.source_extensions.contains(&ext_lower) {
                    FileType::Source
                } else if self.doc_extensions.contains(&ext_lower) {
                    FileType::Documentation
                } else {
                    FileType::Other
                }
            })
            .unwrap_or(FileType::Other)
    }

    /// Detect the programming language of a file.
    ///
    /// # Arguments
    /// * `path` - Path to the file
    ///
    /// # Returns
    /// * `Option<String>` - Detected language name
    pub fn detect_language(&self, path: &Path) -> Option<String> {
        path.extension()
            .and_then(|ext| ext.to_str())
            .and_then(|ext| match ext.to_lowercase().as_str() {
                "rs" => Some("rust"),
                "py" => Some("python"),
                "js" => Some("javascript"),
                "jsx" => Some("javascript"),
                "ts" => Some("typescript"),
                "tsx" => Some("typescript"),
                "go" => Some("go"),
                "java" => Some("java"),
                "c" => Some("c"),
                "cpp" | "cc" => Some("cpp"),
                "h" => Some("c"),
                "hpp" => Some("cpp"),
                "cs" => Some("csharp"),
                "rb" => Some("ruby"),
                "php" => Some("php"),
                "swift" => Some("swift"),
                "kt" => Some("kotlin"),
                "scala" => Some("scala"),
                "sh" | "bash" => Some("bash"),
                "zsh" => Some("zsh"),
                "fish" => Some("fish"),
                "sql" => Some("sql"),
                "json" => Some("json"),
                "yaml" | "yml" => Some("yaml"),
                "toml" => Some("toml"),
                "xml" => Some("xml"),
                _ => None,
            })
            .map(|s| s.to_string())
    }

    /// Calculate SHA-256 hash of a file.
    ///
    /// # Arguments
    /// * `path` - Path to the file
    ///
    /// # Returns
    /// * `String` - Hex-encoded SHA-256 hash
    fn hash_file(&self, path: &Path) -> Result<String> {
        let contents = fs::read(path).map_err(RagError::Io)?;
        let mut hasher = Sha256::new();
        hasher.update(&contents);
        let hash = hasher.finalize();
        Ok(format!("{:x}", hash))
    }

    /// Scan for changed files based on previous index state.
    ///
    /// # Arguments
    /// * `indexed_files` - Map of file paths to their (hash, modified_at)
    /// * `index_source` - Whether to include source files
    /// * `index_docs` - Whether to include documentation files
    /// * `index_other` - Whether to include other files
    ///
    /// # Returns
    /// * `(Vec<ScannedFile>, Vec<String>)` - (New/changed files, deleted files)
    pub fn scan_incremental(
        &self,
        indexed_files: &HashMap<String, (String, DateTime<Utc>)>,
        index_source: bool,
        index_docs: bool,
        index_other: bool,
    ) -> Result<(Vec<ScannedFile>, Vec<String>)> {
        let current_files = self.scan(index_source, index_docs, index_other)?;

        let mut changed_files = Vec::new();
        let mut deleted_files = Vec::new();

        // Check for new or modified files
        for file in &current_files {
            let relative_path = file
                .path
                .strip_prefix(&self.project_root)
                .map_err(|e| RagError::Parse(format!("Invalid path: {e}")))?
                .to_string_lossy()
                .to_string();

            if let Some((prev_hash, prev_modified)) = indexed_files.get(&relative_path) {
                // File exists - check if changed
                if &file.content_hash != prev_hash || &file.modified_at != prev_modified {
                    changed_files.push(file.clone());
                }
            } else {
                // New file
                changed_files.push(file.clone());
            }
        }

        // Check for deleted files
        for path in indexed_files.keys() {
            let exists = current_files.iter().any(|f| {
                f.path
                    .strip_prefix(&self.project_root)
                    .map(|p| p.to_string_lossy() == path.as_str())
                    .unwrap_or(false)
            });

            if !exists {
                deleted_files.push(path.clone());
            }
        }

        Ok((changed_files, deleted_files))
    }

    /// Convert a scanned file to a File model.
    ///
    /// # Arguments
    /// * `scanned` - The scanned file
    ///
    /// # Returns
    /// * `File` - The file model
    pub fn to_file_model(&self, scanned: &ScannedFile) -> File {
        let relative_path = scanned
            .path
            .strip_prefix(&self.project_root)
            .unwrap_or(&scanned.path)
            .to_string_lossy()
            .to_string();

        let id = format!("file:{:x}", Sha256::digest(relative_path.as_bytes()));

        File {
            id,
            project_path: self.project_root.to_string_lossy().to_string(),
            file_path: relative_path,
            language: scanned.language.clone(),
            modified_at: scanned.modified_at,
            size: scanned.size,
            content_hash: scanned.content_hash.clone(),
            indexed: false,
        }
    }
}

impl Default for FileScanner {
    fn default() -> Self {
        Self::new(Path::new(".")).unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File as StdFile;
    use std::io::Write;
    use tempfile::TempDir;

    fn create_test_file(dir: &Path, name: &str, content: &str) -> PathBuf {
        let path = dir.join(name);
        let mut file = StdFile::create(&path).unwrap();
        file.write_all(content.as_bytes()).unwrap();
        path
    }

    #[test]
    fn test_scanner_new() {
        let temp = TempDir::new().unwrap();
        let scanner = FileScanner::new(temp.path()).unwrap();
        assert_eq!(scanner.project_root, temp.path());
    }

    #[test]
    fn test_scanner_default() {
        let temp = TempDir::new().unwrap();
        let scanner = FileScanner::new(temp.path()).unwrap();
        assert_eq!(scanner.project_root, temp.path());
    }

    #[test]
    fn test_classify_source_files() {
        let scanner = FileScanner::default();

        assert_eq!(
            scanner.classify_file(Path::new("test.rs")),
            FileType::Source
        );
        assert_eq!(
            scanner.classify_file(Path::new("test.py")),
            FileType::Source
        );
        assert_eq!(
            scanner.classify_file(Path::new("test.ts")),
            FileType::Source
        );
    }

    #[test]
    fn test_classify_doc_files() {
        let scanner = FileScanner::default();

        assert_eq!(
            scanner.classify_file(Path::new("README.md")),
            FileType::Documentation
        );
        assert_eq!(
            scanner.classify_file(Path::new("doc.txt")),
            FileType::Documentation
        );
    }

    #[test]
    fn test_classify_other_files() {
        let scanner = FileScanner::default();

        assert_eq!(
            scanner.classify_file(Path::new("image.png")),
            FileType::Other
        );
        assert_eq!(
            scanner.classify_file(Path::new("data.bin")),
            FileType::Other
        );
    }

    #[test]
    fn test_detect_language() {
        let scanner = FileScanner::default();

        assert_eq!(scanner.detect_language(Path::new("test.rs")), Some("rust".to_string()));
        assert_eq!(scanner.detect_language(Path::new("test.py")), Some("python".to_string()));
        assert_eq!(scanner.detect_language(Path::new("test.ts")), Some("typescript".to_string()));
        assert_eq!(scanner.detect_language(Path::new("test.md")), None);
    }

    #[test]
    fn test_hash_file() {
        let temp = TempDir::new().unwrap();
        let path = create_test_file(temp.path(), "test.txt", "Hello, World!");

        let scanner = FileScanner::new(temp.path()).unwrap();
        let hash = scanner.hash_file(&path).unwrap();

        assert_eq!(hash.len(), 64); // SHA-256 produces 64 hex characters
        assert_ne!(hash, "d41d8cd98f00b204e9800998ecf8427e"); // Not empty hash
    }

    #[test]
    fn test_scan_empty_directory() {
        let temp = TempDir::new().unwrap();
        let scanner = FileScanner::new(temp.path()).unwrap();

        let files = scanner.scan(true, true, true).unwrap();
        assert_eq!(files.len(), 0);
    }

    #[test]
    fn test_scan_with_files() {
        let temp = TempDir::new().unwrap();
        create_test_file(temp.path(), "main.rs", "fn main() {}");
        create_test_file(temp.path(), "README.md", "# Test");
        create_test_file(temp.path(), "data.txt", "data");

        let scanner = FileScanner::new(temp.path()).unwrap();
        let files = scanner.scan(true, true, true).unwrap();

        assert_eq!(files.len(), 3);
        assert!(files.iter().any(|f| f.file_type == FileType::Source));
        assert!(files.iter().any(|f| f.file_type == FileType::Documentation));
    }

    #[test]
    fn test_scan_source_only() {
        let temp = TempDir::new().unwrap();
        create_test_file(temp.path(), "main.rs", "fn main() {}");
        create_test_file(temp.path(), "README.md", "# Test");

        let scanner = FileScanner::new(temp.path()).unwrap();
        let files = scanner.scan(true, false, false).unwrap();

        assert_eq!(files.len(), 1);
        assert_eq!(files[0].file_type, FileType::Source);
    }

    #[test]
    fn test_to_file_model() {
        let temp = TempDir::new().unwrap();
        create_test_file(temp.path(), "main.rs", "fn main() {}");

        let scanner = FileScanner::new(temp.path()).unwrap();
        let files = scanner.scan(true, false, false).unwrap();
        let file_model = scanner.to_file_model(&files[0]);

        assert_eq!(file_model.file_path, "main.rs");
        assert_eq!(file_model.language, Some("rust".to_string()));
        assert!(!file_model.indexed);
    }

    #[test]
    fn test_scan_incremental() {
        let temp = TempDir::new().unwrap();
        let _path = create_test_file(temp.path(), "main.rs", "fn main() {}");

        let scanner = FileScanner::new(temp.path()).unwrap();
        let files = scanner.scan(true, false, false).unwrap();

        let mut indexed = HashMap::new();
        indexed.insert(
            "main.rs".to_string(),
            (files[0].content_hash.clone(), files[0].modified_at),
        );

        // No changes
        let (changed, deleted) = scanner
            .scan_incremental(&indexed, true, false, false)
            .unwrap();
        assert_eq!(changed.len(), 0);
        assert_eq!(deleted.len(), 0);

        // Modify file
        create_test_file(temp.path(), "main.rs", "fn new() {}");
        let (changed, deleted) = scanner
            .scan_incremental(&indexed, true, false, false)
            .unwrap();
        assert_eq!(changed.len(), 1);
        assert_eq!(deleted.len(), 0);
    }
}
