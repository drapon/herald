//! Configuration management for Herald
//!
//! Handles loading and validation of TOML configuration files.

use crate::error::ConfigError;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;

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

    /// MTG (Meeting) recording settings
    #[serde(default)]
    pub mtg: MtgConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            capture: CaptureConfig::default(),
            storage: StorageConfig::default(),
            ai: AiConfig::default(),
            activity: ActivityConfig::default(),
            mtg: MtgConfig::default(),
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

/// MTG (Meeting) recording configuration
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct MtgConfig {
    /// List of meeting app process names to monitor
    #[serde(default = "default_watched_apps")]
    pub watched_apps: Vec<String>,

    /// Silence detection timeout in seconds for browser meeting fallback (default: 180 = 3 minutes)
    #[serde(default = "default_silence_timeout_seconds")]
    pub silence_timeout_seconds: u64,

    /// Maximum recording duration in seconds (default: 14400 = 4 hours)
    #[serde(default = "default_max_recording_seconds")]
    pub max_recording_seconds: u64,

    /// Capture interval during MTG mode in seconds (default: 10)
    #[serde(default = "default_mtg_capture_interval_seconds")]
    pub mtg_capture_interval_seconds: u64,
}

impl Default for MtgConfig {
    fn default() -> Self {
        Self {
            watched_apps: default_watched_apps(),
            silence_timeout_seconds: default_silence_timeout_seconds(),
            max_recording_seconds: default_max_recording_seconds(),
            mtg_capture_interval_seconds: default_mtg_capture_interval_seconds(),
        }
    }
}

impl MtgConfig {
    /// Validates the MTG configuration values
    ///
    /// # Errors
    /// Returns `ConfigError::InvalidValue` if:
    /// - `watched_apps` is empty
    /// - `silence_timeout_seconds` is 0
    /// - `max_recording_seconds` is 0
    /// - `mtg_capture_interval_seconds` is 0
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.watched_apps.is_empty() {
            return Err(ConfigError::InvalidValue(
                "mtg.watched_apps must not be empty".to_string(),
            ));
        }

        if self.silence_timeout_seconds == 0 {
            return Err(ConfigError::InvalidValue(
                "mtg.silence_timeout_seconds must be > 0".to_string(),
            ));
        }

        if self.max_recording_seconds == 0 {
            return Err(ConfigError::InvalidValue(
                "mtg.max_recording_seconds must be > 0".to_string(),
            ));
        }

        if self.mtg_capture_interval_seconds == 0 {
            return Err(ConfigError::InvalidValue(
                "mtg.mtg_capture_interval_seconds must be > 0".to_string(),
            ));
        }

        Ok(())
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

fn default_watched_apps() -> Vec<String> {
    vec![
        "zoom.us".to_string(),
        "Microsoft Teams".to_string(),
        "Discord".to_string(),
        "Slack".to_string(),
        "Webex".to_string(),
    ]
}

fn default_silence_timeout_seconds() -> u64 {
    180 // 3 minutes
}

fn default_max_recording_seconds() -> u64 {
    14400 // 4 hours
}

fn default_mtg_capture_interval_seconds() -> u64 {
    10
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

        // Validate MTG config
        self.mtg.validate()?;

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

/// Configuration provider with hot-reload support
///
/// This struct wraps a configuration and provides thread-safe access
/// with the ability to reload the configuration from file without
/// restarting the daemon.
#[derive(Debug)]
pub struct ConfigProvider {
    config: Arc<RwLock<Config>>,
    config_path: PathBuf,
}

impl ConfigProvider {
    /// Creates a new ConfigProvider with the given configuration and path
    ///
    /// # Arguments
    /// * `config` - Initial configuration
    /// * `config_path` - Path to the configuration file for reloading
    pub fn new(config: Config, config_path: PathBuf) -> Self {
        Self {
            config: Arc::new(RwLock::new(config)),
            config_path,
        }
    }

    /// Creates a new ConfigProvider from the default config path
    ///
    /// Loads configuration from `~/.herald/config.toml`
    pub fn from_default_path() -> Result<Self, ConfigError> {
        let path = get_default_config_path();
        let config = load_config_from_path(&path)?;
        Ok(Self::new(config, path))
    }

    /// Creates a new ConfigProvider from a specific path
    pub fn from_path(path: &Path) -> Result<Self, ConfigError> {
        let config = load_config_from_path(path)?;
        Ok(Self::new(config, path.to_path_buf()))
    }

    /// Returns a clone of the current configuration
    ///
    /// This is useful when you need an owned copy of the configuration
    pub async fn get(&self) -> Config {
        self.config.read().await.clone()
    }

    /// Returns the inner Arc<RwLock<Config>> for direct access
    ///
    /// This is useful when you need to share the config across multiple tasks
    pub fn shared(&self) -> Arc<RwLock<Config>> {
        Arc::clone(&self.config)
    }

    /// Reloads the configuration from the file
    ///
    /// If the file cannot be read or contains invalid configuration,
    /// the current configuration is kept and an error is returned.
    ///
    /// # Returns
    /// * `Ok(true)` - Configuration was reloaded successfully
    /// * `Ok(false)` - Configuration file hasn't changed (same content)
    /// * `Err(_)` - Failed to reload (current config is preserved)
    pub async fn reload(&self) -> Result<bool, ConfigError> {
        // Read the file
        let content = fs::read_to_string(&self.config_path)?;

        // Parse TOML
        let new_config: Config = toml::from_str(&content)
            .map_err(|e| ConfigError::ParseError(e.to_string()))?;

        // Validate configuration
        new_config.validate()?;

        // Check if config actually changed
        let current_config = self.config.read().await;
        let current_toml = toml::to_string(&*current_config)
            .map_err(|e| ConfigError::ParseError(e.to_string()))?;
        let new_toml = toml::to_string(&new_config)
            .map_err(|e| ConfigError::ParseError(e.to_string()))?;

        if current_toml == new_toml {
            tracing::debug!("Configuration unchanged, skipping reload");
            return Ok(false);
        }
        drop(current_config);

        // Update the configuration
        let mut config = self.config.write().await;
        *config = new_config;

        tracing::info!("Configuration reloaded from {:?}", self.config_path);
        Ok(true)
    }

    /// Returns the path to the configuration file
    pub fn config_path(&self) -> &Path {
        &self.config_path
    }
}

impl Clone for ConfigProvider {
    fn clone(&self) -> Self {
        Self {
            config: Arc::clone(&self.config),
            config_path: self.config_path.clone(),
        }
    }
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

    // === MtgConfig Tests ===

    #[test]
    fn test_mtg_config_default() {
        let config = MtgConfig::default();
        assert_eq!(
            config.watched_apps,
            vec![
                "zoom.us".to_string(),
                "Microsoft Teams".to_string(),
                "Discord".to_string(),
                "Slack".to_string(),
                "Webex".to_string(),
            ]
        );
        assert_eq!(config.silence_timeout_seconds, 180);
        assert_eq!(config.max_recording_seconds, 14400);
        assert_eq!(config.mtg_capture_interval_seconds, 10);
    }

    #[test]
    fn test_mtg_config_validation_valid() {
        let config = MtgConfig::default();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_mtg_config_validation_empty_watched_apps_fails() {
        let mut config = MtgConfig::default();
        config.watched_apps = vec![];
        let result = config.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("watched_apps"));
    }

    #[test]
    fn test_mtg_config_validation_zero_silence_timeout_fails() {
        let mut config = MtgConfig::default();
        config.silence_timeout_seconds = 0;
        let result = config.validate();
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("silence_timeout_seconds"));
    }

    #[test]
    fn test_mtg_config_validation_zero_max_recording_fails() {
        let mut config = MtgConfig::default();
        config.max_recording_seconds = 0;
        let result = config.validate();
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("max_recording_seconds"));
    }

    #[test]
    fn test_mtg_config_validation_zero_capture_interval_fails() {
        let mut config = MtgConfig::default();
        config.mtg_capture_interval_seconds = 0;
        let result = config.validate();
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("mtg_capture_interval_seconds"));
    }

    #[test]
    fn test_mtg_config_deserialization() {
        let toml_str = r#"
watched_apps = ["zoom.us", "Slack"]
silence_timeout_seconds = 300
max_recording_seconds = 7200
mtg_capture_interval_seconds = 5
"#;
        let config: MtgConfig = toml::from_str(toml_str).expect("Failed to parse");
        assert_eq!(config.watched_apps, vec!["zoom.us", "Slack"]);
        assert_eq!(config.silence_timeout_seconds, 300);
        assert_eq!(config.max_recording_seconds, 7200);
        assert_eq!(config.mtg_capture_interval_seconds, 5);
    }

    #[test]
    fn test_mtg_config_partial_uses_defaults() {
        let toml_str = r#"
watched_apps = ["zoom.us"]
"#;
        let config: MtgConfig = toml::from_str(toml_str).expect("Failed to parse");
        assert_eq!(config.watched_apps, vec!["zoom.us"]);
        // Other values should use defaults
        assert_eq!(config.silence_timeout_seconds, 180);
        assert_eq!(config.max_recording_seconds, 14400);
        assert_eq!(config.mtg_capture_interval_seconds, 10);
    }

    #[test]
    fn test_mtg_config_serialization() {
        let config = MtgConfig::default();
        let toml_str = toml::to_string(&config).expect("Failed to serialize");
        assert!(toml_str.contains("watched_apps"));
        assert!(toml_str.contains("zoom.us"));
        assert!(toml_str.contains("silence_timeout_seconds = 180"));
        assert!(toml_str.contains("max_recording_seconds = 14400"));
        assert!(toml_str.contains("mtg_capture_interval_seconds = 10"));
    }

    // === Config with MTG Integration Tests ===

    #[test]
    fn test_config_includes_mtg_defaults() {
        let config = Config::default();
        assert_eq!(config.mtg.watched_apps.len(), 5);
        assert_eq!(config.mtg.silence_timeout_seconds, 180);
        assert_eq!(config.mtg.max_recording_seconds, 14400);
        assert_eq!(config.mtg.mtg_capture_interval_seconds, 10);
    }

    #[test]
    fn test_config_mtg_section_deserialization() {
        let toml_str = r#"
[capture]
interval_seconds = 60

[mtg]
watched_apps = ["zoom.us", "Teams"]
silence_timeout_seconds = 300
max_recording_seconds = 7200
mtg_capture_interval_seconds = 5
"#;
        let config: Config = toml::from_str(toml_str).expect("Failed to parse");
        assert_eq!(config.capture.interval_seconds, 60);
        assert_eq!(config.mtg.watched_apps, vec!["zoom.us", "Teams"]);
        assert_eq!(config.mtg.silence_timeout_seconds, 300);
        assert_eq!(config.mtg.max_recording_seconds, 7200);
        assert_eq!(config.mtg.mtg_capture_interval_seconds, 5);
    }

    #[test]
    fn test_config_mtg_section_partial_uses_defaults() {
        let toml_str = r#"
[mtg]
watched_apps = ["zoom.us"]
"#;
        let config: Config = toml::from_str(toml_str).expect("Failed to parse");
        assert_eq!(config.mtg.watched_apps, vec!["zoom.us"]);
        assert_eq!(config.mtg.silence_timeout_seconds, 180);
        assert_eq!(config.mtg.max_recording_seconds, 14400);
        assert_eq!(config.mtg.mtg_capture_interval_seconds, 10);
    }

    #[test]
    fn test_config_without_mtg_section_uses_defaults() {
        let toml_str = r#"
[capture]
interval_seconds = 60

[ai]
default_provider = "claude"
"#;
        let config: Config = toml::from_str(toml_str).expect("Failed to parse");
        // MTG should use all defaults
        assert_eq!(config.mtg.watched_apps.len(), 5);
        assert!(config.mtg.watched_apps.contains(&"zoom.us".to_string()));
        assert_eq!(config.mtg.silence_timeout_seconds, 180);
    }

    #[test]
    fn test_config_validation_includes_mtg_validation() {
        let mut config = Config::default();
        config.mtg.watched_apps = vec![]; // Invalid: empty
        let result = config.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("watched_apps"));
    }

    #[test]
    fn test_config_validation_mtg_zero_values_fail() {
        let mut config = Config::default();
        config.mtg.silence_timeout_seconds = 0;
        assert!(config.validate().is_err());

        let mut config = Config::default();
        config.mtg.max_recording_seconds = 0;
        assert!(config.validate().is_err());

        let mut config = Config::default();
        config.mtg.mtg_capture_interval_seconds = 0;
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_config_mtg_uses_storage_data_dir() {
        // Verify that storage.data_dir is accessible for MTG recordings
        let toml_str = r#"
[storage]
data_dir = "/custom/data/path"

[mtg]
watched_apps = ["zoom.us"]
"#;
        let config: Config = toml::from_str(toml_str).expect("Failed to parse");
        assert_eq!(config.storage.data_dir, PathBuf::from("/custom/data/path"));
        // MTG recordings should use storage.data_dir
        assert_eq!(config.mtg.watched_apps, vec!["zoom.us"]);
    }

    #[test]
    fn test_load_config_with_invalid_mtg_returns_default() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("config.toml");

        // Create config with invalid MTG values
        let invalid_config = r#"
[mtg]
watched_apps = []
"#;
        fs::write(&config_path, invalid_config).unwrap();

        // Should return default config when validation fails
        let config = load_config_from_path(&config_path).unwrap();
        // Should have default watched_apps since validation failed
        assert_eq!(config.mtg.watched_apps.len(), 5);
    }

    // === ConfigProvider Tests ===

    #[tokio::test]
    async fn test_config_provider_new() {
        let config = Config::default();
        let provider = ConfigProvider::new(config.clone(), PathBuf::from("/test/config.toml"));

        let retrieved = provider.get().await;
        assert_eq!(retrieved.capture.interval_seconds, config.capture.interval_seconds);
        assert_eq!(provider.config_path(), Path::new("/test/config.toml"));
    }

    #[tokio::test]
    async fn test_config_provider_from_path() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("config.toml");

        let custom_config = r#"
[capture]
interval_seconds = 120

[mtg]
watched_apps = ["zoom.us", "Teams"]
"#;
        fs::write(&config_path, custom_config).unwrap();

        let provider = ConfigProvider::from_path(&config_path).unwrap();
        let config = provider.get().await;

        assert_eq!(config.capture.interval_seconds, 120);
        assert_eq!(config.mtg.watched_apps, vec!["zoom.us", "Teams"]);
    }

    #[tokio::test]
    async fn test_config_provider_reload_success() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("config.toml");

        // Initial config
        let initial_config = r#"
[capture]
interval_seconds = 60
"#;
        fs::write(&config_path, initial_config).unwrap();

        let provider = ConfigProvider::from_path(&config_path).unwrap();
        assert_eq!(provider.get().await.capture.interval_seconds, 60);

        // Update config file
        let updated_config = r#"
[capture]
interval_seconds = 120
"#;
        fs::write(&config_path, updated_config).unwrap();

        // Reload
        let result = provider.reload().await;
        assert!(result.is_ok());
        assert!(result.unwrap()); // true = config changed

        // Verify new config
        assert_eq!(provider.get().await.capture.interval_seconds, 120);
    }

    #[tokio::test]
    async fn test_config_provider_reload_unchanged() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("config.toml");

        let config_content = r#"
[capture]
interval_seconds = 60
"#;
        fs::write(&config_path, config_content).unwrap();

        let provider = ConfigProvider::from_path(&config_path).unwrap();

        // Reload without changes
        let result = provider.reload().await;
        assert!(result.is_ok());
        assert!(!result.unwrap()); // false = config unchanged
    }

    #[tokio::test]
    async fn test_config_provider_reload_invalid_keeps_old_config() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("config.toml");

        // Initial valid config
        let initial_config = r#"
[capture]
interval_seconds = 60
"#;
        fs::write(&config_path, initial_config).unwrap();

        let provider = ConfigProvider::from_path(&config_path).unwrap();
        assert_eq!(provider.get().await.capture.interval_seconds, 60);

        // Update with invalid config
        let invalid_config = r#"
[capture]
interval_seconds = 0
"#;
        fs::write(&config_path, invalid_config).unwrap();

        // Reload should fail
        let result = provider.reload().await;
        assert!(result.is_err());

        // Old config should be preserved
        assert_eq!(provider.get().await.capture.interval_seconds, 60);
    }

    #[tokio::test]
    async fn test_config_provider_reload_parse_error_keeps_old_config() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("config.toml");

        // Initial valid config
        let initial_config = r#"
[capture]
interval_seconds = 60
"#;
        fs::write(&config_path, initial_config).unwrap();

        let provider = ConfigProvider::from_path(&config_path).unwrap();

        // Update with invalid TOML
        fs::write(&config_path, "invalid { toml [[[").unwrap();

        // Reload should fail
        let result = provider.reload().await;
        assert!(result.is_err());

        // Old config should be preserved
        assert_eq!(provider.get().await.capture.interval_seconds, 60);
    }

    #[tokio::test]
    async fn test_config_provider_shared() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("config.toml");

        let config_content = r#"
[capture]
interval_seconds = 60
"#;
        fs::write(&config_path, config_content).unwrap();

        let provider = ConfigProvider::from_path(&config_path).unwrap();
        let shared = provider.shared();

        // Access via shared Arc
        let config = shared.read().await;
        assert_eq!(config.capture.interval_seconds, 60);
    }

    #[tokio::test]
    async fn test_config_provider_clone_shares_state() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("config.toml");

        let initial_config = r#"
[capture]
interval_seconds = 60
"#;
        fs::write(&config_path, initial_config).unwrap();

        let provider1 = ConfigProvider::from_path(&config_path).unwrap();
        let provider2 = provider1.clone();

        // Update config file
        let updated_config = r#"
[capture]
interval_seconds = 120
"#;
        fs::write(&config_path, updated_config).unwrap();

        // Reload via provider1
        provider1.reload().await.unwrap();

        // provider2 should see the updated config (shared state)
        assert_eq!(provider2.get().await.capture.interval_seconds, 120);
    }

    #[tokio::test]
    async fn test_config_provider_reload_mtg_config() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("config.toml");

        // Initial config with default MTG
        let initial_config = r#"
[mtg]
watched_apps = ["zoom.us"]
silence_timeout_seconds = 180
"#;
        fs::write(&config_path, initial_config).unwrap();

        let provider = ConfigProvider::from_path(&config_path).unwrap();
        assert_eq!(provider.get().await.mtg.silence_timeout_seconds, 180);

        // Update MTG config
        let updated_config = r#"
[mtg]
watched_apps = ["zoom.us", "Teams"]
silence_timeout_seconds = 300
"#;
        fs::write(&config_path, updated_config).unwrap();

        // Reload
        provider.reload().await.unwrap();

        let config = provider.get().await;
        assert_eq!(config.mtg.watched_apps, vec!["zoom.us", "Teams"]);
        assert_eq!(config.mtg.silence_timeout_seconds, 300);
    }
}
