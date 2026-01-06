//! Gemini API adapter implementation
//!
//! Implements the AIProviderPort trait for Google's Gemini API.

use async_trait::async_trait;
use herald_core::ports::ai::{AIError, AIProviderPort, AIResponse, MultimodalPrompt};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::{debug, error, warn};

/// Default Gemini model
pub const DEFAULT_GEMINI_MODEL: &str = "gemini-2.0-flash-exp";

/// Gemini API base URL
const GEMINI_API_BASE: &str = "https://generativelanguage.googleapis.com/v1beta/models";

/// Maximum retry attempts for rate limiting
const MAX_RETRIES: u32 = 3;

/// Initial backoff delay in milliseconds
const INITIAL_BACKOFF_MS: u64 = 1000;

/// Request timeout in seconds
const REQUEST_TIMEOUT_SECS: u64 = 60;

/// Gemini API adapter
///
/// Communicates with Google's Gemini API for multimodal AI requests.
pub struct GeminiAdapter {
    client: Client,
    api_key: String,
    model: String,
}

impl GeminiAdapter {
    /// Creates a new Gemini adapter
    ///
    /// # Arguments
    /// * `api_key` - Google AI API key
    /// * `model` - Model identifier (e.g., "gemini-2.0-flash-exp")
    ///
    /// # Example
    /// ```
    /// use herald_adapters::ai::GeminiAdapter;
    ///
    /// let adapter = GeminiAdapter::new("AIza...", "gemini-2.0-flash-exp");
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

    /// Creates a new Gemini adapter with default model
    pub fn with_default_model(api_key: impl Into<String>) -> Self {
        Self::new(api_key, DEFAULT_GEMINI_MODEL)
    }

    /// Returns the model identifier
    pub fn model(&self) -> &str {
        &self.model
    }

    /// Returns the API endpoint URL
    fn api_url(&self) -> String {
        format!(
            "{}/{}:generateContent?key={}",
            GEMINI_API_BASE, self.model, self.api_key
        )
    }

    /// Builds the request body for the Gemini API
    fn build_request(&self, prompt: &MultimodalPrompt) -> GenerateContentRequest {
        let mut parts = Vec::new();

        // Add system instruction as first text part if present
        if !prompt.system_message.is_empty() {
            parts.push(Part::Text {
                text: prompt.system_message.clone(),
            });
        }

        // Add images
        for image in &prompt.user_message.images {
            parts.push(Part::InlineData {
                inline_data: InlineData {
                    mime_type: image.media_type.clone(),
                    data: image.base64.clone(),
                },
            });
        }

        // Add user text
        parts.push(Part::Text {
            text: prompt.user_message.text.clone(),
        });

        GenerateContentRequest {
            contents: vec![Content {
                role: Some("user".to_string()),
                parts,
            }],
            generation_config: Some(GenerationConfig {
                max_output_tokens: Some(4096),
                temperature: None,
            }),
        }
    }

    /// Sends a request to the Gemini API with retry logic
    async fn send_request(&self, request: &GenerateContentRequest) -> Result<AIResponse, AIError> {
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

    /// Sends a single request to the Gemini API
    async fn send_single_request(
        &self,
        request: &GenerateContentRequest,
    ) -> Result<AIResponse, AIError> {
        debug!(model = %self.model, "Sending request to Gemini API");

        let response = self
            .client
            .post(&self.api_url())
            .header("content-type", "application/json")
            .json(request)
            .send()
            .await
            .map_err(|e| {
                error!(error = %e, "Failed to send request to Gemini API");
                AIError::RequestFailed(e.to_string())
            })?;

        let status = response.status();

        if status.is_success() {
            let body: GenerateContentResponse = response.json().await.map_err(|e| {
                error!(error = %e, "Failed to parse Gemini API response");
                AIError::InvalidResponse(e.to_string())
            })?;

            // Extract text from response
            let text = body
                .candidates
                .into_iter()
                .flat_map(|c| c.content.parts)
                .filter_map(|part| match part {
                    ResponsePart::Text { text } => Some(text),
                })
                .collect::<Vec<_>>()
                .join("\n");

            if text.is_empty() {
                return Err(AIError::InvalidResponse(
                    "No text content in response".to_string(),
                ));
            }

            debug!(
                text_length = text.len(),
                "Received response from Gemini API"
            );

            Ok(AIResponse { text })
        } else {
            // Handle error responses
            let error_body = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());

            match status.as_u16() {
                401 | 403 => {
                    error!("Gemini API authentication failed");
                    Err(AIError::Unauthorized)
                }
                429 => {
                    warn!("Gemini API rate limit exceeded");
                    Err(AIError::RateLimitExceeded)
                }
                400 => {
                    error!(body = %error_body, "Gemini API invalid request");
                    Err(AIError::InvalidRequest(error_body))
                }
                _ => {
                    error!(status = %status, body = %error_body, "Gemini API error");
                    Err(AIError::ProviderError("gemini".to_string(), error_body))
                }
            }
        }
    }
}

#[async_trait]
impl AIProviderPort for GeminiAdapter {
    async fn generate_suggestion(&self, prompt: MultimodalPrompt) -> Result<AIResponse, AIError> {
        let request = self.build_request(&prompt);
        self.send_request(&request).await
    }
}

// === Request/Response Types ===

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct GenerateContentRequest {
    contents: Vec<Content>,
    #[serde(skip_serializing_if = "Option::is_none")]
    generation_config: Option<GenerationConfig>,
}

#[derive(Debug, Serialize)]
struct Content {
    #[serde(skip_serializing_if = "Option::is_none")]
    role: Option<String>,
    parts: Vec<Part>,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
enum Part {
    Text {
        text: String,
    },
    InlineData {
        #[serde(rename = "inlineData")]
        inline_data: InlineData,
    },
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct InlineData {
    mime_type: String,
    data: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct GenerationConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    max_output_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
}

#[derive(Debug, Deserialize)]
struct GenerateContentResponse {
    candidates: Vec<Candidate>,
}

#[derive(Debug, Deserialize)]
struct Candidate {
    content: ResponseContent,
}

#[derive(Debug, Deserialize)]
struct ResponseContent {
    parts: Vec<ResponsePart>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ResponsePart {
    Text { text: String },
}

#[cfg(test)]
mod tests {
    use super::*;
    use herald_core::ports::ai::{ImageData, UserMessage};

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
    fn test_gemini_adapter_creation() {
        let adapter = GeminiAdapter::new("test-key", "gemini-2.0-flash-exp");
        assert_eq!(adapter.model(), "gemini-2.0-flash-exp");
    }

    #[test]
    fn test_gemini_adapter_with_default_model() {
        let adapter = GeminiAdapter::with_default_model("test-key");
        assert_eq!(adapter.model(), DEFAULT_GEMINI_MODEL);
    }

    #[test]
    fn test_api_url_format() {
        let adapter = GeminiAdapter::new("test-api-key", "gemini-2.0-flash-exp");
        let url = adapter.api_url();

        assert!(url.contains("generativelanguage.googleapis.com"));
        assert!(url.contains("gemini-2.0-flash-exp"));
        assert!(url.contains("generateContent"));
        assert!(url.contains("key=test-api-key"));
    }

    #[test]
    fn test_build_request_structure() {
        let adapter = GeminiAdapter::new("test-key", "gemini-2.0-flash-exp");
        let prompt = create_test_prompt();
        let request = adapter.build_request(&prompt);

        assert_eq!(request.contents.len(), 1);
        assert_eq!(request.contents[0].role, Some("user".to_string()));
        // Should have 3 parts: system message + 1 image + user text
        assert_eq!(request.contents[0].parts.len(), 3);
    }

    #[test]
    fn test_build_request_with_multiple_images() {
        let adapter = GeminiAdapter::new("test-key", "gemini-2.0-flash-exp");
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

        // Should have 4 parts: system message + 2 images + user text
        assert_eq!(request.contents[0].parts.len(), 4);
    }

    #[test]
    fn test_build_request_with_no_system_message() {
        let adapter = GeminiAdapter::new("test-key", "gemini-2.0-flash-exp");
        let prompt = MultimodalPrompt {
            system_message: "".to_string(),
            user_message: UserMessage {
                text: "Text only".to_string(),
                images: vec![],
            },
        };
        let request = adapter.build_request(&prompt);

        // Should have only 1 part: user text (no system message, no images)
        assert_eq!(request.contents[0].parts.len(), 1);
    }

    #[test]
    fn test_request_serialization() {
        let adapter = GeminiAdapter::new("test-key", "gemini-2.0-flash-exp");
        let prompt = create_test_prompt();
        let request = adapter.build_request(&prompt);

        // Should serialize to valid JSON
        let json = serde_json::to_string(&request).expect("Failed to serialize");
        assert!(json.contains("contents"));
        assert!(json.contains("inlineData"));
        assert!(json.contains("mimeType"));
    }

    #[test]
    fn test_response_deserialization() {
        let json = r#"{
            "candidates": [
                {
                    "content": {
                        "parts": [
                            {"text": "This is a test response."}
                        ],
                        "role": "model"
                    },
                    "finishReason": "STOP"
                }
            ]
        }"#;

        let response: GenerateContentResponse =
            serde_json::from_str(json).expect("Failed to deserialize");
        assert_eq!(response.candidates.len(), 1);
        assert_eq!(response.candidates[0].content.parts.len(), 1);

        if let ResponsePart::Text { text } = &response.candidates[0].content.parts[0] {
            assert_eq!(text, "This is a test response.");
        } else {
            panic!("Expected text content");
        }
    }

    #[test]
    fn test_generation_config_serialization() {
        let config = GenerationConfig {
            max_output_tokens: Some(4096),
            temperature: None,
        };

        let json = serde_json::to_string(&config).expect("Failed to serialize");
        assert!(json.contains("maxOutputTokens"));
        assert!(!json.contains("temperature")); // None should be skipped
    }

    // Integration tests (require actual API key, marked as ignored)
    #[tokio::test]
    #[ignore = "Requires GOOGLE_AI_API_KEY environment variable"]
    async fn test_gemini_api_integration() {
        use std::env;

        let api_key = env::var("GOOGLE_AI_API_KEY").expect("GOOGLE_AI_API_KEY not set");
        let adapter = GeminiAdapter::with_default_model(api_key);

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
