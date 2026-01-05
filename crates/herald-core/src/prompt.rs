//! Prompt builder for AI interactions
//!
//! Builds multimodal prompts from user intent and captured images.

use crate::ports::ai::{AIError, ImageData, MultimodalPrompt, UserMessage};

/// Maximum number of images allowed in a single prompt
pub const MAX_IMAGES: usize = 10;

/// Builder for constructing multimodal AI prompts
pub struct PromptBuilder;

impl PromptBuilder {
    /// Builds a multimodal prompt from user intent and images
    ///
    /// # Arguments
    /// * `intent` - User's intention/question (e.g., "UIデザイン改善", "コード最適化")
    /// * `images` - List of images to include in the prompt (max 10)
    ///
    /// # Returns
    /// * `Ok(MultimodalPrompt)` - Successfully constructed prompt
    /// * `Err(AIError::InvalidRequest)` - If intent is empty
    /// * `Err(AIError::ImageLimitExceeded)` - If more than 10 images provided
    ///
    /// # Example
    /// ```
    /// use herald_core::prompt::PromptBuilder;
    /// use herald_core::ports::ai::ImageData;
    ///
    /// let images = vec![
    ///     ImageData {
    ///         base64: "aGVsbG8=".to_string(),
    ///         media_type: "image/png".to_string(),
    ///     }
    /// ];
    ///
    /// let prompt = PromptBuilder::build_multimodal_prompt("UIデザインを改善したい", images)
    ///     .expect("Failed to build prompt");
    ///
    /// assert!(!prompt.system_message.is_empty());
    /// ```
    pub fn build_multimodal_prompt(
        intent: &str,
        images: Vec<ImageData>,
    ) -> Result<MultimodalPrompt, AIError> {
        // Validate intent is not empty
        let trimmed_intent = intent.trim();
        if trimmed_intent.is_empty() {
            return Err(AIError::InvalidRequest(
                "Intent cannot be empty".to_string(),
            ));
        }

        // Validate image count
        if images.len() > MAX_IMAGES {
            return Err(AIError::ImageLimitExceeded(MAX_IMAGES));
        }

        // Build system message
        let system_message = Self::build_system_message();

        // Build user message
        let user_message = UserMessage {
            text: Self::build_user_text(trimmed_intent),
            images,
        };

        Ok(MultimodalPrompt {
            system_message,
            user_message,
        })
    }

    /// Builds the system message for the AI
    fn build_system_message() -> String {
        r#"あなたはデスクトップ作業を観察するAIアシスタントです。
提供されたスクリーンショット群から作業コンテキストを理解し、
ユーザーの意図に基づいた具体的なアドバイスを提供してください。

以下の点を心がけてください：
- スクリーンショットに写っている内容を正確に把握する
- ユーザーの作業フローや目的を推測する
- 実践的で具体的なアドバイスを提供する
- 必要に応じてステップバイステップの手順を示す"#
            .to_string()
    }

    /// Builds the user text portion of the prompt
    fn build_user_text(intent: &str) -> String {
        format!(
            "以下のスクリーンショットを参考に、私の意図に応えてください。\n\n私の意図: {}",
            intent
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_image() -> ImageData {
        ImageData {
            base64: "aGVsbG8gd29ybGQ=".to_string(), // "hello world" in base64
            media_type: "image/png".to_string(),
        }
    }

    fn create_test_images(count: usize) -> Vec<ImageData> {
        (0..count).map(|_| create_test_image()).collect()
    }

    // === Basic Functionality Tests ===

    #[test]
    fn test_build_prompt_with_valid_intent_and_images() {
        let images = create_test_images(3);
        let result = PromptBuilder::build_multimodal_prompt("UIデザインを改善したい", images);

        assert!(result.is_ok());
        let prompt = result.unwrap();

        // System message should not be empty
        assert!(!prompt.system_message.is_empty());

        // User message should contain the intent
        assert!(prompt.user_message.text.contains("UIデザインを改善したい"));

        // Images should be included
        assert_eq!(prompt.user_message.images.len(), 3);
    }

    #[test]
    fn test_build_prompt_with_no_images() {
        let images: Vec<ImageData> = vec![];
        let result = PromptBuilder::build_multimodal_prompt("コードを最適化したい", images);

        assert!(result.is_ok());
        let prompt = result.unwrap();
        assert!(prompt.user_message.images.is_empty());
    }

    #[test]
    fn test_build_prompt_with_max_images() {
        let images = create_test_images(MAX_IMAGES);
        let result = PromptBuilder::build_multimodal_prompt("作業効率を上げたい", images);

        assert!(result.is_ok());
        let prompt = result.unwrap();
        assert_eq!(prompt.user_message.images.len(), MAX_IMAGES);
    }

    // === Validation Tests ===

    #[test]
    fn test_build_prompt_fails_with_empty_intent() {
        let images = create_test_images(1);
        let result = PromptBuilder::build_multimodal_prompt("", images);

        assert!(result.is_err());
        match result {
            Err(AIError::InvalidRequest(msg)) => {
                assert!(msg.contains("empty"));
            }
            _ => panic!("Expected InvalidRequest error"),
        }
    }

    #[test]
    fn test_build_prompt_fails_with_whitespace_only_intent() {
        let images = create_test_images(1);
        let result = PromptBuilder::build_multimodal_prompt("   \t\n  ", images);

        assert!(result.is_err());
        match result {
            Err(AIError::InvalidRequest(msg)) => {
                assert!(msg.contains("empty"));
            }
            _ => panic!("Expected InvalidRequest error"),
        }
    }

    #[test]
    fn test_build_prompt_fails_with_too_many_images() {
        let images = create_test_images(MAX_IMAGES + 1);
        let result = PromptBuilder::build_multimodal_prompt("test", images);

        assert!(result.is_err());
        match result {
            Err(AIError::ImageLimitExceeded(limit)) => {
                assert_eq!(limit, MAX_IMAGES);
            }
            _ => panic!("Expected ImageLimitExceeded error"),
        }
    }

    // === System Message Tests ===

    #[test]
    fn test_system_message_contains_key_instructions() {
        let images = create_test_images(1);
        let prompt =
            PromptBuilder::build_multimodal_prompt("test", images).expect("Failed to build prompt");

        let system_msg = &prompt.system_message;

        // Should contain key instructions in Japanese
        assert!(system_msg.contains("デスクトップ作業"));
        assert!(system_msg.contains("スクリーンショット"));
        assert!(system_msg.contains("アドバイス"));
    }

    // === User Message Tests ===

    #[test]
    fn test_user_message_includes_intent() {
        let images = create_test_images(1);
        let intent = "特定のバグを修正したい";
        let prompt =
            PromptBuilder::build_multimodal_prompt(intent, images).expect("Failed to build prompt");

        assert!(prompt.user_message.text.contains(intent));
    }

    #[test]
    fn test_user_message_trims_intent() {
        let images = create_test_images(1);
        let intent = "  意図の前後に空白  ";
        let prompt =
            PromptBuilder::build_multimodal_prompt(intent, images).expect("Failed to build prompt");

        // Should contain trimmed intent
        assert!(prompt.user_message.text.contains("意図の前後に空白"));
        // But not the extra whitespace at boundaries
        assert!(!prompt.user_message.text.contains("  意図の前後に空白  "));
    }

    // === Edge Cases ===

    #[test]
    fn test_build_prompt_with_special_characters_in_intent() {
        let images = create_test_images(1);
        let intent = "What's the <best> way to \"optimize\" this? & more";
        let result = PromptBuilder::build_multimodal_prompt(intent, images);

        assert!(result.is_ok());
        let prompt = result.unwrap();
        assert!(prompt.user_message.text.contains(intent));
    }

    #[test]
    fn test_build_prompt_with_unicode_intent() {
        let images = create_test_images(1);
        let intent = "日本語と英語とEmoji 🎉 を混ぜた意図";
        let result = PromptBuilder::build_multimodal_prompt(intent, images);

        assert!(result.is_ok());
        let prompt = result.unwrap();
        assert!(prompt.user_message.text.contains(intent));
    }

    #[test]
    fn test_build_prompt_with_different_media_types() {
        let images = vec![
            ImageData {
                base64: "abc123".to_string(),
                media_type: "image/png".to_string(),
            },
            ImageData {
                base64: "def456".to_string(),
                media_type: "image/jpeg".to_string(),
            },
        ];
        let result = PromptBuilder::build_multimodal_prompt("test", images);

        assert!(result.is_ok());
        let prompt = result.unwrap();
        assert_eq!(prompt.user_message.images.len(), 2);
        assert_eq!(prompt.user_message.images[0].media_type, "image/png");
        assert_eq!(prompt.user_message.images[1].media_type, "image/jpeg");
    }
}
