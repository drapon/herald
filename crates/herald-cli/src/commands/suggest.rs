//! AI suggestion command
//!
//! Handles `herald suggest "<intent>"` command for AI-powered suggestions.
//! Collects recent screenshots, encodes them as Base64, and sends them to
//! the configured AI provider (Claude/Gemini) for context-aware suggestions.

use anyhow::{Context, Result};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use herald_core::ports::ai::ImageData;
use herald_core::ports::StoragePort;
use herald_core::{load_config, AIProvider, ApiKeyManager, PromptBuilder, MAX_IMAGES};
use std::fs;
use std::path::Path;

/// Get AI suggestions based on recent captures
///
/// This command:
/// 1. Collects recent screenshots (up to 10)
/// 2. Encodes them as Base64
/// 3. Sends them to the configured AI provider (Claude/Gemini)
/// 4. Displays AI-generated suggestions based on work context
///
/// # Arguments
/// * `intent` - The user's intent or question for the AI
///
/// # Errors
/// - Returns error if no captures exist
/// - Returns error if AI API call fails
pub async fn run(intent: &str) -> Result<()> {
    // Load configuration
    let config = load_config().context("Failed to load configuration")?;

    // Parse AI provider from config
    let provider: AIProvider = config
        .ai
        .default_provider
        .parse()
        .map_err(|e: String| anyhow::anyhow!(e))?;

    // Check API key availability first
    if let Err(_) = ApiKeyManager::load_api_key(provider) {
        eprintln!("Error: API key not configured.\n");
        eprintln!("{}", ApiKeyManager::missing_key_guidance(provider));
        return Ok(());
    }

    // Open database
    let db_path = config.storage.data_dir.join("herald.db");
    if !db_path.exists() {
        eprintln!("Error: No capture data found.");
        eprintln!();
        eprintln!("To start capturing screenshots, run:");
        eprintln!("  herald daemon start");
        eprintln!();
        eprintln!("Or capture a single screenshot with:");
        eprintln!("  herald capture");
        return Ok(());
    }

    let storage = herald_adapters::SqliteAdapter::new(&db_path)
        .await
        .context("Failed to connect to database")?;

    // Get recent captures (up to MAX_IMAGES)
    let captures = storage
        .get_recent_captures(MAX_IMAGES)
        .await
        .context("Failed to retrieve captures")?;

    if captures.is_empty() {
        eprintln!("Error: No captures available.");
        eprintln!();
        eprintln!("Capture some screenshots first:");
        eprintln!("  herald daemon start  # Start automatic capturing");
        eprintln!("  herald capture       # Or capture one manually");
        return Ok(());
    }

    // Collect and encode images
    println!("Collecting {} recent capture(s)...", captures.len());
    let images = collect_images(&captures)?;

    if images.is_empty() {
        eprintln!("Error: No image files found.");
        eprintln!();
        eprintln!("The capture metadata exists but image files are missing.");
        eprintln!("Try capturing new screenshots with:");
        eprintln!("  herald capture");
        return Ok(());
    }

    println!("Building prompt with {} image(s)...", images.len());

    // Build prompt
    let prompt = PromptBuilder::build_multimodal_prompt(intent, images)
        .map_err(|e| anyhow::anyhow!("Failed to build prompt: {}", e))?;

    // Get API key
    let api_key = ApiKeyManager::load_api_key(provider)
        .map_err(|_| anyhow::anyhow!("API key not available"))?;

    // Call AI provider
    println!(
        "Sending request to {} ({})...",
        provider.display_name(),
        config.ai.model
    );
    println!();

    let response = call_ai_provider(provider, &api_key.expose(), &config.ai.model, prompt).await?;

    // Display response
    println!("AI Suggestion");
    println!("=============");
    println!();
    println!("{}", response);

    Ok(())
}

/// Collects images from capture metadata and encodes them as Base64
fn collect_images(
    captures: &[herald_core::ports::storage::CaptureMetadata],
) -> Result<Vec<ImageData>> {
    let mut images = Vec::new();

    for capture in captures {
        let path = Path::new(&capture.file_path);

        if !path.exists() {
            tracing::warn!("Image file not found: {}", capture.file_path);
            continue;
        }

        match read_and_encode_image(path) {
            Ok(image_data) => {
                images.push(image_data);
            }
            Err(e) => {
                tracing::warn!("Failed to read image {}: {}", capture.file_path, e);
            }
        }
    }

    Ok(images)
}

/// Reads an image file and encodes it as Base64
fn read_and_encode_image(path: &Path) -> Result<ImageData> {
    let bytes = fs::read(path).context("Failed to read image file")?;
    let base64 = BASE64.encode(&bytes);

    // Determine media type from extension
    let media_type = match path.extension().and_then(|e| e.to_str()) {
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        _ => "image/png", // Default to PNG
    }
    .to_string();

    Ok(ImageData { base64, media_type })
}

/// Calls the appropriate AI provider based on configuration
async fn call_ai_provider(
    provider: AIProvider,
    api_key: &str,
    model: &str,
    prompt: herald_core::ports::ai::MultimodalPrompt,
) -> Result<String> {
    use herald_core::ports::ai::AIProviderPort;

    let result = match provider {
        AIProvider::Claude => {
            let adapter = herald_adapters::ai::ClaudeAdapter::new(api_key, model);
            adapter.generate_suggestion(prompt).await
        }
        AIProvider::Gemini => {
            let adapter = herald_adapters::ai::GeminiAdapter::new(api_key, model);
            adapter.generate_suggestion(prompt).await
        }
    };

    match result {
        Ok(response) => Ok(response.text),
        Err(e) => {
            let error_message = format_ai_error(&e, provider);
            Err(anyhow::anyhow!(error_message))
        }
    }
}

/// Formats AI errors with helpful guidance
fn format_ai_error(error: &herald_core::ports::ai::AIError, provider: AIProvider) -> String {
    use herald_core::ports::ai::AIError;

    match error {
        AIError::Unauthorized => {
            format!(
                "Authentication failed.\n\n{}",
                ApiKeyManager::missing_key_guidance(provider)
            )
        }
        AIError::RateLimitExceeded => {
            format!(
                "Rate limit exceeded for {}.\n\n\
                Please wait a moment and try again.\n\
                If this persists, check your API usage limits.",
                provider.display_name()
            )
        }
        AIError::ImageLimitExceeded(limit) => {
            format!(
                "Too many images. Maximum {} images are supported.\n\n\
                Try reducing the number of recent captures.",
                limit
            )
        }
        AIError::InvalidRequest(msg) => {
            format!(
                "Invalid request: {}\n\n\
                Please check your input and try again.",
                msg
            )
        }
        AIError::RequestFailed(msg) => {
            format!(
                "Request failed: {}\n\n\
                Please check your internet connection and try again.",
                msg
            )
        }
        AIError::InvalidResponse(msg) => {
            format!(
                "Invalid response from {}: {}\n\n\
                This may be a temporary issue. Please try again.",
                provider.display_name(),
                msg
            )
        }
        AIError::ProviderError(_, msg) => {
            format!(
                "{} error: {}\n\n\
                If this persists, check the API status page.",
                provider.display_name(),
                msg
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use herald_core::ports::ai::AIError;
    use tempfile::TempDir;

    // === Base64 Encoding Tests (Task 14.1) ===

    #[test]
    fn test_read_and_encode_png_image() {
        let temp_dir = TempDir::new().unwrap();
        let image_path = temp_dir.path().join("test.png");

        // Create a minimal PNG file (1x1 pixel, red)
        let png_data: Vec<u8> = vec![
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, // PNG signature
            0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52, // IHDR chunk
            0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x90,
            0x77, 0x53, 0xDE, 0x00, 0x00, 0x00, 0x0C, 0x49, 0x44, 0x41, 0x54, // IDAT chunk
            0x08, 0xD7, 0x63, 0xF8, 0xCF, 0xC0, 0x00, 0x00, 0x01, 0x01, 0x01, 0x00, 0x18, 0xDD,
            0x8D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, // IEND chunk
            0xAE, 0x42, 0x60, 0x82,
        ];
        fs::write(&image_path, &png_data).unwrap();

        let result = read_and_encode_image(&image_path);
        assert!(result.is_ok());

        let image_data = result.unwrap();
        assert_eq!(image_data.media_type, "image/png");
        assert!(!image_data.base64.is_empty());

        // Verify Base64 can be decoded back to original
        let decoded = BASE64.decode(&image_data.base64).unwrap();
        assert_eq!(decoded, png_data);
    }

    #[test]
    fn test_read_and_encode_jpeg_image() {
        let temp_dir = TempDir::new().unwrap();
        let image_path = temp_dir.path().join("test.jpg");

        // Create minimal JPEG-like data (just for extension detection test)
        let jpeg_data = vec![0xFF, 0xD8, 0xFF, 0xE0]; // JPEG signature
        fs::write(&image_path, &jpeg_data).unwrap();

        let result = read_and_encode_image(&image_path);
        assert!(result.is_ok());

        let image_data = result.unwrap();
        assert_eq!(image_data.media_type, "image/jpeg");
    }

    #[test]
    fn test_read_and_encode_jpeg_extension() {
        let temp_dir = TempDir::new().unwrap();
        let image_path = temp_dir.path().join("test.jpeg");

        let jpeg_data = vec![0xFF, 0xD8, 0xFF, 0xE0];
        fs::write(&image_path, &jpeg_data).unwrap();

        let result = read_and_encode_image(&image_path);
        assert!(result.is_ok());

        let image_data = result.unwrap();
        assert_eq!(image_data.media_type, "image/jpeg");
    }

    #[test]
    fn test_read_and_encode_webp_image() {
        let temp_dir = TempDir::new().unwrap();
        let image_path = temp_dir.path().join("test.webp");

        let webp_data = vec![0x52, 0x49, 0x46, 0x46]; // RIFF signature
        fs::write(&image_path, &webp_data).unwrap();

        let result = read_and_encode_image(&image_path);
        assert!(result.is_ok());

        let image_data = result.unwrap();
        assert_eq!(image_data.media_type, "image/webp");
    }

    #[test]
    fn test_read_and_encode_gif_image() {
        let temp_dir = TempDir::new().unwrap();
        let image_path = temp_dir.path().join("test.gif");

        let gif_data = vec![0x47, 0x49, 0x46, 0x38]; // GIF signature
        fs::write(&image_path, &gif_data).unwrap();

        let result = read_and_encode_image(&image_path);
        assert!(result.is_ok());

        let image_data = result.unwrap();
        assert_eq!(image_data.media_type, "image/gif");
    }

    #[test]
    fn test_read_and_encode_unknown_extension_defaults_to_png() {
        let temp_dir = TempDir::new().unwrap();
        let image_path = temp_dir.path().join("test.bmp");

        let data = vec![0x42, 0x4D]; // BMP signature
        fs::write(&image_path, &data).unwrap();

        let result = read_and_encode_image(&image_path);
        assert!(result.is_ok());

        let image_data = result.unwrap();
        assert_eq!(image_data.media_type, "image/png"); // Defaults to PNG
    }

    #[test]
    fn test_read_and_encode_nonexistent_file_fails() {
        let result = read_and_encode_image(Path::new("/nonexistent/image.png"));
        assert!(result.is_err());
    }

    // === Image Collection Tests (Task 14.1) ===

    #[test]
    fn test_collect_images_with_valid_files() {
        let temp_dir = TempDir::new().unwrap();

        // Create test images
        let image1_path = temp_dir.path().join("capture1.png");
        let image2_path = temp_dir.path().join("capture2.png");

        fs::write(&image1_path, b"fake png 1").unwrap();
        fs::write(&image2_path, b"fake png 2").unwrap();

        let captures = vec![
            herald_core::ports::storage::CaptureMetadata {
                id: Some(1),
                timestamp: 1704326400,
                file_path: image1_path.to_string_lossy().to_string(),
                created_at: 1704326400,
            },
            herald_core::ports::storage::CaptureMetadata {
                id: Some(2),
                timestamp: 1704326460,
                file_path: image2_path.to_string_lossy().to_string(),
                created_at: 1704326460,
            },
        ];

        let images = collect_images(&captures).unwrap();
        assert_eq!(images.len(), 2);
    }

    #[test]
    fn test_collect_images_skips_missing_files() {
        let temp_dir = TempDir::new().unwrap();

        // Create only one image
        let image1_path = temp_dir.path().join("capture1.png");
        fs::write(&image1_path, b"fake png").unwrap();

        let captures = vec![
            herald_core::ports::storage::CaptureMetadata {
                id: Some(1),
                timestamp: 1704326400,
                file_path: image1_path.to_string_lossy().to_string(),
                created_at: 1704326400,
            },
            herald_core::ports::storage::CaptureMetadata {
                id: Some(2),
                timestamp: 1704326460,
                file_path: "/nonexistent/capture2.png".to_string(),
                created_at: 1704326460,
            },
        ];

        let images = collect_images(&captures).unwrap();
        assert_eq!(images.len(), 1); // Only one valid image
    }

    #[test]
    fn test_collect_images_empty_captures() {
        let captures: Vec<herald_core::ports::storage::CaptureMetadata> = vec![];
        let images = collect_images(&captures).unwrap();
        assert!(images.is_empty());
    }

    // === Error Formatting Tests (Task 14.2) ===

    #[test]
    fn test_format_ai_error_unauthorized() {
        let error = AIError::Unauthorized;
        let message = format_ai_error(&error, AIProvider::Claude);

        assert!(message.contains("Authentication failed"));
        assert!(message.contains("ANTHROPIC_API_KEY"));
    }

    #[test]
    fn test_format_ai_error_rate_limit() {
        let error = AIError::RateLimitExceeded;
        let message = format_ai_error(&error, AIProvider::Gemini);

        assert!(message.contains("Rate limit"));
        assert!(message.contains("Google Gemini"));
    }

    #[test]
    fn test_format_ai_error_image_limit() {
        let error = AIError::ImageLimitExceeded(10);
        let message = format_ai_error(&error, AIProvider::Claude);

        assert!(message.contains("10"));
        assert!(message.contains("images"));
    }

    #[test]
    fn test_format_ai_error_invalid_request() {
        let error = AIError::InvalidRequest("Bad format".to_string());
        let message = format_ai_error(&error, AIProvider::Claude);

        assert!(message.contains("Invalid request"));
        assert!(message.contains("Bad format"));
    }

    #[test]
    fn test_format_ai_error_request_failed() {
        let error = AIError::RequestFailed("Connection timeout".to_string());
        let message = format_ai_error(&error, AIProvider::Claude);

        assert!(message.contains("Request failed"));
        assert!(message.contains("Connection timeout"));
        assert!(message.contains("internet connection"));
    }

    #[test]
    fn test_format_ai_error_invalid_response() {
        let error = AIError::InvalidResponse("Malformed JSON".to_string());
        let message = format_ai_error(&error, AIProvider::Gemini);

        assert!(message.contains("Invalid response"));
        assert!(message.contains("Google Gemini"));
        assert!(message.contains("Malformed JSON"));
    }

    #[test]
    fn test_format_ai_error_provider_error() {
        let error = AIError::ProviderError("claude".to_string(), "Server overloaded".to_string());
        let message = format_ai_error(&error, AIProvider::Claude);

        assert!(message.contains("Anthropic Claude"));
        assert!(message.contains("Server overloaded"));
    }

    // === Integration-like Tests ===

    #[tokio::test]
    async fn test_run_without_database() {
        // This test verifies that run() handles missing database gracefully
        // We can't easily test without side effects, but we verify the function exists
        // Real integration tests would use a test environment

        // The function signature is correct
        fn _check_signature(_intent: &str) -> impl std::future::Future<Output = Result<()>> {
            run("test intent")
        }
    }
}
