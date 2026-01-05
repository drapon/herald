//! API key management for AI providers
//!
//! Handles secure loading and validation of API keys from environment variables.

use crate::ports::ai::AIError;
use std::env;
use std::fmt;

/// Environment variable name for Anthropic API key
pub const ANTHROPIC_API_KEY_ENV: &str = "ANTHROPIC_API_KEY";

/// Environment variable name for Google AI API key
pub const GOOGLE_AI_API_KEY_ENV: &str = "GOOGLE_AI_API_KEY";

/// A wrapper for API keys that prevents accidental logging
///
/// The `Debug` and `Display` implementations mask the actual key value
/// to prevent sensitive data from appearing in logs.
#[derive(Clone)]
pub struct SecretApiKey {
    key: String,
}

impl SecretApiKey {
    /// Creates a new SecretApiKey from a string
    ///
    /// # Arguments
    /// * `key` - The API key string
    ///
    /// # Returns
    /// * `Some(SecretApiKey)` if the key is non-empty
    /// * `None` if the key is empty or whitespace-only
    pub fn new(key: String) -> Option<Self> {
        let trimmed = key.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(Self {
                key: trimmed.to_string(),
            })
        }
    }

    /// Returns the actual API key value
    ///
    /// Use this only when actually making API calls.
    /// Never log the returned value.
    pub fn expose(&self) -> &str {
        &self.key
    }
}

// Prevent API key from appearing in debug output
impl fmt::Debug for SecretApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SecretApiKey")
            .field("key", &"[REDACTED]")
            .finish()
    }
}

// Prevent API key from appearing in display output
impl fmt::Display for SecretApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[REDACTED API KEY]")
    }
}

/// AI provider types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AIProvider {
    /// Anthropic Claude
    Claude,
    /// Google Gemini
    Gemini,
}

impl AIProvider {
    /// Returns the environment variable name for this provider's API key
    pub fn env_var_name(&self) -> &'static str {
        match self {
            AIProvider::Claude => ANTHROPIC_API_KEY_ENV,
            AIProvider::Gemini => GOOGLE_AI_API_KEY_ENV,
        }
    }

    /// Returns a human-readable name for the provider
    pub fn display_name(&self) -> &'static str {
        match self {
            AIProvider::Claude => "Anthropic Claude",
            AIProvider::Gemini => "Google Gemini",
        }
    }
}

impl std::str::FromStr for AIProvider {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "claude" | "anthropic" => Ok(AIProvider::Claude),
            "gemini" | "google" => Ok(AIProvider::Gemini),
            _ => Err(format!("Unknown AI provider: {}", s)),
        }
    }
}

/// Manages API keys for AI providers
pub struct ApiKeyManager;

impl ApiKeyManager {
    /// Loads an API key from environment variable for the specified provider
    ///
    /// # Arguments
    /// * `provider` - The AI provider to load the key for
    ///
    /// # Returns
    /// * `Ok(SecretApiKey)` - The API key wrapped in SecretApiKey
    /// * `Err(AIError::Unauthorized)` - If the key is not set or empty
    ///
    /// # Example
    /// ```no_run
    /// use herald_core::api_key::{ApiKeyManager, AIProvider};
    ///
    /// let key = ApiKeyManager::load_api_key(AIProvider::Claude)
    ///     .expect("API key not set");
    /// ```
    pub fn load_api_key(provider: AIProvider) -> Result<SecretApiKey, AIError> {
        let env_var = provider.env_var_name();

        match env::var(env_var) {
            Ok(key) => SecretApiKey::new(key).ok_or_else(|| AIError::Unauthorized),
            Err(_) => Err(AIError::Unauthorized),
        }
    }

    /// Generates a helpful error message when an API key is missing
    ///
    /// # Arguments
    /// * `provider` - The AI provider that needs configuration
    ///
    /// # Returns
    /// A user-friendly error message with setup instructions
    pub fn missing_key_guidance(provider: AIProvider) -> String {
        let env_var = provider.env_var_name();
        let provider_name = provider.display_name();

        format!(
            r#"{provider_name} API key is not configured.

To set up your API key:

1. Create a .env file in your project root:
   {env_var}=your-api-key-here

2. Or set the environment variable directly:
   export {env_var}=your-api-key-here

For more information:
- Claude: https://console.anthropic.com/
- Gemini: https://aistudio.google.com/"#
        )
    }

    /// Checks if an API key is available for the specified provider
    ///
    /// This is a non-destructive check that doesn't expose the key.
    ///
    /// # Arguments
    /// * `provider` - The AI provider to check
    ///
    /// # Returns
    /// * `true` if the API key is set and non-empty
    /// * `false` otherwise
    pub fn is_key_available(provider: AIProvider) -> bool {
        Self::load_api_key(provider).is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    // Helper to set/unset environment variables safely
    struct EnvGuard {
        key: &'static str,
        original: Option<String>,
    }

    impl EnvGuard {
        fn new(key: &'static str) -> Self {
            let original = env::var(key).ok();
            Self { key, original }
        }

        fn set(&self, value: &str) {
            env::set_var(self.key, value);
        }

        fn unset(&self) {
            env::remove_var(self.key);
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match &self.original {
                Some(val) => env::set_var(self.key, val),
                None => env::remove_var(self.key),
            }
        }
    }

    // === SecretApiKey Tests ===

    #[test]
    fn test_secret_api_key_new_valid() {
        let key = SecretApiKey::new("sk-test-key-123".to_string());
        assert!(key.is_some());
    }

    #[test]
    fn test_secret_api_key_new_empty() {
        let key = SecretApiKey::new("".to_string());
        assert!(key.is_none());
    }

    #[test]
    fn test_secret_api_key_new_whitespace_only() {
        let key = SecretApiKey::new("   \t\n  ".to_string());
        assert!(key.is_none());
    }

    #[test]
    fn test_secret_api_key_trims_whitespace() {
        let key = SecretApiKey::new("  sk-test-key  ".to_string()).unwrap();
        assert_eq!(key.expose(), "sk-test-key");
    }

    #[test]
    fn test_secret_api_key_expose() {
        let key = SecretApiKey::new("my-secret-key".to_string()).unwrap();
        assert_eq!(key.expose(), "my-secret-key");
    }

    #[test]
    fn test_secret_api_key_debug_redacted() {
        let key = SecretApiKey::new("super-secret-key".to_string()).unwrap();
        let debug_str = format!("{:?}", key);

        // Should not contain the actual key
        assert!(!debug_str.contains("super-secret-key"));
        // Should contain REDACTED
        assert!(debug_str.contains("REDACTED"));
    }

    #[test]
    fn test_secret_api_key_display_redacted() {
        let key = SecretApiKey::new("super-secret-key".to_string()).unwrap();
        let display_str = format!("{}", key);

        // Should not contain the actual key
        assert!(!display_str.contains("super-secret-key"));
        // Should indicate it's redacted
        assert!(display_str.contains("REDACTED"));
    }

    // === AIProvider Tests ===

    #[test]
    fn test_ai_provider_env_var_names() {
        assert_eq!(AIProvider::Claude.env_var_name(), "ANTHROPIC_API_KEY");
        assert_eq!(AIProvider::Gemini.env_var_name(), "GOOGLE_AI_API_KEY");
    }

    #[test]
    fn test_ai_provider_display_names() {
        assert_eq!(AIProvider::Claude.display_name(), "Anthropic Claude");
        assert_eq!(AIProvider::Gemini.display_name(), "Google Gemini");
    }

    #[test]
    fn test_ai_provider_from_str() {
        assert_eq!("claude".parse::<AIProvider>().unwrap(), AIProvider::Claude);
        assert_eq!("Claude".parse::<AIProvider>().unwrap(), AIProvider::Claude);
        assert_eq!(
            "anthropic".parse::<AIProvider>().unwrap(),
            AIProvider::Claude
        );
        assert_eq!("gemini".parse::<AIProvider>().unwrap(), AIProvider::Gemini);
        assert_eq!("Gemini".parse::<AIProvider>().unwrap(), AIProvider::Gemini);
        assert_eq!("google".parse::<AIProvider>().unwrap(), AIProvider::Gemini);
    }

    #[test]
    fn test_ai_provider_from_str_invalid() {
        let result = "invalid".parse::<AIProvider>();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Unknown"));
    }

    // === ApiKeyManager Tests ===

    #[test]
    fn test_load_api_key_success() {
        let guard = EnvGuard::new(ANTHROPIC_API_KEY_ENV);
        guard.set("test-api-key-12345");

        let result = ApiKeyManager::load_api_key(AIProvider::Claude);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().expose(), "test-api-key-12345");
    }

    #[test]
    fn test_load_api_key_not_set() {
        let guard = EnvGuard::new(ANTHROPIC_API_KEY_ENV);
        guard.unset();

        let result = ApiKeyManager::load_api_key(AIProvider::Claude);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), AIError::Unauthorized));
    }

    #[test]
    fn test_load_api_key_empty() {
        let guard = EnvGuard::new(ANTHROPIC_API_KEY_ENV);
        guard.set("");

        let result = ApiKeyManager::load_api_key(AIProvider::Claude);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), AIError::Unauthorized));
    }

    #[test]
    fn test_load_api_key_gemini() {
        let guard = EnvGuard::new(GOOGLE_AI_API_KEY_ENV);
        guard.set("gemini-test-key");

        let result = ApiKeyManager::load_api_key(AIProvider::Gemini);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().expose(), "gemini-test-key");
    }

    #[test]
    fn test_is_key_available_true() {
        let guard = EnvGuard::new(ANTHROPIC_API_KEY_ENV);
        guard.set("some-key");

        assert!(ApiKeyManager::is_key_available(AIProvider::Claude));
    }

    #[test]
    fn test_is_key_available_false() {
        let guard = EnvGuard::new(ANTHROPIC_API_KEY_ENV);
        guard.unset();

        assert!(!ApiKeyManager::is_key_available(AIProvider::Claude));
    }

    // === Guidance Message Tests ===

    #[test]
    fn test_missing_key_guidance_claude() {
        let guidance = ApiKeyManager::missing_key_guidance(AIProvider::Claude);

        assert!(guidance.contains("ANTHROPIC_API_KEY"));
        assert!(guidance.contains("Anthropic Claude"));
        assert!(guidance.contains(".env"));
        assert!(guidance.contains("export"));
    }

    #[test]
    fn test_missing_key_guidance_gemini() {
        let guidance = ApiKeyManager::missing_key_guidance(AIProvider::Gemini);

        assert!(guidance.contains("GOOGLE_AI_API_KEY"));
        assert!(guidance.contains("Google Gemini"));
        assert!(guidance.contains(".env"));
    }

    // === Clone Tests ===

    #[test]
    fn test_secret_api_key_clone() {
        let key = SecretApiKey::new("cloneable-key".to_string()).unwrap();
        let cloned = key.clone();

        assert_eq!(key.expose(), cloned.expose());
    }
}
