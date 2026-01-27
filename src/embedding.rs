//! Zhipu AI embedding API client.
//!
//! Provides async embedding generation using the Zhipu AI embedding-3 API.

use reqwest::{Client, header};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::time::timeout;

use crate::error::{Result, RagError};
use crate::config::EmbeddingConfig;

/// Default API timeout in seconds.
const DEFAULT_TIMEOUT_SECS: u64 = 30;

/// Maximum retry attempts for API calls.
const MAX_RETRIES: u32 = 3;

/// Base delay between retries in milliseconds.
const RETRY_DELAY_MS: u64 = 1000;

/// Zhipu AI embedding API request.
#[derive(Debug, Serialize)]
struct EmbeddingRequest {
    model: String,
    input: Vec<String>,
    encoding_format: Option<String>,
}

/// Zhipu AI embedding API response.
#[derive(Debug, Deserialize)]
struct EmbeddingResponse {
    data: Vec<EmbeddingData>,
    model: String,
    usage: Option<Usage>,
}

/// Individual embedding data.
#[derive(Debug, Deserialize)]
struct EmbeddingData {
    embedding: Vec<f32>,
    index: usize,
}

/// Token usage information.
#[derive(Debug, Deserialize)]
struct Usage {
    prompt_tokens: usize,
    total_tokens: usize,
}

/// Zhipu AI API error response.
#[derive(Debug, Deserialize)]
struct ApiError {
    error: ErrorDetail,
}

#[derive(Debug, Deserialize)]
struct ErrorDetail {
    message: String,
    #[serde(default)]
    code: Option<String>,
}

/// Embedding client for Zhipu AI API.
pub struct EmbeddingClient {
    /// HTTP client.
    client: Client,
    /// API token.
    api_token: String,
    /// API URL.
    api_url: String,
    /// Model name.
    model: String,
    /// Request timeout.
    timeout: Duration,
}

impl EmbeddingClient {
    /// Create a new embedding client from config.
    pub fn from_config(config: &EmbeddingConfig) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_millis(config.timeout_ms))
            .build()
            .unwrap_or_else(|_| Client::new());

        Self {
            client,
            api_token: config.api_token.clone(),
            api_url: config.api_url.clone(),
            model: "embedding-3".to_string(),
            timeout: Duration::from_millis(config.timeout_ms),
        }
    }

    /// Create a new embedding client.
    pub fn new(api_token: String, api_url: Option<String>) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(DEFAULT_TIMEOUT_SECS))
            .build()
            .unwrap_or_else(|_| Client::new());

        Self {
            client,
            api_token,
            api_url: api_url.unwrap_or_else(|| {
                "https://open.bigmodel.cn/api/paas/v4/embeddings".to_string()
            }),
            model: "embedding-3".to_string(),
            timeout: Duration::from_secs(DEFAULT_TIMEOUT_SECS),
        }
    }

    /// Generate embeddings for a single text.
    pub async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let mut results = self.embed_batch(&[text.to_string()]).await?;
        Ok(results.remove(0))
    }

    /// Generate embeddings for multiple texts (batch).
    pub async fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }

        let mut last_error = None;

        for attempt in 0..MAX_RETRIES {
            // Calculate delay with exponential backoff
            if attempt > 0 {
                let delay = RETRY_DELAY_MS * 2_u64.pow(attempt - 1);
                tokio::time::sleep(Duration::from_millis(delay)).await;
            }

            // Try to send request
            match self.send_request(texts).await {
                Ok(response) => return Ok(response),
                Err(e) => {
                    // Check if error is retryable
                    let retryable = matches!(
                        &e,
                        RagError::Http(_) | RagError::Embedding(_)
                    );

                    if retryable && attempt < MAX_RETRIES - 1 {
                        last_error = Some(e);
                        continue;
                    } else {
                        return Err(e);
                    }
                }
            }
        }

        Err(last_error.unwrap_or_else(|| {
            RagError::Embedding("Max retries exceeded".to_string())
        }))
    }

    /// Send embedding request to API.
    async fn send_request(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let request = EmbeddingRequest {
            model: self.model.clone(),
            input: texts.to_vec(),
            encoding_format: Some("float".to_string()),
        };

        // Build request with authorization
        let response = timeout(self.timeout, async {
            self.client
                .post(&self.api_url)
                .header(header::AUTHORIZATION, format!("Bearer {}", self.api_token))
                .header(header::CONTENT_TYPE, "application/json")
                .json(&request)
                .send()
                .await
        })
        .await
        .map_err(|_| RagError::Embedding("Request timeout".to_string()))?
        .map_err(|e| RagError::Http(e.to_string()))?;

        // Check status code
        let status = response.status();
        if !status.is_success() {
            // Try to parse error response
            let error_text = response.text().await.unwrap_or_else(|_| "Unknown error".to_string());
            if let Ok(api_error) = serde_json::from_str::<ApiError>(&error_text) {
                return Err(RagError::Embedding(api_error.error.message));
            }
            return Err(RagError::Embedding(format!("HTTP error: {}", status)));
        }

        // Parse response
        let embedding_response: EmbeddingResponse = response
            .json()
            .await
            .map_err(|e| RagError::Embedding(format!("Invalid response: {}", e)))?;

        // Extract embeddings in order by index
        let mut embeddings = vec![Vec::new(); embedding_response.data.len()];
        for data in embedding_response.data {
            if data.index < embeddings.len() {
                embeddings[data.index] = data.embedding;
            }
        }

        Ok(embeddings)
    }

    /// Get the model name being used.
    pub fn model(&self) -> &str {
        &self.model
    }

    /// Check if the client is properly configured.
    pub fn is_configured(&self) -> bool {
        !self.api_token.is_empty() && !self.api_url.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_new() {
        let client = EmbeddingClient::new("test-token".to_string(), None);
        assert_eq!(client.model(), "embedding-3");
        assert!(client.is_configured());
    }

    #[test]
    fn test_client_not_configured() {
        let client = EmbeddingClient::new("".to_string(), None);
        assert!(!client.is_configured());
    }

    #[test]
    fn test_client_from_config() {
        let config = EmbeddingConfig {
            api_token: "test-token".to_string(),
            api_url: "https://api.example.com".to_string(),
            dimensions: 1024,
            batch_size: 8,
            timeout_ms: 30000,
        };

        let client = EmbeddingClient::from_config(&config);
        assert!(client.is_configured());
    }

    #[test]
    fn test_embedding_request_serialization() {
        let request = EmbeddingRequest {
            model: "embedding-3".to_string(),
            input: vec!["test".to_string()],
            encoding_format: Some("float".to_string()),
        };

        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("embedding-3"));
        assert!(json.contains("test"));
    }

    #[test]
    fn test_api_error_deserialization() {
        let json = r#"{"error":{"message":"Invalid token","code":"401"}}"#;
        let error: ApiError = serde_json::from_str(json).unwrap();
        assert_eq!(error.error.message, "Invalid token");
        assert_eq!(error.error.code, Some("401".to_string()));
    }
}
