//! Vector building with text chunking strategies.
//!
//! This module provides functionality to chunk different types of content
//! into appropriate pieces for embedding generation.

use crate::error::{RagError, Result};
use crate::models::ContentType;

/// Maximum chunk size for text chunks.
const MAX_CHUNK_SIZE: usize = 500;
/// Chunk overlap for better context.
const CHUNK_OVERLAP: usize = 50;

/// Vector builder for creating embeddings.
///
/// Provides different chunking strategies based on content type:
/// - **Sessions**: Chunk per message (no splitting)
/// - **Source**: Chunk per symbol + file summary
/// - **Docs**: Chunk by paragraph + file summary
pub struct VectorBuilder;

impl VectorBuilder {
    /// Create a new vector builder.
    pub fn new() -> Self {
        Self
    }

    /// Chunk text based on content type.
    ///
    /// # Arguments
    /// * `content` - The text content to chunk
    /// * `content_type` - The type of content (determines chunking strategy)
    ///
    /// # Returns
    /// A vector of text chunks ready for embedding
    pub fn chunk(&self, content: &str, content_type: ContentType) -> Result<Vec<String>> {
        match content_type {
            ContentType::Session => self.chunk_message(content),
            ContentType::Message => self.chunk_message(content),
            ContentType::File => self.chunk_file(content),
            ContentType::Symbol => self.chunk_symbol(content),
            ContentType::Commit => self.chunk_commit(content),
            ContentType::GitDiff => self.chunk_diff(content),
        }
    }

    /// Chunk text for message-level embedding.
    ///
    /// Messages are kept whole as they represent atomic units of conversation.
    /// Long messages (>500 chars) are split at sentence boundaries.
    pub fn chunk_message(&self, content: &str) -> Result<Vec<String>> {
        let content = content.trim();

        if content.is_empty() {
            return Ok(vec![]);
        }

        // If content is short enough, keep it whole
        if content.len() <= MAX_CHUNK_SIZE {
            return Ok(vec![content.to_string()]);
        }

        // Split long content into chunks at sentence boundaries
        let mut chunks = Vec::new();
        let mut current_chunk = String::new();
        let mut last_sentence_end = 0;

        for (i, c) in content.char_indices() {
            current_chunk.push(c);

            // Track sentence endings
            if c == '.' || c == '!' || c == '?' {
                // Check if followed by space or end of string
                let next_is_boundary = content[i + c.len_utf8()..].starts_with(' ')
                    || i + c.len_utf8() == content.len();

                if next_is_boundary {
                    last_sentence_end = current_chunk.len();
                }
            }

            // When approaching max size, try to split at last sentence
            if current_chunk.len() >= MAX_CHUNK_SIZE && last_sentence_end > 0 {
                let split_point = last_sentence_end;
                chunks.push(current_chunk[..split_point].trim().to_string());

                // Start new chunk with overlap
                let overlap_start = split_point.saturating_sub(CHUNK_OVERLAP);
                current_chunk = current_chunk[overlap_start..].to_string();
                last_sentence_end = 0;
            }
        }

        // Add remaining content
        if !current_chunk.trim().is_empty() {
            chunks.push(current_chunk.trim().to_string());
        }

        Ok(chunks)
    }

    /// Chunk text for file-level embedding.
    ///
    /// Files are chunked by paragraphs to maintain semantic coherence.
    pub fn chunk_file(&self, content: &str) -> Result<Vec<String>> {
        let content = content.trim();

        if content.is_empty() {
            return Ok(vec![]);
        }

        // Split by double newlines (paragraphs)
        let paragraphs: Vec<&str> = content.split("\n\n")
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect();

        if paragraphs.is_empty() {
            return Ok(vec![]);
        }

        // Group paragraphs into chunks
        let mut chunks = Vec::new();
        let mut current_chunk = String::new();

        for para in paragraphs {
            // If adding this paragraph would exceed chunk size
            if !current_chunk.is_empty()
                && current_chunk.len() + para.len() + 2 > MAX_CHUNK_SIZE
            {
                chunks.push(current_chunk.clone());
                current_chunk = String::new();
            }

            if !current_chunk.is_empty() {
                current_chunk.push_str("\n\n");
            }
            current_chunk.push_str(para);
        }

        if !current_chunk.is_empty() {
            chunks.push(current_chunk);
        }

        Ok(chunks)
    }

    /// Chunk text for symbol-level embedding.
    ///
    /// Symbols (functions, classes, etc.) are kept whole with their signature
    /// and docstring. Very long symbols (>500 chars) are truncated.
    pub fn chunk_symbol(&self, content: &str) -> Result<Vec<String>> {
        let content = content.trim();

        if content.is_empty() {
            return Ok(vec![]);
        }

        // For symbols, prefer keeping them whole
        // If too long, truncate with indicator
        if content.len() > MAX_CHUNK_SIZE * 2 {
            let truncated = format!("{}...\n[truncated: {} total chars]",
                &content[..MAX_CHUNK_SIZE],
                content.len()
            );
            Ok(vec![truncated])
        } else {
            Ok(vec![content.to_string()])
        }
    }

    /// Chunk text for commit message embedding.
    ///
    /// Commit messages are kept whole, including conventional commit structure.
    pub fn chunk_commit(&self, content: &str) -> Result<Vec<String>> {
        let content = content.trim();

        if content.is_empty() {
            return Ok(vec![]);
        }

        // Keep commit messages whole for context
        Ok(vec![content.to_string()])
    }

    /// Chunk text for git diff embedding.
    ///
    /// Diffs are chunked per file to maintain file-specific context.
    pub fn chunk_diff(&self, content: &str) -> Result<Vec<String>> {
        let content = content.trim();

        if content.is_empty() {
            return Ok(vec![]);
        }

        // Split by file headers (diff --git a/...)
        let mut chunks = Vec::new();
        let mut current_chunk = String::new();

        for line in content.lines() {
            // New file starts
            if line.starts_with("diff --git") {
                if !current_chunk.trim().is_empty() {
                    chunks.push(current_chunk.trim().to_string());
                }
                current_chunk = String::new();
            }

            current_chunk.push_str(line);
            current_chunk.push('\n');
        }

        // Add last chunk
        if !current_chunk.trim().is_empty() {
            chunks.push(current_chunk.trim().to_string());
        }

        // If no file headers found, treat as single chunk
        if chunks.is_empty() && !content.is_empty() {
            chunks.push(content.to_string());
        }

        Ok(chunks)
    }

    /// Normalize vector using L2 normalization.
    ///
    /// Ensures all vectors have unit length for cosine similarity calculation.
    ///
    /// # Arguments
    /// * `vector` - The vector to normalize in-place
    pub fn normalize(&self, vector: &mut [f32]) {
        let norm: f32 = vector.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 0.0 {
            for v in vector.iter_mut() {
                *v /= norm;
            }
        }
    }

    /// Calculate cosine similarity between two vectors.
    ///
    /// # Arguments
    /// * `a` - First vector (must be normalized)
    /// * `b` - Second vector (must be normalized)
    ///
    /// # Returns
    /// Cosine similarity score (-1.0 to 1.0, higher is more similar)
    pub fn cosine_similarity(&self, a: &[f32], b: &[f32]) -> Result<f32> {
        if a.len() != b.len() {
            return Err(RagError::Validation(format!(
                "Vector dimension mismatch: {} vs {}",
                a.len(),
                b.len()
            )));
        }

        let dot_product: f32 = a.iter()
            .zip(b.iter())
            .map(|(x, y)| x * y)
            .sum();

        Ok(dot_product)
    }

    /// Create a file summary chunk.
    ///
    /// Combines file path, language, and overview for file-level context.
    ///
    /// # Arguments
    /// * `file_path` - Path to the file
    /// * `language` - Programming language (optional)
    /// * `content_preview` - First few lines of content
    pub fn create_file_summary(
        &self,
        file_path: &str,
        language: Option<&str>,
        content_preview: &str,
    ) -> String {
        let mut summary = format!("File: {}", file_path);

        if let Some(lang) = language {
            summary.push_str(&format!("\nLanguage: {}", lang));
        }

        if !content_preview.is_empty() {
            let preview = content_preview.lines()
                .take(5)
                .collect::<Vec<_>>()
                .join("\n");

            summary.push_str(&format!("\n\n{}", preview));
        }

        summary
    }
}

impl Default for VectorBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize() {
        let builder = VectorBuilder::new();
        let mut vec = vec![3.0, 4.0];
        builder.normalize(&mut vec);

        let norm: f32 = vec.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_normalize_zero_vector() {
        let builder = VectorBuilder::new();
        let mut vec = vec![0.0, 0.0, 0.0];
        builder.normalize(&mut vec);

        // Should remain zero without division by zero
        assert_eq!(vec, vec![0.0, 0.0, 0.0]);
    }

    #[test]
    fn test_chunk_short_message() {
        let builder = VectorBuilder::new();
        let content = "This is a short message.";
        let chunks = builder.chunk_message(content).unwrap();

        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], content);
    }

    #[test]
    fn test_chunk_long_message() {
        let builder = VectorBuilder::new();
        let content = "This is sentence one. This is sentence two. ".repeat(20);
        let chunks = builder.chunk_message(&content).unwrap();

        assert!(chunks.len() > 1);
        // Each chunk should be within reasonable bounds
        for chunk in &chunks {
            assert!(chunk.len() <= MAX_CHUNK_SIZE + CHUNK_OVERLAP + 100);
        }
    }

    #[test]
    fn test_chunk_empty_message() {
        let builder = VectorBuilder::new();
        let chunks = builder.chunk_message("").unwrap();

        assert_eq!(chunks.len(), 0);
    }

    #[test]
    fn test_chunk_whitespace_only() {
        let builder = VectorBuilder::new();
        let chunks = builder.chunk_message("   \n\n  \t  ").unwrap();

        assert_eq!(chunks.len(), 0);
    }

    #[test]
    fn test_chunk_file_by_paragraphs() {
        let builder = VectorBuilder::new();
        let content = "Paragraph one.\n\nParagraph two.\n\nParagraph three.";
        let chunks = builder.chunk_file(content).unwrap();

        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].contains("Paragraph one"));
        assert!(chunks[0].contains("Paragraph two"));
        assert!(chunks[0].contains("Paragraph three"));
    }

    #[test]
    fn test_chunk_symbol() {
        let builder = VectorBuilder::new();
        let content = "fn example() {\n    // implementation\n}";
        let chunks = builder.chunk_symbol(content).unwrap();

        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], content);
    }

    #[test]
    fn test_chunk_symbol_truncate() {
        let builder = VectorBuilder::new();
        let content = "x".repeat(2000);
        let chunks = builder.chunk_symbol(&content).unwrap();

        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].len() < 2000);
        assert!(chunks[0].contains("truncated"));
    }

    #[test]
    fn test_chunk_commit() {
        let builder = VectorBuilder::new();
        let content = "feat(api): add new endpoint\n\nThis adds a new API endpoint.";
        let chunks = builder.chunk_commit(content).unwrap();

        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], content);
    }

    #[test]
    fn test_chunk_diff_multiple_files() {
        let builder = VectorBuilder::new();
        let content = "diff --git a/file1.rs b/file1.rs\n+new line\n\
                       diff --git a/file2.rs b/file2.rs\n+another line";
        let chunks = builder.chunk_diff(content).unwrap();

        assert_eq!(chunks.len(), 2);
        assert!(chunks[0].contains("file1.rs"));
        assert!(chunks[1].contains("file2.rs"));
    }

    #[test]
    fn test_chunk_diff_single_file() {
        let builder = VectorBuilder::new();
        let content = "+new line\n-another line";
        let chunks = builder.chunk_diff(content).unwrap();

        assert_eq!(chunks.len(), 1);
    }

    #[test]
    fn test_cosine_similarity() {
        let builder = VectorBuilder::new();
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![1.0, 0.0, 0.0];

        let similarity = builder.cosine_similarity(&a, &b).unwrap();
        assert!((similarity - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_cosine_similarity_orthogonal() {
        let builder = VectorBuilder::new();
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![0.0, 1.0, 0.0];

        let similarity = builder.cosine_similarity(&a, &b).unwrap();
        assert!((similarity - 0.0).abs() < 0.001);
    }

    #[test]
    fn test_cosine_similarity_dimension_mismatch() {
        let builder = VectorBuilder::new();
        let a = vec![1.0, 0.0];
        let b = vec![1.0];

        let result = builder.cosine_similarity(&a, &b);
        assert!(result.is_err());
    }

    #[test]
    fn test_create_file_summary() {
        let builder = VectorBuilder::new();
        let summary = builder.create_file_summary(
            "src/main.rs",
            Some("Rust"),
            "fn main() {\n    println!(\"Hello\");\n}"
        );

        assert!(summary.contains("src/main.rs"));
        assert!(summary.contains("Rust"));
        assert!(summary.contains("fn main"));
    }

    #[test]
    fn test_create_file_summary_no_language() {
        let builder = VectorBuilder::new();
        let summary = builder.create_file_summary(
            "README.md",
            None,
            "# Project Title\n\nDescription here"
        );

        assert!(summary.contains("README.md"));
        assert!(!summary.contains("Language:"));
    }

    #[test]
    fn test_chunk_by_content_type() {
        let builder = VectorBuilder::new();
        let content = "test content";

        // Test different content types
        let msg_chunks = builder.chunk(content, ContentType::Message).unwrap();
        let file_chunks = builder.chunk(content, ContentType::File).unwrap();
        let commit_chunks = builder.chunk(content, ContentType::Commit).unwrap();

        assert_eq!(msg_chunks.len(), 1);
        assert_eq!(file_chunks.len(), 1);
        assert_eq!(commit_chunks.len(), 1);
    }
}
