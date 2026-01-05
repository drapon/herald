//! AI suggestion command
//!
//! Handles `herald suggest "<intent>"` command for AI-powered suggestions.
//! This is a placeholder for Task 14 implementation.

use anyhow::Result;

/// Get AI suggestions based on recent captures
///
/// This command collects recent screenshots and sends them to an AI provider
/// for context-aware suggestions based on the user's intent.
///
/// # Arguments
/// * `intent` - The user's intent or question for the AI
pub async fn run(intent: &str) -> Result<()> {
    println!("AI Suggestion Request");
    println!("====================");
    println!();
    println!("Intent: {}", intent);
    println!();
    println!("Note: AI integration will be implemented in Task 12-14.");
    println!("This command will:");
    println!("  1. Collect recent screenshots (up to 10)");
    println!("  2. Send them to the configured AI provider (Claude/Gemini)");
    println!("  3. Display AI-generated suggestions based on your work context");
    println!();
    println!("To use this feature, you'll need to:");
    println!("  1. Set ANTHROPIC_API_KEY or GOOGLE_AI_API_KEY environment variable");
    println!("  2. Configure ai.default_provider in ~/.herald/config.toml");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_suggest_runs_without_error() {
        // Placeholder test - should not error even without AI setup
        let result = run("test intent").await;
        assert!(result.is_ok());
    }
}
