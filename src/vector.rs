//! Vector building with text chunking strategies.

use crate::error::Result;

/// Vector builder for creating embeddings.
pub struct VectorBuilder;

impl VectorBuilder {
    /// Create a new vector builder.
    pub fn new() -> Self {
        Self
    }

    /// Chunk text for message-level embedding.
    pub fn chunk_message(&self, _content: &str) -> Result<Vec<String>> {
        // TODO: Implement message chunking
        Ok(vec![])
    }

    /// Chunk text for symbol-level embedding.
    pub fn chunk_symbol(&self, _content: &str) -> Result<Vec<String>> {
        // TODO: Implement symbol chunking
        Ok(vec![])
    }

    /// Normalize vector (L2 normalization).
    pub fn normalize(&self, vector: &mut [f32]) {
        let norm: f32 = vector.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 0.0 {
            for v in vector.iter_mut() {
                *v /= norm;
            }
        }
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
}
