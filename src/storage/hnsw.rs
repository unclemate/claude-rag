//! HNSW (Hierarchical Navigable Small World) vector index.

use std::path::Path;
use serde::{Deserialize, Serialize};
use anyhow::Context;

use crate::error::{Result, RagError};
use crate::models::ContentType;

/// HNSW index node.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct HnswNode {
    /// Node ID (matches content ID).
    id: String,
    /// Content type.
    content_type: ContentType,
    /// Vector embedding.
    vector: Vec<f32>,
    /// Neighbors at each level [(level, neighbor_id)].
    neighbors: Vec<(usize, String)>,
}

/// HNSW vector index.
pub struct HnswIndex {
    /// Index nodes.
    nodes: Vec<HnswNode>,
    /// Entry point for search.
    entry_point: Option<String>,
    /// Max connections per layer.
    m: usize,
    /// Search width during build.
    ef_construction: usize,
    /// Search width during query.
    ef_search: usize,
}

impl HnswIndex {
    /// Create a new HNSW index.
    pub fn new(m: usize, ef_construction: usize, ef_search: usize) -> Self {
        Self {
            nodes: Vec::new(),
            entry_point: None,
            m,
            ef_construction,
            ef_search,
        }
    }

    /// Insert a vector into the index.
    pub fn insert(&mut self, id: String, content_type: ContentType, vector: Vec<f32>) -> Result<()> {
        let node = HnswNode {
            id: id.clone(),
            content_type,
            vector,
            neighbors: Vec::new(),
        };

        // For now, just add to nodes (simplified)
        self.nodes.push(node);

        // Update entry point if this is the first node
        if self.entry_point.is_none() {
            self.entry_point = Some(id);
        }

        Ok(())
    }

    /// Search for nearest neighbors.
    pub fn search(&self, _query: &[f32], _k: usize, _content_type_filter: Option<ContentType>) -> Result<Vec<(String, f32)>> {
        // Simplified: return empty for now
        // Full HNSW implementation will be added later
        Ok(Vec::new())
    }

    /// Save index to disk.
    pub fn save(&self, _path: &Path) -> Result<()> {
        // TODO: Implement persistence
        Ok(())
    }

    /// Load index from disk.
    pub fn load(_path: &Path) -> Result<Self> {
        // TODO: Implement loading
        Err(RagError::NotFound("Load not implemented".to_string()))
    }

    /// Get number of nodes in index.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Check if index is empty.
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hnsw_new() {
        let index = HnswIndex::new(16, 200, 50);
        assert_eq!(index.m, 16);
        assert!(index.is_empty());
    }

    #[test]
    fn test_hnsw_insert() {
        let mut index = HnswIndex::new(16, 200, 50);
        index.insert(
            "test-id".to_string(),
            ContentType::Message,
            vec![0.1, 0.2, 0.3],
        ).unwrap();

        assert_eq!(index.len(), 1);
    }
}
