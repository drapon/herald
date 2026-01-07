//! Activity analyzer adapter implementation
//!
//! Implements the ActivityAnalyzerPort trait using AI providers (Claude/Gemini)
//! to analyze screenshots and extract activity information.

use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use herald_core::ports::activity::{
    ActivityAnalyzerPort, ActivityCategory, AnalysisError, AnalysisResult, CaptureForAnalysis,
};
use herald_core::ports::ai::{AIProviderPort, ImageData, MultimodalPrompt, UserMessage};
use serde::Deserialize;
use std::path::Path;
use std::sync::Arc;
use tokio::fs;
use tracing::{debug, error, warn};

/// System prompt for activity analysis
const ACTIVITY_ANALYSIS_SYSTEM_PROMPT: &str = r#"You are an assistant that analyzes screenshots to determine what activity the user is performing.

Analyze the screenshot and extract:
1. The name of the application being used (e.g., "Visual Studio Code", "Chrome", "Slack")
2. A brief description of what the user is doing (e.g., "Writing Rust code", "Reading documentation", "Chatting with team")
3. The category of the activity

Categories:
- coding: Software development, writing code, debugging
- documentation: Writing or reading documentation, notes, articles
- communication: Email, chat, messaging apps
- browsing: General web browsing
- meeting: Video calls, conferences
- design: Design work, graphics editing
- other: Anything that doesn't fit the above categories

Respond ONLY with valid JSON in this exact format:
{
  "application_name": "Application Name",
  "activity_description": "Brief description of the activity",
  "category": "category_name"
}

Do not include any text before or after the JSON."#;

/// User prompt template for activity analysis
const ACTIVITY_ANALYSIS_USER_PROMPT: &str =
    "Analyze this screenshot and determine what activity the user is performing.";

/// AI-based activity analyzer
///
/// Uses an AI provider to analyze screenshots and extract activity information.
pub struct AIActivityAnalyzer<P: AIProviderPort> {
    ai_provider: Arc<P>,
}

impl<P: AIProviderPort> AIActivityAnalyzer<P> {
    /// Creates a new AI activity analyzer
    ///
    /// # Arguments
    /// * `ai_provider` - The AI provider to use for analysis
    pub fn new(ai_provider: Arc<P>) -> Self {
        Self { ai_provider }
    }

    /// Reads and encodes an image file to base64
    async fn read_image_as_base64(file_path: &str) -> Result<ImageData, AnalysisError> {
        let path = Path::new(file_path);

        // Check if file exists
        if !path.exists() {
            return Err(AnalysisError::ImageReadError(format!(
                "File not found: {}",
                file_path
            )));
        }

        // Read file contents
        let data = fs::read(path).await.map_err(|e| {
            error!(error = %e, path = %file_path, "Failed to read image file");
            AnalysisError::ImageReadError(e.to_string())
        })?;

        // Encode to base64
        let base64 = BASE64.encode(&data);

        // Determine media type from extension
        let media_type = match path.extension().and_then(|ext| ext.to_str()) {
            Some("png") => "image/png",
            Some("jpg") | Some("jpeg") => "image/jpeg",
            Some("gif") => "image/gif",
            Some("webp") => "image/webp",
            _ => "image/png", // Default to PNG
        }
        .to_string();

        debug!(
            path = %file_path,
            size_bytes = data.len(),
            media_type = %media_type,
            "Image encoded successfully"
        );

        Ok(ImageData { base64, media_type })
    }

    /// Parses the AI response into an AnalysisResult
    fn parse_response(response_text: &str) -> Result<AnalysisResult, AnalysisError> {
        // Try to extract JSON from the response
        let json_text = Self::extract_json(response_text);

        #[derive(Deserialize)]
        struct RawAnalysisResult {
            application_name: String,
            activity_description: String,
            category: String,
        }

        let raw: RawAnalysisResult = serde_json::from_str(&json_text).map_err(|e| {
            error!(
                error = %e,
                response = %response_text,
                "Failed to parse AI response as JSON"
            );
            AnalysisError::ParseError(format!("Invalid JSON response: {}", e))
        })?;

        Ok(AnalysisResult {
            application_name: raw.application_name,
            activity_description: raw.activity_description,
            category: ActivityCategory::from_str(&raw.category),
        })
    }

    /// Extracts JSON from a response that might contain additional text
    fn extract_json(text: &str) -> String {
        // Find the first { and last }
        let start = text.find('{');
        let end = text.rfind('}');

        match (start, end) {
            (Some(s), Some(e)) if s < e => text[s..=e].to_string(),
            _ => text.to_string(),
        }
    }
}

#[async_trait]
impl<P: AIProviderPort + 'static> ActivityAnalyzerPort for AIActivityAnalyzer<P> {
    async fn analyze(&self, capture: CaptureForAnalysis) -> Result<AnalysisResult, AnalysisError> {
        debug!(
            capture_id = capture.id,
            file_path = %capture.file_path,
            "Starting activity analysis"
        );

        // Read and encode the image
        let image_data = Self::read_image_as_base64(&capture.file_path).await?;

        // Build the prompt
        let prompt = MultimodalPrompt {
            system_message: ACTIVITY_ANALYSIS_SYSTEM_PROMPT.to_string(),
            user_message: UserMessage {
                text: ACTIVITY_ANALYSIS_USER_PROMPT.to_string(),
                images: vec![image_data],
            },
        };

        // Send to AI provider
        let response = self
            .ai_provider
            .generate_suggestion(prompt)
            .await
            .map_err(|e| {
                error!(error = %e, capture_id = capture.id, "AI provider returned error");
                AnalysisError::AIError(e.to_string())
            })?;

        // Parse the response
        let result = Self::parse_response(&response.text)?;

        debug!(
            capture_id = capture.id,
            application = %result.application_name,
            category = %result.category,
            "Activity analysis completed"
        );

        Ok(result)
    }

    async fn analyze_batch(
        &self,
        captures: Vec<CaptureForAnalysis>,
    ) -> Vec<Result<(i64, AnalysisResult), AnalysisError>> {
        let mut results = Vec::with_capacity(captures.len());

        for capture in captures {
            let capture_id = capture.id;
            let result = self.analyze(capture).await;

            match result {
                Ok(analysis) => {
                    results.push(Ok((capture_id, analysis)));
                }
                Err(e) => {
                    warn!(
                        capture_id = capture_id,
                        error = %e,
                        "Failed to analyze capture, will retry in next batch"
                    );
                    results.push(Err(e));
                }
            }
        }

        results
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use herald_core::ports::ai::{AIError, AIResponse};
    use std::sync::Mutex;

    /// Mock AI provider for testing
    struct MockAIProvider {
        response: Mutex<Option<String>>,
        should_fail: bool,
    }

    impl MockAIProvider {
        fn new(response: &str) -> Self {
            Self {
                response: Mutex::new(Some(response.to_string())),
                should_fail: false,
            }
        }

        fn failing() -> Self {
            Self {
                response: Mutex::new(None),
                should_fail: true,
            }
        }
    }

    #[async_trait]
    impl AIProviderPort for MockAIProvider {
        async fn generate_suggestion(
            &self,
            _prompt: MultimodalPrompt,
        ) -> Result<AIResponse, AIError> {
            if self.should_fail {
                return Err(AIError::ProviderError(
                    "mock".to_string(),
                    "Test error".to_string(),
                ));
            }

            let response = self.response.lock().unwrap();
            Ok(AIResponse {
                text: response.as_ref().unwrap().clone(),
            })
        }
    }

    #[test]
    fn test_parse_response_valid_json() {
        let json = r#"{
            "application_name": "VS Code",
            "activity_description": "Writing Rust code",
            "category": "coding"
        }"#;

        let result = AIActivityAnalyzer::<MockAIProvider>::parse_response(json);
        assert!(result.is_ok());

        let analysis = result.unwrap();
        assert_eq!(analysis.application_name, "VS Code");
        assert_eq!(analysis.activity_description, "Writing Rust code");
        assert_eq!(analysis.category, ActivityCategory::Coding);
    }

    #[test]
    fn test_parse_response_with_surrounding_text() {
        let response = r#"Here is my analysis:
{
    "application_name": "Chrome",
    "activity_description": "Reading documentation",
    "category": "documentation"
}
That's what I found."#;

        let result = AIActivityAnalyzer::<MockAIProvider>::parse_response(response);
        assert!(result.is_ok());

        let analysis = result.unwrap();
        assert_eq!(analysis.application_name, "Chrome");
        assert_eq!(analysis.category, ActivityCategory::Documentation);
    }

    #[test]
    fn test_parse_response_unknown_category() {
        let json = r#"{
            "application_name": "Steam",
            "activity_description": "Playing a game",
            "category": "gaming"
        }"#;

        let result = AIActivityAnalyzer::<MockAIProvider>::parse_response(json);
        assert!(result.is_ok());

        let analysis = result.unwrap();
        assert_eq!(
            analysis.category,
            ActivityCategory::Other("gaming".to_string())
        );
    }

    #[test]
    fn test_parse_response_invalid_json() {
        let invalid = "This is not JSON";
        let result = AIActivityAnalyzer::<MockAIProvider>::parse_response(invalid);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), AnalysisError::ParseError(_)));
    }

    #[test]
    fn test_extract_json() {
        let text = "Here is the result: {\"key\": \"value\"} and more text";
        let json = AIActivityAnalyzer::<MockAIProvider>::extract_json(text);
        assert_eq!(json, r#"{"key": "value"}"#);
    }

    #[test]
    fn test_extract_json_no_braces() {
        let text = "No JSON here";
        let json = AIActivityAnalyzer::<MockAIProvider>::extract_json(text);
        assert_eq!(json, text);
    }

    #[tokio::test]
    async fn test_analyze_with_mock_provider() {
        let mock_response = r#"{
            "application_name": "Terminal",
            "activity_description": "Running cargo tests",
            "category": "coding"
        }"#;

        let provider = Arc::new(MockAIProvider::new(mock_response));
        let analyzer = AIActivityAnalyzer::new(provider);

        // Create a temp file for testing
        let temp_dir = tempfile::TempDir::new().unwrap();
        let test_image_path = temp_dir.path().join("test.png");

        // Write a minimal valid PNG file (1x1 transparent pixel)
        let minimal_png: [u8; 67] = [
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48,
            0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00,
            0x00, 0x1F, 0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0A, 0x49, 0x44, 0x41, 0x54, 0x78,
            0x9C, 0x63, 0x00, 0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00,
            0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
        ];
        fs::write(&test_image_path, &minimal_png).await.unwrap();

        let capture = CaptureForAnalysis {
            id: 1,
            timestamp: 1704326400,
            file_path: test_image_path.to_string_lossy().to_string(),
        };

        let result = analyzer.analyze(capture).await;
        assert!(result.is_ok());

        let analysis = result.unwrap();
        assert_eq!(analysis.application_name, "Terminal");
        assert_eq!(analysis.category, ActivityCategory::Coding);
    }

    #[tokio::test]
    async fn test_analyze_file_not_found() {
        let provider = Arc::new(MockAIProvider::new("{}"));
        let analyzer = AIActivityAnalyzer::new(provider);

        let capture = CaptureForAnalysis {
            id: 1,
            timestamp: 1704326400,
            file_path: "/nonexistent/path/image.png".to_string(),
        };

        let result = analyzer.analyze(capture).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            AnalysisError::ImageReadError(_)
        ));
    }

    #[tokio::test]
    async fn test_analyze_ai_provider_error() {
        let provider = Arc::new(MockAIProvider::failing());
        let analyzer = AIActivityAnalyzer::new(provider);

        // Create a temp file
        let temp_dir = tempfile::TempDir::new().unwrap();
        let test_image_path = temp_dir.path().join("test.png");
        let minimal_png: [u8; 67] = [
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48,
            0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00,
            0x00, 0x1F, 0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0A, 0x49, 0x44, 0x41, 0x54, 0x78,
            0x9C, 0x63, 0x00, 0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00,
            0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
        ];
        fs::write(&test_image_path, &minimal_png).await.unwrap();

        let capture = CaptureForAnalysis {
            id: 1,
            timestamp: 1704326400,
            file_path: test_image_path.to_string_lossy().to_string(),
        };

        let result = analyzer.analyze(capture).await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), AnalysisError::AIError(_)));
    }

    #[tokio::test]
    async fn test_analyze_batch() {
        let mock_response = r#"{
            "application_name": "Test App",
            "activity_description": "Testing",
            "category": "coding"
        }"#;

        let provider = Arc::new(MockAIProvider::new(mock_response));
        let analyzer = AIActivityAnalyzer::new(provider);

        // Create temp files
        let temp_dir = tempfile::TempDir::new().unwrap();
        let minimal_png: [u8; 67] = [
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48,
            0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00,
            0x00, 0x1F, 0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0A, 0x49, 0x44, 0x41, 0x54, 0x78,
            0x9C, 0x63, 0x00, 0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00,
            0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
        ];

        let mut captures = Vec::new();
        for i in 0..3 {
            let path = temp_dir.path().join(format!("test_{}.png", i));
            fs::write(&path, &minimal_png).await.unwrap();
            captures.push(CaptureForAnalysis {
                id: i + 1,
                timestamp: 1704326400 + i * 60,
                file_path: path.to_string_lossy().to_string(),
            });
        }

        let results = analyzer.analyze_batch(captures).await;
        assert_eq!(results.len(), 3);

        for result in results {
            assert!(result.is_ok());
            let (id, analysis) = result.unwrap();
            assert!(id >= 1 && id <= 3);
            assert_eq!(analysis.application_name, "Test App");
        }
    }
}
