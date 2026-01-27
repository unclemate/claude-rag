//! HNSW (Hierarchical Navigable Small World) vector index.
//!
//! Implements a simplified HNSW algorithm for approximate nearest neighbor search.
//! Supports type-based filtering for multi-modal retrieval.

use std::path::Path;
use std::collections::{HashMap, HashSet};
use serde::{Deserialize, Serialize, Serializer, Deserializer};

use crate::error::Result;
use crate::models::ContentType;

/// HNSW index node.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct HnswNode {
    /// Node ID (matches content ID).
    id: String,
    /// Content type for filtering.
    content_type: ContentType,
    /// Vector embedding.
    vector: Vec<f32>,
    /// Neighbors at each level [(level, neighbor_id)].
    neighbors: Vec<(usize, String)>,
}

/// Serializable state for HnswIndex.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct HnswState {
    nodes: HashMap<String, HnswNode>,
    entry_point: Option<String>,
    max_level: usize,
}

/// HNSW vector index.
pub struct HnswIndex {
    /// All nodes in the index.
    nodes: HashMap<String, HnswNode>,
    /// Entry point for search.
    entry_point: Option<String>,
    /// Max connections per layer.
    m: usize,
    /// Search width during build.
    ef_construction: usize,
    /// Search width during query.
    ef_search: usize,
    /// Max level of any node.
    max_level: usize,
    /// ML parameter (normalization factor for level generation).
    ml: f64,
}

// Custom serialization for HnswIndex
impl Serialize for HnswIndex {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        HnswState {
            nodes: self.nodes.clone(),
            entry_point: self.entry_point.clone(),
            max_level: self.max_level,
        }.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for HnswIndex {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let state = HnswState::deserialize(deserializer)?;
        let ml = 1.0 / (16.0_f64).ln(); // Default m=16
        Ok(Self {
            nodes: state.nodes,
            entry_point: state.entry_point,
            m: 16, // Default
            ef_construction: 200, // Default
            ef_search: 50, // Default
            max_level: state.max_level,
            ml,
        })
    }
}

impl HnswIndex {
    /// Create a new HNSW index.
    pub fn new(m: usize, ef_construction: usize, ef_search: usize) -> Self {
        let ml = 1.0 / (m as f64).ln();
        Self {
            nodes: HashMap::new(),
            entry_point: None,
            m,
            ef_construction,
            ef_search,
            max_level: 0,
            ml,
        }
    }

    /// Generate random level for a new node.
    fn random_level(&self, rng: &mut fastrand::Rng) -> usize {
        // Using exponential distribution
        (-rng.f64().ln() * self.ml).floor() as usize
    }

    /// Calculate Euclidean distance between two vectors.
    fn distance(&self, a: &[f32], b: &[f32]) -> f32 {
        if a.len() != b.len() {
            return f32::INFINITY;
        }
        a.iter()
            .zip(b.iter())
            .map(|(x, y)| (x - y).powi(2))
            .sum::<f32>()
            .sqrt()
    }

    /// Greedy search for nearest neighbors at a specific level.
    fn search_layer(
        &self,
        query: &[f32],
        entry_points: &[String],
        ef: usize,
        _level: usize,
        content_filter: Option<ContentType>,
    ) -> Vec<(String, f32)> {
        let mut visited: HashSet<String> = HashSet::new();
        let mut candidates: Vec<(String, f32)> = Vec::new();
        let mut w: Vec<(String, f32)> = Vec::new();

        // Initialize with entry points
        for ep in entry_points {
            if let Some(node) = self.nodes.get(ep) {
                // Apply content filter
                if let Some(filter) = content_filter {
                    if node.content_type != filter {
                        continue;
                    }
                }
                let dist = self.distance(query, &node.vector);
                candidates.push((ep.clone(), dist));
                w.push((ep.clone(), dist));
                visited.insert(ep.clone());
            }
        }

        while !candidates.is_empty() {
            // Sort candidates by distance
            candidates.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());

            // Get closest candidate
            let (current_id, current_dist) = candidates.remove(0);

            // Check if we should continue
            if let Some((_, farthest_dist)) = w.last() {
                if !w.is_empty() && current_dist > *farthest_dist && w.len() >= ef {
                    break;
                }
            }

            // Explore neighbors
            if let Some(current_node) = self.nodes.get(&current_id) {
                for (_, neighbor_id) in &current_node.neighbors {
                    if visited.contains(neighbor_id) {
                        continue;
                    }

                    if let Some(neighbor_node) = self.nodes.get(neighbor_id) {
                        // Apply content filter
                        if let Some(filter) = content_filter {
                            if neighbor_node.content_type != filter {
                                continue;
                            }
                        }

                        visited.insert(neighbor_id.clone());
                        let dist = self.distance(query, &neighbor_node.vector);

                        // Add to candidates
                        candidates.push((neighbor_id.clone(), dist));

                        // Add to w if there's room or if closer
                        if w.len() < ef {
                            w.push((neighbor_id.clone(), dist));
                            w.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
                        } else if let Some((_, farthest_dist)) = w.last() {
                            if dist < *farthest_dist {
                                w.pop();
                                w.push((neighbor_id.clone(), dist));
                                w.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
                            }
                        }
                    }
                }
            }
        }

        w
    }

    /// Insert a vector into the index.
    pub fn insert(&mut self, id: String, content_type: ContentType, vector: Vec<f32>) -> Result<()> {
        let mut rng = fastrand::Rng::new();
        let level = self.random_level(&mut rng);

        // Create new node
        let mut node = HnswNode {
            id: id.clone(),
            content_type,
            vector,
            neighbors: Vec::new(),
        };

        // Find entry point
        let mut entry_points = Vec::new();
        if let Some(ep) = &self.entry_point {
            entry_points.push(ep.clone());
        }

        // Search from top level down to level+1
        for lc in (level + 1..=self.max_level).rev() {
            if !entry_points.is_empty() {
                let result = self.search_layer(&node.vector, &entry_points, 1, lc, None);
                if !result.is_empty() {
                    entry_points = vec![result[0].0.clone()];
                }
            }
        }

        // Insert at levels 0..=level
        for lc in 0..=level {
            let ef = if lc == 0 { self.ef_construction } else { 1 };
            let w = self.search_layer(&node.vector, &entry_points, ef, lc, None);

            // Select neighbors
            let num_neighbors = self.m.min(w.len());
            let selected: Vec<String> = w.into_iter()
                .take(num_neighbors)
                .map(|(id, _)| id)
                .collect();

            // Add bidirectional connections
            for neighbor_id in &selected {
                if let Some(neighbor_node) = self.nodes.get_mut(neighbor_id) {
                    neighbor_node.neighbors.push((lc, id.clone()));
                }
            }

            // Update node's neighbors
            for neighbor_id in &selected {
                node.neighbors.push((lc, neighbor_id.clone()));
            }

            // Update entry points for next level
            entry_points = selected;
        }

        // Insert node
        self.nodes.insert(id.clone(), node);

        // Update entry point if necessary
        if level > self.max_level || self.entry_point.is_none() {
            self.entry_point = Some(id);
            self.max_level = level.max(self.max_level);
        }

        Ok(())
    }

    /// Search for nearest neighbors.
    pub fn search(
        &self,
        query: &[f32],
        k: usize,
        content_type_filter: Option<ContentType>,
    ) -> Result<Vec<(String, f32)>> {
        if self.nodes.is_empty() {
            return Ok(Vec::new());
        }

        let mut entry_points = Vec::new();
        if let Some(ep) = &self.entry_point {
            entry_points.push(ep.clone());
        }

        // Search from top level down to level 1
        for level in (1..=self.max_level).rev() {
            if !entry_points.is_empty() {
                let result = self.search_layer(query, &entry_points, 1, level, content_type_filter);
                if !result.is_empty() {
                    entry_points = vec![result[0].0.clone()];
                }
            }
        }

        // Final search at level 0
        let mut results = self.search_layer(query, &entry_points, self.ef_search.max(k), 0, content_type_filter);

        // Return top-k results
        results.truncate(k);
        Ok(results)
    }

    /// Save index to disk.
    pub fn save(&self, path: &Path) -> Result<()> {
        let data = serde_json::to_vec_pretty(self)?;
        std::fs::write(path, data)?;
        Ok(())
    }

    /// Load index from disk.
    pub fn load(path: &Path) -> Result<Self> {
        let data = std::fs::read(path)?;
        let index: HnswIndex = serde_json::from_slice(&data)?;
        Ok(index)
    }

    /// Get number of nodes in index.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Check if index is empty.
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Get max level.
    pub fn max_level(&self) -> usize {
        self.max_level
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
        assert_eq!(index.max_level(), 0);
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
        assert!(!index.is_empty());
    }

    #[test]
    fn test_hnsw_search() {
        let mut index = HnswIndex::new(16, 200, 50);

        // Insert some vectors
        index.insert("a".to_string(), ContentType::Message, vec![0.0, 0.0]).unwrap();
        index.insert("b".to_string(), ContentType::Message, vec![1.0, 1.0]).unwrap();
        index.insert("c".to_string(), ContentType::File, vec![0.5, 0.5]).unwrap();

        // Search without filter - should find something
        let results = index.search(&[0.1, 0.1], 2, None).unwrap();
        assert!(!results.is_empty());

        // Search without filter - should find closest
        let results = index.search(&[0.0, 0.0], 2, None).unwrap();
        assert!(!results.is_empty());
        // First result should be "a" (closest to [0.0, 0.0])
        assert_eq!(results[0].0, "a");
    }

    #[test]
    fn test_hnsw_distance() {
        let index = HnswIndex::new(16, 200, 50);

        let a = vec![0.0, 0.0];
        let b = vec![1.0, 0.0];
        let c = vec![0.0, 1.0];

        let dist_ab = index.distance(&a, &b);
        let dist_ac = index.distance(&a, &c);
        let dist_bc = index.distance(&b, &c);

        // Distance should be symmetric
        assert_eq!(dist_ab, index.distance(&b, &a));
        assert_eq!(dist_ac, index.distance(&c, &a));

        // b and c should have same distance from a
        assert_eq!(dist_ab, dist_ac);

        // b and c should be farther apart
        assert!(dist_bc > dist_ab);
    }

    #[test]
    fn test_hnsw_save_load() {
        let mut index = HnswIndex::new(16, 200, 50);
        index.insert("test".to_string(), ContentType::Message, vec![0.1, 0.2]).unwrap();

        let temp_dir = tempfile::TempDir::new().unwrap();
        let path = temp_dir.path().join("hnsw.json");

        index.save(&path).unwrap();
        let loaded = HnswIndex::load(&path).unwrap();

        assert_eq!(loaded.len(), 1);
        assert!(!loaded.is_empty());
    }
}
