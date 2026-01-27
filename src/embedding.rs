//! Zhipu AI embedding API client.

use crate::error::Result;

/// Embedding client for Zhipu AI API.
pub struct EmbeddingClient {
    /// API token.
    api_token: String,
    /// API URL.
    api_url: String,
}

impl EmbeddingClient {
    /// Create a new embedding client.
    pub fn new(api_token: String, api_url: Option<String>) -> Self {
        Self {
            api_token,
            api_url: api_url.unwrap_or_else(|| {
                "https://open.bigmodel.cn/api/paas/v4/embeddings".to_string()
            }),
        }
    }

    /// Generate embeddings for a single text.
    pub async fn embed(&self, _text: &str) -> Result<Vec<f32>> {
        // TODO: Implement API call
        Ok(Vec::new())
    }

    /// Generate embeddings for multiple texts (batch).
    pub async fn embed_batch(&self, _texts: &[String]) -> Result<Vec<Vec<f32>>> {
        // TODO: Implement batch API call
        Ok(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_new() {
        let client = EmbeddingClient::new("test-token".to_string(), None);
        assert_eq!(client.api_token, "test-token");
    }
}
