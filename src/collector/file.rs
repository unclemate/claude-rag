//! File collection and indexing.

use crate::document::DocumentParser;
use crate::error::{RagError, Result};
use crate::models::File;
use crate::progress::ProgressReporter;
use crate::scanner::FileScanner;
use crate::storage::sled::StorageManager;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use tracing::{debug, info, trace, warn};

/// File chunk for indexing.
#[derive(Debug, Clone)]
pub struct FileChunk {
    /// Chunk content
    pub content: String,
    /// Line range (start, end)
    pub line_range: (usize, usize),
    /// Chunk identifier
    pub chunk_id: String,
}

/// Collection statistics.
#[derive(Debug, Clone, Default)]
pub struct CollectionStats {
    /// Number of files scanned
    pub files_scanned: usize,
    /// Number of files collected
    pub files_collected: usize,
    /// Number of chunks created
    pub chunks_created: usize,
    /// Number of errors
    pub errors: usize,
}

/// File collector for scanning and processing project files.
pub struct FileCollector {
    /// Project root path
    project_path: PathBuf,
    /// File scanner
    scanner: FileScanner,
    /// Chunk size in lines
    chunk_size: usize,
    /// Chunk overlap in lines
    chunk_overlap: usize,
}

impl FileCollector {
    /// Create a new file collector.
    ///
    /// # Arguments
    /// * `project_path` - Path to the project root
    pub fn new(project_path: &Path) -> Result<Self> {
        debug!(
            "Creating FileCollector for project: {}",
            project_path.display()
        );
        let scanner = FileScanner::new(project_path)?;

        Ok(Self {
            project_path: project_path.to_path_buf(),
            scanner,
            chunk_size: 100,
            chunk_overlap: 10,
        })
    }

    /// Set chunk size for file content splitting.
    pub fn with_chunk_size(mut self, chunk_size: usize) -> Self {
        self.chunk_size = chunk_size;
        self
    }

    /// Set chunk overlap for file content splitting.
    pub fn with_chunk_overlap(mut self, chunk_overlap: usize) -> Self {
        self.chunk_overlap = chunk_overlap;
        self
    }

    /// Collect files from the project.
    ///
    /// # Arguments
    /// * `index_source` - Whether to include source files
    /// * `index_docs` - Whether to include documentation files
    /// * `index_other` - Whether to include other files
    ///
    /// # Returns
    /// * `Vec<File>` - List of collected files
    pub fn collect_files(
        &self,
        index_source: bool,
        index_docs: bool,
        index_other: bool,
    ) -> Result<Vec<File>> {
        info!(
            index_source = index_source,
            index_docs = index_docs,
            index_other = index_other,
            "Collecting files from project"
        );
        let scanned_files = self.scanner.scan(index_source, index_docs, index_other)?;

        let files: Vec<File> = scanned_files
            .into_iter()
            .map(|scanned| self.scanner.to_file_model(&scanned))
            .collect();

        info!(
            files_count = files.len(),
            "Collected files from project"
        );
        Ok(files)
    }

    /// Collect files incrementally based on previous index state.
    ///
    /// # Arguments
    /// * `storage` - Storage manager for checking index state
    /// * `index_source` - Whether to include source files
    /// * `index_docs` - Whether to include documentation files
    /// * `index_other` - Whether to include other files
    ///
    /// # Returns
    /// * `(Vec<File>, Vec<String>)` - (New/changed files, deleted file paths)
    pub fn collect_incremental(
        &self,
        storage: &StorageManager,
        index_source: bool,
        index_docs: bool,
        index_other: bool,
    ) -> Result<(Vec<File>, Vec<String>)> {
        debug!(
            index_source = index_source,
            index_docs = index_docs,
            index_other = index_other,
            "Collecting incremental file changes"
        );

        // Build map of indexed files using relative paths as keys
        let project_files = storage.get_project_files(&self.project_path.to_string_lossy())?;
        let mut indexed_files = HashMap::new();

        for file_id in project_files {
            if let Some(file) = storage.get_file(&file_id)? {
                // Use relative path as key to match scanner's expectation
                indexed_files.insert(file.file_path.clone(), (file.content_hash, file.modified_at));
            }
        }

        debug!(
            indexed_files_count = indexed_files.len(),
            "Found indexed files in storage"
        );

        // Scan for changes
        let (changed_scanned, deleted_paths) = self
            .scanner
            .scan_incremental(&indexed_files, index_source, index_docs, index_other)?;

        let changed_files: Vec<File> = changed_scanned
            .into_iter()
            .map(|scanned| self.scanner.to_file_model(&scanned))
            .collect();

        info!(
            changed_files_count = changed_files.len(),
            deleted_files_count = deleted_paths.len(),
            "Incremental collection complete"
        );

        Ok((changed_files, deleted_paths))
    }

    /// Read and collect file content.
    ///
    /// # Arguments
    /// * `file_path` - Absolute path to the file
    ///
    /// # Returns
    /// * `String` - File content
    pub fn collect_file_content(&self, file_path: &Path) -> Result<String> {
        trace!("Reading file content: {}", file_path.display());
        fs::read_to_string(file_path).map_err(RagError::Io)
    }

    /// Split file content into chunks.
    ///
    /// For document files (.md, .rst, .adoc, .txt), uses DocumentParser for
    /// structured chunking. For source files, uses line-based chunking.
    ///
    /// # Arguments
    /// * `content` - File content
    /// * `file_path` - File path for chunk ID generation
    ///
    /// # Returns
    /// * `Vec<FileChunk>` - List of chunks
    pub fn chunk_file_content(&self, content: &str, file_path: &Path) -> Vec<FileChunk> {
        // Early exit for empty content
        if content.is_empty() {
            trace!("Skipping empty file: {}", file_path.display());
            return Vec::new();
        }

        trace!(
            "Chunking file content: {} ({} bytes, {} lines)",
            file_path.display(),
            content.len(),
            content.lines().count()
        );

        // Try document parsing for supported formats
        match DocumentParser::from_path(file_path) {
            Ok(parser) => self.chunk_with_parser(content, file_path, parser),
            Err(_) => self.chunk_source_file(content, file_path),
        }
    }

    /// Chunk content using a specific DocumentParser instance.
    fn chunk_with_parser(&self, content: &str, file_path: &Path, parser: DocumentParser) -> Vec<FileChunk> {
        let file_id = self.compute_file_id(file_path);

        match parser.parse(content, file_path, &file_id) {
            Ok(doc_chunks) => {
                trace!(
                    "Document parser generated {} chunks for {}",
                    doc_chunks.len(),
                    file_path.display()
                );
                // Convert DocChunk to FileChunk
                doc_chunks
                    .into_iter()
                    .map(|dc| FileChunk {
                        content: dc.content,
                        line_range: (dc.start_line, dc.end_line),
                        chunk_id: dc.id,
                    })
                    .collect()
            }
            Err(e) => {
                warn!(
                    file_path = %file_path.display(),
                    error = %e,
                    "Document parsing failed, falling back to line-based chunking"
                );
                self.chunk_source_file(content, file_path)
            }
        }
    }

    /// Chunk source files using line-based approach (existing logic).
    fn chunk_source_file(&self, content: &str, file_path: &Path) -> Vec<FileChunk> {
        let lines: Vec<&str> = content.lines().collect();

        if lines.is_empty() {
            return Vec::new();
        }

        trace!(
            "Line-based chunking for {}: {} lines, chunk_size={}, overlap={}",
            file_path.display(),
            lines.len(),
            self.chunk_size,
            self.chunk_overlap
        );

        let mut chunks = Vec::new();
        let mut start = 0;

        while start < lines.len() {
            let end = (start + self.chunk_size).min(lines.len());
            let chunk_content = lines[start..end].join("\n");

            // Generate chunk ID
            let chunk_id = self.generate_chunk_id(file_path, start, end);

            chunks.push(FileChunk {
                content: chunk_content,
                line_range: (start + 1, end), // 1-indexed for display
                chunk_id,
            });

            // Move to next chunk with overlap
            if end == lines.len() {
                break;
            }
            start = end.saturating_sub(self.chunk_overlap);

            // Prevent infinite loop
            if start <= start.saturating_sub(self.chunk_overlap) && end < lines.len() {
                start = end;
            }
        }

        trace!("Created {} chunks for {}", chunks.len(), file_path.display());
        chunks
    }

    /// Compute file ID for document parsing.
    fn compute_file_id(&self, file_path: &Path) -> String {
        let path_str = file_path.to_string_lossy();
        format!("file:{:x}", Sha256::digest(path_str.as_bytes()))
    }

    /// Generate a unique chunk ID.
    fn generate_chunk_id(&self, file_path: &Path, start: usize, end: usize) -> String {
        let path_str = file_path.to_string_lossy();
        let input = format!("{}:{}:{}", path_str, start, end);
        format!("chunk:{:x}", Sha256::digest(input.as_bytes()))
    }

    /// Store collected files to storage.
    ///
    /// # Arguments
    /// * `files` - Files to store
    /// * `storage` - Storage manager
    ///
    /// # Returns
    /// * `CollectionStats` - Collection statistics
    pub fn store_files(&self, files: &[File], storage: &StorageManager) -> Result<CollectionStats> {
        info!(files_count = files.len(), "Storing files to storage");
        let mut stats = CollectionStats {
            files_scanned: files.len(),
            ..Default::default()
        };

        for file in files {
            match storage.store_file(file) {
                Ok(_) => {
                    stats.files_collected += 1;
                }
                Err(e) => {
                    warn!(
                        file_path = %file.file_path,
                        error = %e,
                        "Error storing file"
                    );
                    stats.errors += 1;
                }
            }
        }

        info!(
            files_collected = stats.files_collected,
            errors_count = stats.errors,
            "Storage complete"
        );

        Ok(stats)
    }

    /// Store collected files to storage with progress reporting.
    ///
    /// # Arguments
    /// * `files` - Files to store
    /// * `storage` - Storage manager
    /// * `reporter` - Progress reporter
    ///
    /// # Returns
    /// * `CollectionStats` - Collection statistics
    pub fn store_files_with_progress(
        &self,
        files: &[File],
        storage: &StorageManager,
        reporter: &dyn ProgressReporter,
    ) -> Result<CollectionStats> {
        use crate::progress::ProgressEvent;

        let mut stats = CollectionStats {
            files_scanned: files.len(),
            ..Default::default()
        };

        // Report phase started
        reporter.report(ProgressEvent::PhaseStarted {
            name: "files".to_string(),
            total: files.len(),
        });

        for (i, file) in files.iter().enumerate() {
            // Report progress
            reporter.report(ProgressEvent::ItemProgress {
                current: i + 1,
                total: files.len(),
                name: file.file_path.clone(),
            });

            match storage.store_file(file) {
                Ok(_) => {
                    stats.files_collected += 1;
                    reporter.report(ProgressEvent::ItemCompleted {
                        name: file.file_path.clone(),
                        success: true,
                    });
                }
                Err(e) => {
                    stats.errors += 1;
                    warn!(
                        file_path = %file.file_path,
                        error = %e,
                        "Error storing file"
                    );
                    reporter.report(ProgressEvent::ItemCompleted {
                        name: file.file_path.clone(),
                        success: false,
                    });
                    reporter.report(ProgressEvent::Error {
                        message: format!("Failed to store {}: {}", file.file_path, e),
                    });
                }
            }
        }

        // Report phase completed
        reporter.report(ProgressEvent::PhaseCompleted {
            name: "files".to_string(),
            duration_secs: 0.0, // Duration will be tracked by the reporter stats
        });

        Ok(stats)
    }

    /// Get the project path.
    pub fn project_path(&self) -> &Path {
        &self.project_path
    }
}

impl Default for FileCollector {
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

    /// Helper to create test files
    fn create_test_file(dir: &Path, name: &str, content: &str) -> PathBuf {
        let path = dir.join(name);
        let mut file = StdFile::create(&path).unwrap();
        file.write_all(content.as_bytes()).unwrap();
        path
    }

    /// Helper to create test directory structure
    fn create_test_project() -> TempDir {
        let temp = TempDir::new().unwrap();

        // Create source files
        create_test_file(temp.path(), "main.rs", "fn main() { println!(\"Hello\"); }");
        create_test_file(temp.path(), "lib.rs", "pub fn hello() { \"Hello\" }");

        // Create documentation
        create_test_file(temp.path(), "README.md", "# Test Project\n\nThis is a test.");

        // Create subdirectory
        let src_dir = temp.path().join("src");
        std::fs::create_dir_all(&src_dir).unwrap();
        create_test_file(&src_dir, "utils.rs", "pub fn util() {}");

        temp
    }

    #[test]
    fn test_collector_new() {
        let temp = TempDir::new().unwrap();
        let collector = FileCollector::new(temp.path()).unwrap();
        assert_eq!(collector.project_path(), temp.path());
    }

    #[test]
    fn test_collector_default() {
        let collector = FileCollector::default();
        assert_eq!(collector.project_path(), Path::new("."));
    }

    #[test]
    fn test_collector_with_chunk_size() {
        let temp = TempDir::new().unwrap();
        let collector = FileCollector::new(temp.path())
            .unwrap()
            .with_chunk_size(50)
            .with_chunk_overlap(5);

        assert_eq!(collector.chunk_size, 50);
        assert_eq!(collector.chunk_overlap, 5);
    }

    #[test]
    fn test_collect_files_basic() {
        let temp = create_test_project();
        let collector = FileCollector::new(temp.path()).unwrap();

        let files = collector.collect_files(true, true, true).unwrap();

        assert_eq!(files.len(), 4);
        assert!(files.iter().any(|f| f.file_path == "main.rs"));
        assert!(files.iter().any(|f| f.file_path == "README.md"));
        assert!(files.iter().any(|f| f.file_path == "src/utils.rs"));

        // Check that files have expected properties
        let main_rs = files.iter().find(|f| f.file_path == "main.rs").unwrap();
        assert_eq!(main_rs.language, Some("rust".to_string()));
        assert!(!main_rs.indexed);
    }

    #[test]
    fn test_collect_files_source_only() {
        let temp = create_test_project();
        let collector = FileCollector::new(temp.path()).unwrap();

        let files = collector.collect_files(true, false, false).unwrap();

        assert_eq!(files.len(), 3); // Only .rs files
        assert!(files.iter().all(|f| f.file_path.ends_with(".rs")));
    }

    #[test]
    fn test_collect_files_docs_only() {
        let temp = create_test_project();
        let collector = FileCollector::new(temp.path()).unwrap();

        let files = collector.collect_files(false, true, false).unwrap();

        assert_eq!(files.len(), 1);
        assert_eq!(files[0].file_path, "README.md");
    }

    #[test]
    fn test_collect_file_content() {
        let temp = create_test_project();
        let main_rs = temp.path().join("main.rs");
        let collector = FileCollector::new(temp.path()).unwrap();

        let content = collector.collect_file_content(&main_rs).unwrap();

        assert!(content.contains("fn main()"));
        assert!(content.contains("Hello"));
    }

    #[test]
    fn test_collect_file_content_missing() {
        let temp = TempDir::new().unwrap();
        let collector = FileCollector::new(temp.path()).unwrap();

        let result = collector.collect_file_content(&temp.path().join("nonexistent.rs"));

        assert!(result.is_err());
    }

    #[test]
    fn test_file_chunking() {
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("test.rs");

        // Create a file with 50 lines
        let content: Vec<String> = (1..=50).map(|i| format!("Line {}", i)).collect();
        create_test_file(temp.path(), "test.rs", &content.join("\n"));

        let collector = FileCollector::new(temp.path()).unwrap();
        let chunks = collector.chunk_file_content(&content.join("\n"), &file_path);

        // With chunk_size=100, all 50 lines should be in one chunk
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].line_range, (1, 50));
        assert!(chunks[0].content.contains("Line 1"));
        assert!(chunks[0].content.contains("Line 50"));
    }

    #[test]
    fn test_file_chunking_multiple() {
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("test.rs");

        // Create a file with 250 lines
        let content: Vec<String> = (1..=250).map(|i| format!("Line {}", i)).collect();
        create_test_file(temp.path(), "test.rs", &content.join("\n"));

        let collector = FileCollector::new(temp.path())
            .unwrap()
            .with_chunk_size(100)
            .with_chunk_overlap(10);

        let chunks = collector.chunk_file_content(&content.join("\n"), &file_path);

        // Should create 3 chunks: 1-100, 91-190, 181-250
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].line_range, (1, 100));
        assert_eq!(chunks[1].line_range, (91, 190));
        assert_eq!(chunks[2].line_range, (181, 250));

        // Verify overlap
        assert!(chunks[1].content.contains("Line 91"));
        assert!(chunks[1].content.contains("Line 100"));
    }

    #[test]
    fn test_file_chunking_empty() {
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("test.rs");

        let collector = FileCollector::new(temp.path()).unwrap();
        let chunks = collector.chunk_file_content("", &file_path);

        assert_eq!(chunks.len(), 0);
    }

    #[test]
    fn test_generate_chunk_id() {
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("test.rs");
        let collector = FileCollector::new(temp.path()).unwrap();

        let id1 = collector.generate_chunk_id(&file_path, 0, 100);
        let id2 = collector.generate_chunk_id(&file_path, 0, 100);
        let id3 = collector.generate_chunk_id(&file_path, 100, 200);

        assert_eq!(id1, id2); // Same parameters produce same ID
        assert_ne!(id1, id3); // Different parameters produce different ID
        assert!(id1.starts_with("chunk:"));
    }

    #[test]
    fn test_storage_integration() {
        let temp = create_test_project();
        let collector = FileCollector::new(temp.path()).unwrap();
        let storage = StorageManager::open_project_db(temp.path()).unwrap();

        // Collect files
        let files = collector.collect_files(true, true, true).unwrap();

        // Store files
        let stats = collector.store_files(&files, &storage).unwrap();

        assert_eq!(stats.files_scanned, 4);
        assert_eq!(stats.files_collected, 4);
        assert_eq!(stats.errors, 0);

        // Verify files were stored
        let project_files = storage.get_project_files(&temp.path().to_string_lossy()).unwrap();
        assert_eq!(project_files.len(), 4);

        // Verify we can retrieve files
        for file_id in project_files {
            let file = storage.get_file(&file_id).unwrap();
            assert!(file.is_some());
        }
    }

    #[test]
    fn test_storage_incremental() {
        let temp = create_test_project();
        let collector = FileCollector::new(temp.path()).unwrap();
        let storage = StorageManager::open_project_db(temp.path()).unwrap();

        // Initial collection
        let (files1, deleted1) = collector
            .collect_incremental(&storage, true, true, true)
            .unwrap();
        assert_eq!(files1.len(), 4); // All files are new
        assert_eq!(deleted1.len(), 0);

        // Store files
        collector.store_files(&files1, &storage).unwrap();

        // Second collection should find no changes
        let (files2, deleted2) = collector
            .collect_incremental(&storage, true, true, true)
            .unwrap();
        assert_eq!(files2.len(), 0);
        assert_eq!(deleted2.len(), 0);

        // Modify a file
        create_test_file(temp.path(), "main.rs", "fn new_main() {}");

        // Third collection should find the modified file
        let (files3, deleted3) = collector
            .collect_incremental(&storage, true, true, true)
            .unwrap();
        assert_eq!(files3.len(), 1);
        assert_eq!(deleted3.len(), 0);
        assert_eq!(files3[0].file_path, "main.rs");
    }

    #[test]
    fn test_storage_incremental_deleted() {
        let temp = create_test_project();
        let collector = FileCollector::new(temp.path()).unwrap();
        let storage = StorageManager::open_project_db(temp.path()).unwrap();

        // Initial collection and storage
        let files = collector.collect_files(true, true, true).unwrap();
        collector.store_files(&files, &storage).unwrap();

        // Delete a file
        std::fs::remove_file(temp.path().join("main.rs")).unwrap();

        // Collection should detect deletion
        let (_, deleted) = collector
            .collect_incremental(&storage, true, true, true)
            .unwrap();
        assert_eq!(deleted.len(), 1);
        assert!(deleted[0].contains("main.rs"));
    }

    // Document parser integration tests

    #[test]
    fn test_chunk_markdown_document() {
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("test.md");

        let content = r#"# Title

## Section 1

First paragraph.

## Section 2

Second paragraph with **bold** text.
"#;

        create_test_file(temp.path(), "test.md", content);

        let collector = FileCollector::new(temp.path()).unwrap();
        let chunks = collector.chunk_file_content(content, &file_path);

        // DocumentParser should generate multiple chunks (headers + paragraphs)
        assert!(chunks.len() >= 3);

        // Verify chunk ID format (DocChunk-generated IDs)
        assert!(chunks[0].chunk_id.starts_with("chunk:"));
    }

    #[test]
    fn test_chunk_source_file_unchanged() {
        // Verify that source file behavior is unchanged
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("test.rs");

        let content = "fn main() {\n    println!(\"Hello\");\n}";

        create_test_file(temp.path(), "test.rs", content);

        let collector = FileCollector::new(temp.path()).unwrap();
        let chunks = collector.chunk_file_content(content, &file_path);

        // Source files still use line-based chunking
        assert_eq!(chunks.len(), 1);
    }

    #[test]
    fn test_chunk_rst_document() {
        // Test reStructuredText
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("test.rst");

        let content = "Title\n=====\n\nContent here.";

        create_test_file(temp.path(), "test.rst", content);

        let collector = FileCollector::new(temp.path()).unwrap();
        let chunks = collector.chunk_file_content(content, &file_path);

        assert!(chunks.len() >= 1);
    }

    #[test]
    fn test_chunk_unsupported_format() {
        // Test that unsupported formats fall back to line-based chunking
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("test.xyz");

        let content = "Line 1\nLine 2\nLine 3";

        create_test_file(temp.path(), "test.xyz", content);

        let collector = FileCollector::new(temp.path()).unwrap();
        let chunks = collector.chunk_file_content(content, &file_path);

        // Should fall back to line-based chunking
        assert_eq!(chunks.len(), 1);
    }

    #[test]
    fn test_chunk_markdown_with_code_blocks() {
        // Test that Markdown code blocks are recognized as separate chunks
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("test.md");

        let content = r#"# Title

Some text before code.

```rust
fn main() {
    println!("Hello");
}
```

Some text after code.
"#;

        create_test_file(temp.path(), "test.md", content);

        let collector = FileCollector::new(temp.path()).unwrap();
        let chunks = collector.chunk_file_content(content, &file_path);

        // Should have at least: title, paragraph, code block, paragraph = 4 chunks
        assert!(chunks.len() >= 3);

        // Verify code block content is preserved
        let has_code = chunks.iter().any(|c| c.content.contains("fn main"));
        assert!(has_code, "Code block should be preserved in chunks");
    }

    #[test]
    fn test_chunk_empty_document() {
        // Test empty document handling
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("empty.md");

        let collector = FileCollector::new(temp.path()).unwrap();
        let chunks = collector.chunk_file_content("", &file_path);

        assert_eq!(chunks.len(), 0);
    }

    #[test]
    fn test_chunk_document_parse_error_fallback() {
        // Test fallback behavior when document parsing fails
        // Files without extension will fall back to line-based chunking
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("no-extension");

        let content = "Line 1\nLine 2\nLine 3";

        create_test_file(temp.path(), "no-extension", content);

        let collector = FileCollector::new(temp.path()).unwrap();
        let chunks = collector.chunk_file_content(content, &file_path);

        // Should fall back to line-based chunking, return 1 chunk
        assert_eq!(chunks.len(), 1);
    }

    #[test]
    fn test_chunk_markdown_unicode() {
        // Test Unicode content handling
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("test.md");

        let content = "# 标题\n\n这是一段中文内容。\n\nAnd English too.";

        create_test_file(temp.path(), "test.md", content);

        let collector = FileCollector::new(temp.path()).unwrap();
        let chunks = collector.chunk_file_content(content, &file_path);

        // Should correctly parse Chinese content
        assert!(chunks.len() >= 2);
        let has_chinese = chunks.iter().any(|c| c.content.contains("中文"));
        assert!(has_chinese, "Chinese content should be preserved");
    }

    #[test]
    fn test_chunk_adoc_document() {
        // Test AsciiDoc format
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("test.adoc");

        let content = "= Title\n\nSome content.";

        create_test_file(temp.path(), "test.adoc", content);

        let collector = FileCollector::new(temp.path()).unwrap();
        let chunks = collector.chunk_file_content(content, &file_path);

        // Should parse AsciiDoc
        assert!(chunks.len() >= 1);
    }

    #[test]
    fn test_chunk_txt_document() {
        // Test plain text format
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("test.txt");

        let content = "First paragraph\n\nSecond paragraph";

        create_test_file(temp.path(), "test.txt", content);

        let collector = FileCollector::new(temp.path()).unwrap();
        let chunks = collector.chunk_file_content(content, &file_path);

        // Should parse plain text
        assert!(chunks.len() >= 1);
    }
}
