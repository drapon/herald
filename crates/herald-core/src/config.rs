//! Configuration management for Herald
//!
//! Handles loading and validation of TOML configuration files.

use crate::error::ConfigError;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// Main configuration structure for Herald
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    /// Capture-related settings
    #[serde(default)]
    pub capture: CaptureConfig,

    /// Storage-related settings
    #[serde(default)]
    pub storage: StorageConfig,

    /// AI provider settings
    #[serde(default)]
    pub ai: AiConfig,

    /// Activity analysis settings
    #[serde(default)]
    pub activity: ActivityConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            capture: CaptureConfig::default(),
            storage: StorageConfig::default(),
            ai: AiConfig::default(),
            activity: ActivityConfig::default(),
        }
    }
}

/// Display selection for capture
///
/// Controls which displays are captured and how they are saved.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(untagged)]
pub enum DisplaySelection {
    /// Single display by index (e.g., `display = 0`)
    Single(u32),
    /// Multiple displays by indices, saved as separate files (e.g., `display = [0, 1]`)
    Multiple(Vec<u32>),
}

/// Capture configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CaptureConfig {
    /// Interval between automatic captures in seconds (default: 60)
    #[serde(default = "default_interval_seconds")]
    pub interval_seconds: u64,

    /// PNG compression level 0-9 (default: 6)
    #[serde(default = "default_image_quality")]
    pub image_quality: u8,

    /// Display selection for capture
    ///
    /// - `None` (default): Capture all displays and combine them horizontally into one image
    /// - `Some(Single(n))`: Capture only display n
    /// - `Some(Multiple([n, m, ...]))`: Capture specified displays as separate files
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<DisplaySelection>,
}

impl Default for CaptureConfig {
    fn default() -> Self {
        Self {
            interval_seconds: default_interval_seconds(),
            image_quality: default_image_quality(),
            display: None,
        }
    }
}

/// Storage configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct StorageConfig {
    /// Base data directory (default: ~/.herald/)
    #[serde(
        default = "default_data_dir",
        deserialize_with = "deserialize_data_dir"
    )]
    pub data_dir: PathBuf,

    /// Data retention period in seconds (default: 86400 = 24 hours)
    #[serde(default = "default_retention_seconds")]
    pub retention_seconds: u64,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            data_dir: default_data_dir(),
            retention_seconds: default_retention_seconds(),
        }
    }
}

/// AI provider configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AiConfig {
    /// Default AI provider: "claude" or "gemini"
    #[serde(default = "default_provider")]
    pub default_provider: String,

    /// Model name to use
    #[serde(default = "default_model")]
    pub model: String,

    /// API key for the AI provider
    /// If not set, falls back to environment variables (ANTHROPIC_API_KEY or GEMINI_API_KEY)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
}

impl Default for AiConfig {
    fn default() -> Self {
        Self {
            default_provider: default_provider(),
            model: default_model(),
            api_key: None,
        }
    }
}

/// Activity analysis configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ActivityConfig {
    /// Enable automatic activity analysis (default: false)
    #[serde(default)]
    pub enabled: bool,

    /// Analysis interval in seconds (default: 300 = 5 minutes)
    #[serde(default = "default_analyze_interval")]
    pub analyze_interval_seconds: u64,
}

impl Default for ActivityConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            analyze_interval_seconds: default_analyze_interval(),
        }
    }
}

// Default value functions
fn default_interval_seconds() -> u64 {
    60
}

fn default_image_quality() -> u8 {
    6
}

fn default_data_dir() -> PathBuf {
    dirs::home_dir()
        .expect("ホームディレクトリを取得できませんでした")
        .join(".herald")
}

fn default_retention_seconds() -> u64 {
    86400 // 24 hours
}

fn default_provider() -> String {
    "claude".to_string()
}

fn default_model() -> String {
    "claude-3-5-sonnet-20241022".to_string()
}

fn default_analyze_interval() -> u64 {
    300 // 5 minutes
}

/// Expands tilde (~) in a path to the home directory
///
/// # Arguments
/// * `path` - Path that may contain a leading tilde
///
/// # Returns
/// * Expanded PathBuf with tilde replaced by home directory
fn expand_tilde(path: &Path) -> PathBuf {
    let path_str = path.to_string_lossy();
    if path_str.starts_with("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(path_str.strip_prefix("~/").unwrap());
        }
    } else if path_str == "~" {
        if let Some(home) = dirs::home_dir() {
            return home;
        }
    }
    path.to_path_buf()
}

/// Custom deserializer for data_dir that expands tilde
fn deserialize_data_dir<'de, D>(deserializer: D) -> Result<PathBuf, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let path_str = String::deserialize(deserializer)?;
    let path = PathBuf::from(path_str);
    Ok(expand_tilde(&path))
}

impl Config {
    /// Validates the configuration values
    ///
    /// # Errors
    /// Returns `ConfigError::InvalidValue` if:
    /// - `capture.interval_seconds` is 0
    /// - `capture.image_quality` is greater than 9
    /// - `ai.default_provider` is not "claude" or "gemini"
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.capture.interval_seconds == 0 {
            return Err(ConfigError::InvalidValue(
                "interval_seconds must be > 0".to_string(),
            ));
        }

        if self.capture.image_quality > 9 {
            return Err(ConfigError::InvalidValue(
                "image_quality must be between 0 and 9".to_string(),
            ));
        }

        if !["claude", "gemini"].contains(&self.ai.default_provider.as_str()) {
            return Err(ConfigError::InvalidValue(
                "default_provider must be 'claude' or 'gemini'".to_string(),
            ));
        }

        if self.activity.enabled && self.activity.analyze_interval_seconds == 0 {
            return Err(ConfigError::InvalidValue(
                "analyze_interval_seconds must be > 0 when activity is enabled".to_string(),
            ));
        }

        Ok(())
    }
}

/// Returns the default configuration file path (`~/.herald/config.toml`)
pub fn get_default_config_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".herald")
        .join("config.toml")
}

/// Loads configuration from the specified path
///
/// If the file doesn't exist, creates a default configuration file.
/// If the file is invalid or contains invalid values, returns default configuration.
///
/// # Arguments
/// * `path` - Path to the configuration file
///
/// # Returns
/// * `Ok(Config)` - Successfully loaded or default configuration
/// * `Err(ConfigError)` - Only for IO errors during file creation
pub fn load_config_from_path(path: &Path) -> Result<Config, ConfigError> {
    if !path.exists() {
        // Create parent directories if needed
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        // Generate default config and write to file
        let default_config = Config::default();
        let toml_str = toml::to_string_pretty(&default_config)
            .map_err(|e| ConfigError::ParseError(e.to_string()))?;
        fs::write(path, &toml_str)?;

        tracing::info!("Created default configuration file at {:?}", path);
        return Ok(default_config);
    }

    // Read existing file
    let content = fs::read_to_string(path)?;

    // Parse TOML
    let config: Config = match toml::from_str(&content) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(
                "Failed to parse configuration file {:?}: {}. Using default configuration.",
                path,
                e
            );
            return Ok(Config::default());
        }
    };

    // Validate configuration
    if let Err(e) = config.validate() {
        tracing::warn!(
            "Invalid configuration in {:?}: {}. Using default configuration.",
            path,
            e
        );
        return Ok(Config::default());
    }

    Ok(config)
}

/// Loads configuration from the default path (`~/.herald/config.toml`)
///
/// Convenience wrapper around `load_config_from_path`.
pub fn load_config() -> Result<Config, ConfigError> {
    load_config_from_path(&get_default_config_path())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert_eq!(config.capture.interval_seconds, 60);
        assert_eq!(config.capture.image_quality, 6);
        assert_eq!(config.storage.retention_seconds, 86400);
        assert_eq!(config.ai.default_provider, "claude");
    }

    #[test]
    fn test_config_serialization() {
        let config = Config::default();
        let toml_str = toml::to_string(&config).expect("Failed to serialize");
        assert!(toml_str.contains("interval_seconds"));
        assert!(toml_str.contains("retention_seconds"));
    }

    #[test]
    fn test_config_deserialization() {
        let toml_str = r#"
[capture]
interval_seconds = 120
image_quality = 9

[storage]
retention_seconds = 172800

[ai]
default_provider = "gemini"
model = "gemini-2.0-flash"
"#;
        let config: Config = toml::from_str(toml_str).expect("Failed to parse");
        assert_eq!(config.capture.interval_seconds, 120);
        assert_eq!(config.capture.image_quality, 9);
        assert_eq!(config.storage.retention_seconds, 172800);
        assert_eq!(config.ai.default_provider, "gemini");
    }

    #[test]
    fn test_partial_config_uses_defaults() {
        let toml_str = r#"
[capture]
interval_seconds = 30
"#;
        let config: Config = toml::from_str(toml_str).expect("Failed to parse");
        assert_eq!(config.capture.interval_seconds, 30);
        // Other values should use defaults
        assert_eq!(config.capture.image_quality, 6);
        assert_eq!(config.storage.retention_seconds, 86400);
        assert_eq!(config.ai.default_provider, "claude");
    }

    // === Validation Tests (Task 2.2) ===

    #[test]
    fn test_validate_valid_config() {
        let config = Config::default();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_validate_interval_zero_fails() {
        let mut config = Config::default();
        config.capture.interval_seconds = 0;
        let result = config.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("interval_seconds"));
    }

    #[test]
    fn test_validate_invalid_provider_fails() {
        let mut config = Config::default();
        config.ai.default_provider = "invalid_provider".to_string();
        let result = config.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("default_provider"));
    }

    #[test]
    fn test_validate_valid_providers() {
        let mut config = Config::default();

        config.ai.default_provider = "claude".to_string();
        assert!(config.validate().is_ok());

        config.ai.default_provider = "gemini".to_string();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_validate_image_quality_range() {
        let mut config = Config::default();

        // Valid range 0-9
        config.capture.image_quality = 0;
        assert!(config.validate().is_ok());

        config.capture.image_quality = 9;
        assert!(config.validate().is_ok());

        // Invalid: > 9
        config.capture.image_quality = 10;
        let result = config.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("image_quality"));
    }

    // === Config Loading Tests (Task 2.2) ===

    #[test]
    fn test_load_config_creates_default_when_missing() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("config.toml");

        // File doesn't exist
        assert!(!config_path.exists());

        let config = load_config_from_path(&config_path).unwrap();

        // Default values should be used
        assert_eq!(config.capture.interval_seconds, 60);
        assert_eq!(config.ai.default_provider, "claude");

        // File should be created
        assert!(config_path.exists());

        // File content should be valid TOML
        let content = fs::read_to_string(&config_path).unwrap();
        assert!(content.contains("[capture]"));
        assert!(content.contains("interval_seconds"));
    }

    #[test]
    fn test_load_config_reads_existing_file() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("config.toml");

        // Create custom config file
        let custom_config = r#"
[capture]
interval_seconds = 120

[ai]
default_provider = "gemini"
"#;
        fs::write(&config_path, custom_config).unwrap();

        let config = load_config_from_path(&config_path).unwrap();

        assert_eq!(config.capture.interval_seconds, 120);
        assert_eq!(config.ai.default_provider, "gemini");
        // Defaults for unspecified values
        assert_eq!(config.storage.retention_seconds, 86400);
    }

    #[test]
    fn test_load_config_invalid_toml_returns_default() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("config.toml");

        // Create invalid TOML
        fs::write(&config_path, "this is not valid toml {{{").unwrap();

        // Should return default config with warning (not error)
        let config = load_config_from_path(&config_path).unwrap();
        assert_eq!(config.capture.interval_seconds, 60);
    }

    #[test]
    fn test_load_config_invalid_values_returns_default() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("config.toml");

        // Create config with invalid values
        let invalid_config = r#"
[capture]
interval_seconds = 0

[ai]
default_provider = "invalid"
"#;
        fs::write(&config_path, invalid_config).unwrap();

        // Should return default config when validation fails
        let config = load_config_from_path(&config_path).unwrap();
        assert_eq!(config.capture.interval_seconds, 60);
        assert_eq!(config.ai.default_provider, "claude");
    }

    #[test]
    fn test_get_default_config_path() {
        let path = get_default_config_path();
        assert!(path.ends_with("config.toml"));
        assert!(path.to_string_lossy().contains(".herald"));
    }

    #[test]
    fn test_tilde_expansion_in_data_dir() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("config.toml");

        // Create config with tilde in data_dir
        let config_with_tilde = r#"
[storage]
data_dir = "~/.herald"
"#;
        fs::write(&config_path, config_with_tilde).unwrap();

        let config = load_config_from_path(&config_path).unwrap();

        // data_dir should be expanded to home directory
        let home = dirs::home_dir().expect("Failed to get home directory");
        let expected_path = home.join(".herald");
        assert_eq!(config.storage.data_dir, expected_path);

        // Verify it's an absolute path, not a relative path with literal "~"
        assert!(config.storage.data_dir.is_absolute());
        assert!(!config.storage.data_dir.to_string_lossy().starts_with("~"));
    }

    #[test]
    fn test_tilde_expansion_with_subdirectories() {
        let toml_str = r#"
[storage]
data_dir = "~/my_custom/herald_data"
"#;
        let config: Config = toml::from_str(toml_str).expect("Failed to parse");

        let home = dirs::home_dir().expect("Failed to get home directory");
        let expected_path = home.join("my_custom/herald_data");
        assert_eq!(config.storage.data_dir, expected_path);
        assert!(config.storage.data_dir.is_absolute());
    }

    #[test]
    fn test_absolute_path_unchanged() {
        let toml_str = r#"
[storage]
data_dir = "/absolute/path/to/herald"
"#;
        let config: Config = toml::from_str(toml_str).expect("Failed to parse");

        assert_eq!(
            config.storage.data_dir,
            PathBuf::from("/absolute/path/to/herald")
        );
    }

    // === DisplaySelection Tests ===

    #[test]
    fn test_display_selection_default_is_none() {
        let config = Config::default();
        assert!(config.capture.display.is_none());
    }

    #[test]
    fn test_display_selection_single() {
        let toml_str = r#"
[capture]
display = 0
"#;
        let config: Config = toml::from_str(toml_str).expect("Failed to parse");
        assert_eq!(config.capture.display, Some(DisplaySelection::Single(0)));
    }

    #[test]
    fn test_display_selection_single_non_zero() {
        let toml_str = r#"
[capture]
display = 2
"#;
        let config: Config = toml::from_str(toml_str).expect("Failed to parse");
        assert_eq!(config.capture.display, Some(DisplaySelection::Single(2)));
    }

    #[test]
    fn test_display_selection_multiple() {
        let toml_str = r#"
[capture]
display = [0, 1]
"#;
        let config: Config = toml::from_str(toml_str).expect("Failed to parse");
        assert_eq!(
            config.capture.display,
            Some(DisplaySelection::Multiple(vec![0, 1]))
        );
    }

    #[test]
    fn test_display_selection_multiple_non_contiguous() {
        let toml_str = r#"
[capture]
display = [0, 2, 4]
"#;
        let config: Config = toml::from_str(toml_str).expect("Failed to parse");
        assert_eq!(
            config.capture.display,
            Some(DisplaySelection::Multiple(vec![0, 2, 4]))
        );
    }

    #[test]
    fn test_display_selection_omitted_uses_none() {
        let toml_str = r#"
[capture]
interval_seconds = 30
"#;
        let config: Config = toml::from_str(toml_str).expect("Failed to parse");
        assert!(config.capture.display.is_none());
    }

    #[test]
    fn test_display_selection_serialization_none_omitted() {
        let config = Config::default();
        let toml_str = toml::to_string(&config).expect("Failed to serialize");
        // display should not appear in output when None
        assert!(!toml_str.contains("display"));
    }

    #[test]
    fn test_display_selection_serialization_single() {
        let mut config = Config::default();
        config.capture.display = Some(DisplaySelection::Single(1));
        let toml_str = toml::to_string(&config).expect("Failed to serialize");
        assert!(toml_str.contains("display = 1"));
    }

    #[test]
    fn test_display_selection_serialization_multiple() {
        let mut config = Config::default();
        config.capture.display = Some(DisplaySelection::Multiple(vec![0, 2]));
        let toml_str = toml::to_string(&config).expect("Failed to serialize");
        assert!(toml_str.contains("display = [0, 2]"));
    }

    #[test]
    fn test_backward_compatibility_no_display_field() {
        // Old config files without display field should still work
        let toml_str = r#"
[capture]
interval_seconds = 60
image_quality = 6

[storage]
retention_seconds = 86400

[ai]
default_provider = "claude"
"#;
        let config: Config = toml::from_str(toml_str).expect("Failed to parse");
        assert_eq!(config.capture.interval_seconds, 60);
        assert_eq!(config.capture.image_quality, 6);
        assert!(config.capture.display.is_none());
    }

    // === ActivityConfig Tests ===

    #[test]
    fn test_activity_config_default() {
        let config = ActivityConfig::default();
        assert!(!config.enabled);
        assert_eq!(config.analyze_interval_seconds, 300);
    }

    #[test]
    fn test_activity_config_in_main_config() {
        let config = Config::default();
        assert!(!config.activity.enabled);
        assert_eq!(config.activity.analyze_interval_seconds, 300);
    }

    #[test]
    fn test_activity_config_deserialization() {
        let toml_str = r#"
[activity]
enabled = true
analyze_interval_seconds = 600
"#;
        let config: Config = toml::from_str(toml_str).expect("Failed to parse");
        assert!(config.activity.enabled);
        assert_eq!(config.activity.analyze_interval_seconds, 600);
    }

    #[test]
    fn test_activity_config_partial_uses_defaults() {
        let toml_str = r#"
[activity]
enabled = true
"#;
        let config: Config = toml::from_str(toml_str).expect("Failed to parse");
        assert!(config.activity.enabled);
        assert_eq!(config.activity.analyze_interval_seconds, 300);
    }

    #[test]
    fn test_activity_config_omitted_uses_defaults() {
        let toml_str = r#"
[capture]
interval_seconds = 30
"#;
        let config: Config = toml::from_str(toml_str).expect("Failed to parse");
        assert!(!config.activity.enabled);
        assert_eq!(config.activity.analyze_interval_seconds, 300);
    }

    #[test]
    fn test_activity_config_serialization() {
        let mut config = Config::default();
        config.activity.enabled = true;
        config.activity.analyze_interval_seconds = 600;
        let toml_str = toml::to_string(&config).expect("Failed to serialize");
        assert!(toml_str.contains("[activity]"));
        assert!(toml_str.contains("enabled = true"));
        assert!(toml_str.contains("analyze_interval_seconds = 600"));
    }

    #[test]
    fn test_activity_config_validation_enabled_with_zero_interval_fails() {
        let mut config = Config::default();
        config.activity.enabled = true;
        config.activity.analyze_interval_seconds = 0;
        let result = config.validate();
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("analyze_interval_seconds"));
    }

    #[test]
    fn test_activity_config_validation_disabled_with_zero_interval_ok() {
        let mut config = Config::default();
        config.activity.enabled = false;
        config.activity.analyze_interval_seconds = 0;
        // Should be OK because activity is disabled
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_activity_config_validation_valid() {
        let mut config = Config::default();
        config.activity.enabled = true;
        config.activity.analyze_interval_seconds = 300;
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_backward_compatibility_no_activity_section() {
        // Old config files without activity section should still work
        let toml_str = r#"
[capture]
interval_seconds = 60

[storage]
retention_seconds = 86400

[ai]
default_provider = "claude"
"#;
        let config: Config = toml::from_str(toml_str).expect("Failed to parse");
        // Activity should use defaults
        assert!(!config.activity.enabled);
        assert_eq!(config.activity.analyze_interval_seconds, 300);
    }
}
