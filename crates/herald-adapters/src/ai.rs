//! AI provider adapter implementations
//!
//! Contains Claude and Gemini API adapters implementing the AIProviderPort trait.

mod claude;
mod gemini;

pub use claude::ClaudeAdapter;
pub use gemini::GeminiAdapter;

#[cfg(test)]
mod tests {
    use super::*;
    use herald_core::ports::ai::{AIProviderPort, ImageData, MultimodalPrompt, UserMessage};

    fn create_test_prompt() -> MultimodalPrompt {
        MultimodalPrompt {
            system_message: "You are a helpful assistant.".to_string(),
            user_message: UserMessage {
                text: "What's in this image?".to_string(),
                images: vec![ImageData {
                    base64: "aGVsbG8=".to_string(),
                    media_type: "image/png".to_string(),
                }],
            },
        }
    }

    #[test]
    fn test_claude_adapter_implements_trait() {
        // Verify that ClaudeAdapter can be used as AIProviderPort
        fn _assert_trait<T: AIProviderPort>() {}
        _assert_trait::<ClaudeAdapter>();
    }

    #[test]
    fn test_gemini_adapter_implements_trait() {
        // Verify that GeminiAdapter can be used as AIProviderPort
        fn _assert_trait<T: AIProviderPort>() {}
        _assert_trait::<GeminiAdapter>();
    }

    #[test]
    fn test_claude_adapter_new() {
        let adapter = ClaudeAdapter::new("test-api-key", "claude-3-5-sonnet-20241022");
        assert!(adapter.model().contains("claude"));
    }

    #[test]
    fn test_gemini_adapter_new() {
        let adapter = GeminiAdapter::new("test-api-key", "gemini-2.0-flash-exp");
        assert!(adapter.model().contains("gemini"));
    }
}
