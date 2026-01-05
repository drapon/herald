//! AI Provider port definition

use async_trait::async_trait;
use thiserror::Error;

/// Image data for AI analysis
#[derive(Debug, Clone)]
pub struct ImageData {
    /// Base64 encoded image data
    pub base64: String,
    /// MIME type (e.g., "image/png")
    pub media_type: String,
}

/// Multimodal prompt for AI providers
#[derive(Debug, Clone)]
pub struct MultimodalPrompt {
    /// System message for the AI
    pub system_message: String,
    /// User message with text and images
    pub user_message: UserMessage,
}

/// User message containing text and optional images
#[derive(Debug, Clone)]
pub struct UserMessage {
    /// Text content
    pub text: String,
    /// List of images to include
    pub images: Vec<ImageData>,
}

/// Response from an AI provider
#[derive(Debug, Clone)]
pub struct AIResponse {
    /// Generated text response
    pub text: String,
}

/// Errors that can occur during AI operations
#[derive(Debug, Error)]
pub enum AIError {
    /// API key is missing or invalid
    #[error("Unauthorized: API key is missing or invalid. Please set the appropriate environment variable (ANTHROPIC_API_KEY or GOOGLE_AI_API_KEY)")]
    Unauthorized,

    /// Rate limit exceeded
    #[error("Rate limit exceeded. Please wait and try again.")]
    RateLimitExceeded,

    /// Invalid request (e.g., too many images, invalid format)
    #[error("Invalid request: {0}")]
    InvalidRequest(String),

    /// Provider-specific error
    #[error("Provider '{0}' error: {1}")]
    ProviderError(String, String),

    /// Request failed
    #[error("API request failed: {0}")]
    RequestFailed(String),

    /// Invalid response from API
    #[error("Invalid API response: {0}")]
    InvalidResponse(String),

    /// Too many images provided
    #[error("Image limit exceeded: maximum {0} images allowed")]
    ImageLimitExceeded(usize),
}

/// Port for AI provider operations
#[async_trait]
pub trait AIProviderPort: Send + Sync {
    /// Generate a suggestion based on the given prompt
    async fn generate_suggestion(&self, prompt: MultimodalPrompt) -> Result<AIResponse, AIError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_image_data() {
        let image = ImageData {
            base64: "aGVsbG8=".to_string(),
            media_type: "image/png".to_string(),
        };
        assert_eq!(image.media_type, "image/png");
    }

    #[test]
    fn test_multimodal_prompt() {
        let prompt = MultimodalPrompt {
            system_message: "You are a helpful assistant.".to_string(),
            user_message: UserMessage {
                text: "What's in this image?".to_string(),
                images: vec![],
            },
        };
        assert!(!prompt.system_message.is_empty());
        assert!(!prompt.user_message.text.is_empty());
    }

    #[test]
    fn test_ai_error_messages() {
        let err = AIError::Unauthorized;
        assert!(err.to_string().contains("API key"));

        let err = AIError::RateLimitExceeded;
        assert!(err.to_string().contains("Rate limit"));

        let err = AIError::ImageLimitExceeded(10);
        assert!(err.to_string().contains("10"));

        let err = AIError::InvalidRequest("Too many images".to_string());
        assert!(err.to_string().contains("Too many images"));

        let err = AIError::ProviderError("claude".to_string(), "Server error".to_string());
        assert!(err.to_string().contains("claude"));
    }
}
