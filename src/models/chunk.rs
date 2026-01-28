//! Document chunk model for paragraph-level indexing.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Kind of document chunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ChunkKind {
    /// Document header/title.
    Header,
    /// Paragraph text.
    Paragraph,
    /// Code block.
    CodeBlock,
    /// Other content.
    Other,
}

impl fmt::Display for ChunkKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ChunkKind::Header => write!(f, "Header"),
            ChunkKind::Paragraph => write!(f, "Paragraph"),
            ChunkKind::CodeBlock => write!(f, "CodeBlock"),
            ChunkKind::Other => write!(f, "Other"),
        }
    }
}

/// A document chunk for paragraph-level indexing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocChunk {
    /// Unique chunk ID.
    pub id: String,
    /// File ID this chunk belongs to.
    pub file_id: String,
    /// Chunk kind.
    pub kind: ChunkKind,
    /// Chunk content.
    pub content: String,
    /// Header level (1-6) if this is a header.
    pub header_level: Option<u8>,
    /// Header path (e.g., "Introduction > Getting Started").
    pub header_path: Vec<String>,
    /// Start line number (1-indexed).
    pub start_line: usize,
    /// End line number (1-indexed).
    pub end_line: usize,
    /// Programming language (for code blocks).
    pub language: Option<String>,
}

impl DocChunk {
    /// Create a new document chunk.
    ///
    /// # Arguments
    /// * `file_id` - The file ID this chunk belongs to
    /// * `kind` - The chunk kind
    /// * `content` - The chunk content
    /// * `start_line` - Start line number (1-indexed)
    /// * `end_line` - End line number (1-indexed)
    #[must_use]
    pub fn new(
        file_id: String,
        kind: ChunkKind,
        content: String,
        start_line: usize,
        end_line: usize,
    ) -> Self {
        let id = Self::generate_id(&file_id, start_line);
        Self {
            id,
            file_id,
            kind,
            content,
            header_level: None,
            header_path: Vec::new(),
            start_line,
            end_line,
            language: None,
        }
    }

    /// Set the header level.
    #[must_use]
    pub const fn with_header_level(mut self, level: u8) -> Self {
        self.header_level = Some(level);
        self
    }

    /// Set the header path.
    #[must_use]
    pub fn with_header_path(mut self, path: Vec<String>) -> Self {
        self.header_path = path;
        self
    }

    /// Set the programming language (for code blocks).
    #[must_use]
    pub fn with_language(mut self, language: String) -> Self {
        self.language = Some(language);
        self
    }

    /// Generate a unique chunk ID.
    fn generate_id(file_id: &str, line: usize) -> String {
        use sha2::{Digest, Sha256};
        let input = format!("{}:chunk:{}", file_id, line);
        format!("chunk:{:x}", Sha256::digest(input.as_bytes()))
    }

    /// Get the full header path as a string.
    #[must_use]
    pub fn header_path_string(&self) -> String {
        self.header_path.join(" > ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chunk_kind_display() {
        assert_eq!(ChunkKind::Header.to_string(), "Header");
        assert_eq!(ChunkKind::Paragraph.to_string(), "Paragraph");
        assert_eq!(ChunkKind::CodeBlock.to_string(), "CodeBlock");
    }

    #[test]
    fn test_doc_chunk_new() {
        let chunk = DocChunk::new(
            "file-1".to_string(),
            ChunkKind::Paragraph,
            "Test content".to_string(),
            10,
            15,
        );

        assert_eq!(chunk.file_id, "file-1");
        assert_eq!(chunk.kind, ChunkKind::Paragraph);
        assert_eq!(chunk.content, "Test content");
        assert_eq!(chunk.start_line, 10);
        assert_eq!(chunk.end_line, 15);
        assert!(chunk.header_level.is_none());
        assert!(chunk.header_path.is_empty());
        assert!(chunk.language.is_none());
    }

    #[test]
    fn test_doc_chunk_with_header_level() {
        let chunk = DocChunk::new(
            "file-1".to_string(),
            ChunkKind::Header,
            "# Test".to_string(),
            1,
            1,
        )
        .with_header_level(1);

        assert_eq!(chunk.header_level, Some(1));
    }

    #[test]
    fn test_doc_chunk_with_header_path() {
        let chunk = DocChunk::new(
            "file-1".to_string(),
            ChunkKind::Paragraph,
            "Content".to_string(),
            10,
            12,
        )
        .with_header_path(vec!["Chapter 1".to_string(), "Section 1.1".to_string()]);

        assert_eq!(chunk.header_path.len(), 2);
        assert_eq!(chunk.header_path_string(), "Chapter 1 > Section 1.1");
    }

    #[test]
    fn test_doc_chunk_with_language() {
        let chunk = DocChunk::new(
            "file-1".to_string(),
            ChunkKind::CodeBlock,
            "fn main() {}".to_string(),
            10,
            12,
        )
        .with_language("rust".to_string());

        assert_eq!(chunk.language, Some("rust".to_string()));
    }

    #[test]
    fn test_doc_chunk_id_is_deterministic() {
        let chunk1 = DocChunk::new("file-1".to_string(), ChunkKind::Paragraph, "Test".to_string(), 10, 12);
        let chunk2 = DocChunk::new("file-1".to_string(), ChunkKind::Paragraph, "Test".to_string(), 10, 12);

        assert_eq!(chunk1.id, chunk2.id);
        assert!(chunk1.id.starts_with("chunk:"));
    }

    #[test]
    fn test_doc_chunk_id_differs_by_line() {
        let chunk1 = DocChunk::new("file-1".to_string(), ChunkKind::Paragraph, "Test".to_string(), 10, 12);
        let chunk2 = DocChunk::new("file-1".to_string(), ChunkKind::Paragraph, "Test".to_string(), 20, 22);

        assert_ne!(chunk1.id, chunk2.id);
    }

    #[test]
    fn test_header_path_string_empty() {
        let chunk = DocChunk::new("file-1".to_string(), ChunkKind::Paragraph, "Test".to_string(), 1, 2);
        assert_eq!(chunk.header_path_string(), "");
    }

    #[test]
    fn test_chunk_kind_equality() {
        assert_eq!(ChunkKind::Header, ChunkKind::Header);
        assert_ne!(ChunkKind::Header, ChunkKind::Paragraph);
    }

    #[test]
    fn test_chunk_kind_hashable() {
        use std::collections::HashSet;
        let mut set = HashSet::new();
        set.insert(ChunkKind::Header);
        set.insert(ChunkKind::Paragraph);
        set.insert(ChunkKind::CodeBlock);
        assert_eq!(set.len(), 3);
    }
}
