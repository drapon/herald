//! ScreenCaptureKit adapter for macOS
//!
//! Provides screen capture functionality using Apple's ScreenCaptureKit framework.
//! Requires macOS 14.0+ for SCScreenshotManager API.

use async_trait::async_trait;
use herald_core::ports::capture::{CaptureError, CapturePort, CapturedImage};
use png::{BitDepth, ColorType, Encoder};
use screencapturekit::prelude::*;
use screencapturekit::screenshot_manager::SCScreenshotManager;
use std::io::BufWriter;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{debug, info, warn};

/// Minimum macOS version required for ScreenCaptureKit (12.3)
const MIN_MACOS_MAJOR: u32 = 12;
const MIN_MACOS_MINOR: u32 = 3;

/// Minimum macOS version required for SCScreenshotManager (14.0)
const MIN_SCREENSHOT_MAJOR: u32 = 14;
const MIN_SCREENSHOT_MINOR: u32 = 0;

/// ScreenCaptureKit adapter for macOS screen capture
///
/// This adapter uses Apple's ScreenCaptureKit framework to capture
/// screenshots of the main display. Requires macOS 14.0+.
pub struct ScreenCaptureKitAdapter {
    /// Target width for captured images (default: display width)
    width: Option<u32>,
    /// Target height for captured images (default: display height)
    height: Option<u32>,
}

impl ScreenCaptureKitAdapter {
    /// Create a new ScreenCaptureKit adapter with default settings
    pub fn new() -> Self {
        Self {
            width: None,
            height: None,
        }
    }

    /// Create a new adapter with specific capture dimensions
    pub fn with_dimensions(width: u32, height: u32) -> Self {
        Self {
            width: Some(width),
            height: Some(height),
        }
    }

    /// Get the current macOS version
    fn get_macos_version() -> Result<(u32, u32, u32), CaptureError> {
        let output = Command::new("sw_vers")
            .arg("-productVersion")
            .output()
            .map_err(|e| {
                CaptureError::CaptureFailed(format!("Failed to get macOS version: {}", e))
            })?;

        let version_str = String::from_utf8_lossy(&output.stdout);
        let version_str = version_str.trim();

        let parts: Vec<&str> = version_str.split('.').collect();
        if parts.is_empty() {
            return Err(CaptureError::CaptureFailed(format!(
                "Invalid macOS version format: {}",
                version_str
            )));
        }

        let major: u32 = parts.first().and_then(|s| s.parse().ok()).unwrap_or(0);
        let minor: u32 = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
        let patch: u32 = parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);

        Ok((major, minor, patch))
    }

    /// Check if the current macOS version meets the minimum requirement
    fn check_macos_version(min_major: u32, min_minor: u32) -> Result<String, CaptureError> {
        let (major, minor, patch) = Self::get_macos_version()?;
        let version_str = format!("{}.{}.{}", major, minor, patch);

        if major < min_major || (major == min_major && minor < min_minor) {
            return Err(CaptureError::UnsupportedOS(version_str));
        }

        Ok(version_str)
    }

    /// Get the main display for capture
    fn get_main_display() -> Result<SCDisplay, CaptureError> {
        let content =
            SCShareableContent::get().map_err(|e: screencapturekit::error::SCError| {
                let msg = e.to_string();
                if msg.to_lowercase().contains("permission")
                    || msg.to_lowercase().contains("denied")
                    || msg.to_lowercase().contains("not authorized")
                {
                    CaptureError::PermissionDenied
                } else {
                    CaptureError::CaptureFailed(format!("Failed to get shareable content: {}", msg))
                }
            })?;

        content
            .displays()
            .into_iter()
            .next()
            .ok_or_else(|| CaptureError::CaptureFailed("No displays found".to_string()))
    }

    /// Encode image data as PNG
    fn encode_png(rgba_data: &[u8], width: u32, height: u32) -> Result<Vec<u8>, CaptureError> {
        let mut png_data = Vec::new();
        {
            let buf_writer = BufWriter::new(&mut png_data);
            let mut encoder = Encoder::new(buf_writer, width, height);
            encoder.set_color(ColorType::Rgba);
            encoder.set_depth(BitDepth::Eight);
            // Use default compression (level 6) for good balance of size/speed
            encoder.set_compression(png::Compression::Default);

            let mut writer = encoder
                .write_header()
                .map_err(|e| CaptureError::CaptureFailed(format!("PNG header error: {}", e)))?;

            writer
                .write_image_data(rgba_data)
                .map_err(|e| CaptureError::CaptureFailed(format!("PNG encoding error: {}", e)))?;
        }

        Ok(png_data)
    }
}

impl Default for ScreenCaptureKitAdapter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl CapturePort for ScreenCaptureKitAdapter {
    async fn capture_screen(&self) -> Result<CapturedImage, CaptureError> {
        // Run the blocking capture operation in a separate thread
        let width = self.width;
        let height = self.height;

        tokio::task::spawn_blocking(move || {
            // Check macOS version (14.0+ required for SCScreenshotManager)
            debug!("Checking macOS version...");
            let version = Self::check_macos_version(MIN_SCREENSHOT_MAJOR, MIN_SCREENSHOT_MINOR)?;
            debug!("macOS version: {}", version);

            debug!("Getting main display...");
            let display = Self::get_main_display()?;

            let display_width = display.width() as u32;
            let display_height = display.height() as u32;
            info!("Capturing display: {}x{}", display_width, display_height);

            // Create content filter for the display
            let filter = SCContentFilter::create()
                .with_display(&display)
                .with_excluding_windows(&[])
                .build();

            // Configure capture dimensions
            let capture_width = width.unwrap_or(display_width);
            let capture_height = height.unwrap_or(display_height);

            let config = SCStreamConfiguration::new()
                .with_width(capture_width)
                .with_height(capture_height)
                .with_shows_cursor(true);

            debug!(
                "Capturing screenshot at {}x{}...",
                capture_width, capture_height
            );

            // Capture the screenshot using SCScreenshotManager (macOS 14.0+)
            let image = SCScreenshotManager::capture_image(&filter, &config).map_err(
                |e: screencapturekit::error::SCError| {
                    let msg = e.to_string();
                    if msg.to_lowercase().contains("permission")
                        || msg.to_lowercase().contains("denied")
                    {
                        CaptureError::PermissionDenied
                    } else if msg.contains("version") || msg.contains("14.0") {
                        CaptureError::UnsupportedOS(
                            "macOS 14.0+ required for screenshots".to_string(),
                        )
                    } else {
                        CaptureError::CaptureFailed(format!("Screenshot capture failed: {}", msg))
                    }
                },
            )?;

            // Get image dimensions
            let img_width = image.width() as u32;
            let img_height = image.height() as u32;

            debug!("Converting to RGBA...");
            // Get RGBA pixel data
            let rgba_data = image.rgba_data().map_err(|e| {
                CaptureError::CaptureFailed(format!("Failed to get RGBA data: {}", e))
            })?;

            debug!("Encoding as PNG...");
            // Encode as PNG
            let png_data = Self::encode_png(&rgba_data, img_width, img_height)?;

            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);

            info!(
                "Screenshot captured: {}x{}, {} bytes PNG",
                img_width,
                img_height,
                png_data.len()
            );

            Ok(CapturedImage {
                data: png_data,
                width: img_width,
                height: img_height,
                timestamp,
            })
        })
        .await
        .map_err(|e| CaptureError::CaptureFailed(format!("Task join error: {}", e)))?
    }

    async fn check_permission(&self) -> Result<bool, CaptureError> {
        // Run the blocking permission check in a separate thread
        tokio::task::spawn_blocking(|| {
            // Check macOS version (12.3+ required for ScreenCaptureKit)
            debug!("Checking macOS version...");
            let version = Self::check_macos_version(MIN_MACOS_MAJOR, MIN_MACOS_MINOR)?;
            debug!("macOS version: {}", version);

            debug!("Checking screen recording permission...");

            // Try to get shareable content - this will fail if permission is denied
            match SCShareableContent::get() {
                Ok(content) => {
                    let displays: Vec<SCDisplay> = content.displays();
                    if displays.is_empty() {
                        warn!("No displays found - permission may be denied");
                        Ok(false)
                    } else {
                        info!(
                            "Screen recording permission granted ({} displays available)",
                            displays.len()
                        );
                        Ok(true)
                    }
                }
                Err(e) => {
                    let msg: String = e.to_string();
                    if msg.to_lowercase().contains("permission")
                        || msg.to_lowercase().contains("denied")
                        || msg.to_lowercase().contains("not authorized")
                    {
                        warn!("Screen recording permission denied");
                        Ok(false)
                    } else {
                        // Other error - might be OS version issue
                        Err(CaptureError::CaptureFailed(format!(
                            "Permission check failed: {}",
                            msg
                        )))
                    }
                }
            }
        })
        .await
        .map_err(|e| CaptureError::CaptureFailed(format!("Task join error: {}", e)))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_adapter_creation() {
        let adapter = ScreenCaptureKitAdapter::new();
        assert!(adapter.width.is_none());
        assert!(adapter.height.is_none());
    }

    #[test]
    fn test_adapter_with_dimensions() {
        let adapter = ScreenCaptureKitAdapter::with_dimensions(1920, 1080);
        assert_eq!(adapter.width, Some(1920));
        assert_eq!(adapter.height, Some(1080));
    }

    #[test]
    fn test_png_encoding() {
        // Create a small test image (2x2 red pixels)
        let rgba_data: Vec<u8> = vec![
            255, 0, 0, 255, // Red pixel
            255, 0, 0, 255, // Red pixel
            255, 0, 0, 255, // Red pixel
            255, 0, 0, 255, // Red pixel
        ];

        let png_data = ScreenCaptureKitAdapter::encode_png(&rgba_data, 2, 2);
        assert!(png_data.is_ok());

        let data = png_data.unwrap();
        // PNG magic bytes
        assert_eq!(&data[0..8], &[137, 80, 78, 71, 13, 10, 26, 10]);
    }

    #[test]
    fn test_default_impl() {
        let adapter = ScreenCaptureKitAdapter::default();
        assert!(adapter.width.is_none());
    }

    #[test]
    fn test_get_macos_version() {
        // This test only runs on macOS
        let result = ScreenCaptureKitAdapter::get_macos_version();
        assert!(result.is_ok());

        let (major, minor, _patch) = result.unwrap();
        // Should be at least macOS 10.x or higher
        assert!(major >= 10);
        // Minor version should be reasonable
        assert!(minor <= 99);
    }

    #[test]
    fn test_check_macos_version_current() {
        // Current macOS should pass for 12.3 requirement
        let result = ScreenCaptureKitAdapter::check_macos_version(MIN_MACOS_MAJOR, MIN_MACOS_MINOR);
        // This might fail on older macOS, which is expected behavior
        if let Ok(version) = result {
            assert!(!version.is_empty());
        }
    }
}
