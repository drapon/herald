//! Claude API adapter implementation
//!
//! Implements the AIProviderPort trait for Anthropic's Claude API.

use async_trait::async_trait;
use herald_core::ports::ai::{AIError, AIProviderPort, AIResponse, MultimodalPrompt};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::{debug, error, warn};

/// Default Claude model
pub const DEFAULT_CLAUDE_MODEL: &str = "claude-sonnet-4-20250514";

/// Claude API endpoint
const CLAUDE_API_URL: &str = "https://api.anthropic.com/v1/messages";

/// Anthropic API version header
const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Maximum retry attempts for rate limiting
const MAX_RETRIES: u32 = 3;

/// Initial backoff delay in milliseconds
const INITIAL_BACKOFF_MS: u64 = 1000;

/// Request timeout in seconds
const REQUEST_TIMEOUT_SECS: u64 = 60;

/// Claude API adapter
///
/// Communicates with Anthropic's Claude Messages API for multimodal AI requests.
pub struct ClaudeAdapter {
    client: Client,
    api_key: String,
    model: String,
}

impl ClaudeAdapter {
    /// Creates a new Claude adapter
    ///
    /// # Arguments
    /// * `api_key` - Anthropic API key
    /// * `model` - Model identifier (e.g., "claude-3-5-sonnet-20241022")
    ///
    /// # Example
    /// ```
    /// use herald_adapters::ai::ClaudeAdapter;
    ///
    /// let adapter = ClaudeAdapter::new("sk-ant-...", "claude-3-5-sonnet-20241022");
    /// ```
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
            .build()
            .expect("Failed to create HTTP client");

        Self {
            client,
            api_key: api_key.into(),
            model: model.into(),
        }
    }

    /// Creates a new Claude adapter with default model
    pub fn with_default_model(api_key: impl Into<String>) -> Self {
        Self::new(api_key, DEFAULT_CLAUDE_MODEL)
    }

    /// Returns the model identifier
    pub fn model(&self) -> &str {
        &self.model
    }

    /// Builds the request body for the Claude API
    fn build_request(&self, prompt: &MultimodalPrompt) -> CreateMessageRequest {
        let mut content = Vec::new();

        // Add images first
        for image in &prompt.user_message.images {
            content.push(ContentBlock::Image {
                source: ImageSource {
                    source_type: "base64".to_string(),
                    media_type: image.media_type.clone(),
                    data: image.base64.clone(),
                },
            });
        }

        // Add text
        content.push(ContentBlock::Text {
            text: prompt.user_message.text.clone(),
        });

        CreateMessageRequest {
            model: self.model.clone(),
            max_tokens: 4096,
            system: Some(prompt.system_message.clone()),
            messages: vec![Message {
                role: "user".to_string(),
                content,
            }],
        }
    }

    /// Sends a request to the Claude API with retry logic
    async fn send_request(&self, request: &CreateMessageRequest) -> Result<AIResponse, AIError> {
        let mut last_error = None;

        for attempt in 0..MAX_RETRIES {
            match self.send_single_request(request).await {
                Ok(response) => return Ok(response),
                Err(AIError::RateLimitExceeded) => {
                    let backoff = Duration::from_millis(INITIAL_BACKOFF_MS * 2u64.pow(attempt));
                    warn!(
                        attempt = attempt + 1,
                        backoff_ms = backoff.as_millis(),
                        "Rate limited, retrying after backoff"
                    );
                    tokio::time::sleep(backoff).await;
                    last_error = Some(AIError::RateLimitExceeded);
                }
                Err(e) => return Err(e),
            }
        }

        Err(last_error.unwrap_or(AIError::RateLimitExceeded))
    }

    /// Sends a single request to the Claude API
    async fn send_single_request(
        &self,
        request: &CreateMessageRequest,
    ) -> Result<AIResponse, AIError> {
        debug!(model = %self.model, "Sending request to Claude API");

        let response = self
            .client
            .post(CLAUDE_API_URL)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("content-type", "application/json")
            .json(request)
            .send()
            .await
            .map_err(|e| {
                error!(error = %e, "Failed to send request to Claude API");
                AIError::RequestFailed(e.to_string())
            })?;

        let status = response.status();

        if status.is_success() {
            let body: CreateMessageResponse = response.json().await.map_err(|e| {
                error!(error = %e, "Failed to parse Claude API response");
                AIError::InvalidResponse(e.to_string())
            })?;

            // Extract text from response
            let text = body
                .content
                .into_iter()
                .filter_map(|block| match block {
                    ResponseContent::Text { text } => Some(text),
                })
                .collect::<Vec<_>>()
                .join("\n");

            if text.is_empty() {
                return Err(AIError::InvalidResponse(
                    "No text content in response".to_string(),
                ));
            }

            debug!(
                stop_reason = ?body.stop_reason,
                text_length = text.len(),
                "Received response from Claude API"
            );

            Ok(AIResponse { text })
        } else {
            // Handle error responses
            let error_body = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());

            match status.as_u16() {
                401 => {
                    error!("Claude API authentication failed");
                    Err(AIError::Unauthorized)
                }
                429 => {
                    warn!("Claude API rate limit exceeded");
                    Err(AIError::RateLimitExceeded)
                }
                400 => {
                    error!(body = %error_body, "Claude API invalid request");
                    Err(AIError::InvalidRequest(error_body))
                }
                _ => {
                    error!(status = %status, body = %error_body, "Claude API error");
                    Err(AIError::ProviderError("claude".to_string(), error_body))
                }
            }
        }
    }
}

#[async_trait]
impl AIProviderPort for ClaudeAdapter {
    async fn generate_suggestion(&self, prompt: MultimodalPrompt) -> Result<AIResponse, AIError> {
        let request = self.build_request(&prompt);
        self.send_request(&request).await
    }
}

// === Request/Response Types ===

#[derive(Debug, Serialize)]
struct CreateMessageRequest {
    model: String,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    messages: Vec<Message>,
}

#[derive(Debug, Serialize)]
struct Message {
    role: String,
    content: Vec<ContentBlock>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ContentBlock {
    Text { text: String },
    Image { source: ImageSource },
}

#[derive(Debug, Serialize)]
struct ImageSource {
    #[serde(rename = "type")]
    source_type: String,
    media_type: String,
    data: String,
}

#[derive(Debug, Deserialize)]
struct CreateMessageResponse {
    #[allow(dead_code)]
    id: String,
    content: Vec<ResponseContent>,
    #[allow(dead_code)]
    stop_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ResponseContent {
    Text { text: String },
}

#[cfg(test)]
mod tests {
    use super::*;
    use herald_core::ports::ai::UserMessage;

    fn create_test_prompt() -> MultimodalPrompt {
        MultimodalPrompt {
            system_message: "You are a helpful assistant.".to_string(),
            user_message: UserMessage {
                text: "Describe this image.".to_string(),
                images: vec![ImageData {
                    base64: "aGVsbG8gd29ybGQ=".to_string(),
                    media_type: "image/png".to_string(),
                }],
            },
        }
    }

    #[test]
    fn test_claude_adapter_creation() {
        let adapter = ClaudeAdapter::new("test-key", "claude-3-5-sonnet-20241022");
        assert_eq!(adapter.model(), "claude-3-5-sonnet-20241022");
    }

    #[test]
    fn test_claude_adapter_with_default_model() {
        let adapter = ClaudeAdapter::with_default_model("test-key");
        assert_eq!(adapter.model(), DEFAULT_CLAUDE_MODEL);
    }

    #[test]
    fn test_build_request_structure() {
        let adapter = ClaudeAdapter::new("test-key", "claude-3-5-sonnet-20241022");
        let prompt = create_test_prompt();
        let request = adapter.build_request(&prompt);

        assert_eq!(request.model, "claude-3-5-sonnet-20241022");
        assert_eq!(request.max_tokens, 4096);
        assert!(request.system.is_some());
        assert_eq!(request.messages.len(), 1);
        assert_eq!(request.messages[0].role, "user");
        // Should have 2 content blocks: 1 image + 1 text
        assert_eq!(request.messages[0].content.len(), 2);
    }

    #[test]
    fn test_build_request_with_multiple_images() {
        let adapter = ClaudeAdapter::new("test-key", "claude-3-5-sonnet-20241022");
        let prompt = MultimodalPrompt {
            system_message: "Test".to_string(),
            user_message: UserMessage {
                text: "Test text".to_string(),
                images: vec![
                    ImageData {
                        base64: "img1".to_string(),
                        media_type: "image/png".to_string(),
                    },
                    ImageData {
                        base64: "img2".to_string(),
                        media_type: "image/jpeg".to_string(),
                    },
                ],
            },
        };
        let request = adapter.build_request(&prompt);

        // Should have 3 content blocks: 2 images + 1 text
        assert_eq!(request.messages[0].content.len(), 3);
    }

    #[test]
    fn test_build_request_with_no_images() {
        let adapter = ClaudeAdapter::new("test-key", "claude-3-5-sonnet-20241022");
        let prompt = MultimodalPrompt {
            system_message: "Test".to_string(),
            user_message: UserMessage {
                text: "Text only prompt".to_string(),
                images: vec![],
            },
        };
        let request = adapter.build_request(&prompt);

        // Should have only 1 content block: text
        assert_eq!(request.messages[0].content.len(), 1);
    }

    #[test]
    fn test_request_serialization() {
        let adapter = ClaudeAdapter::new("test-key", "claude-3-5-sonnet-20241022");
        let prompt = create_test_prompt();
        let request = adapter.build_request(&prompt);

        // Should serialize to valid JSON
        let json = serde_json::to_string(&request).expect("Failed to serialize");
        assert!(json.contains("claude-3-5-sonnet-20241022"));
        assert!(json.contains("image"));
        assert!(json.contains("base64"));
    }

    #[test]
    fn test_response_deserialization() {
        let json = r#"{
            "id": "msg_123",
            "content": [
                {"type": "text", "text": "This is a test response."}
            ],
            "stop_reason": "end_turn"
        }"#;

        let response: CreateMessageResponse =
            serde_json::from_str(json).expect("Failed to deserialize");
        assert_eq!(response.id, "msg_123");
        assert_eq!(response.content.len(), 1);

        if let ResponseContent::Text { text } = &response.content[0] {
            assert_eq!(text, "This is a test response.");
        } else {
            panic!("Expected text content");
        }
    }

    #[test]
    fn test_response_with_multiple_text_blocks() {
        let json = r#"{
            "id": "msg_456",
            "content": [
                {"type": "text", "text": "First part."},
                {"type": "text", "text": "Second part."}
            ],
            "stop_reason": "end_turn"
        }"#;

        let response: CreateMessageResponse =
            serde_json::from_str(json).expect("Failed to deserialize");
        assert_eq!(response.content.len(), 2);
    }

    // Integration tests (require actual API key, marked as ignored)
    #[tokio::test]
    #[ignore = "Requires ANTHROPIC_API_KEY environment variable"]
    async fn test_claude_api_integration() {
        use std::env;

        let api_key = env::var("ANTHROPIC_API_KEY").expect("ANTHROPIC_API_KEY not set");
        let adapter = ClaudeAdapter::with_default_model(api_key);

        let prompt = MultimodalPrompt {
            system_message: "You are a helpful assistant. Respond in one sentence.".to_string(),
            user_message: UserMessage {
                text: "What is 2 + 2?".to_string(),
                images: vec![],
            },
        };

        let result = adapter.generate_suggestion(prompt).await;
        assert!(result.is_ok(), "API call failed: {:?}", result.err());

        let response = result.unwrap();
        assert!(!response.text.is_empty());
    }
}
