//! Herald CLI - AI-powered screen capture assistant
//!
//! Main entry point for the Herald application.

use anyhow::Result;
use clap::{Parser, Subcommand};

mod commands;

/// Herald - AI-powered screen capture assistant for macOS
///
/// Captures your desktop screen at regular intervals and provides
/// AI-powered suggestions based on your work context.
#[derive(Parser, Debug)]
#[command(name = "herald")]
#[command(author, version, about, long_about = None)]
#[command(propagate_version = true)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

/// Available subcommands
#[derive(Subcommand, Debug)]
enum Commands {
    /// Manage the Herald daemon (background service)
    Daemon {
        #[command(subcommand)]
        action: DaemonAction,
    },
    /// Capture a screenshot immediately
    Capture,
    /// Show daemon status and capture statistics
    Status,
    /// Get AI suggestions based on recent captures
    Suggest {
        /// Your intent or question for the AI
        intent: String,
    },
}

/// Daemon subcommand actions
#[derive(Subcommand, Debug)]
enum DaemonAction {
    /// Start the Herald daemon in the background
    Start,
    /// Stop the running Herald daemon
    Stop,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Daemon { action }) => match action {
            DaemonAction::Start => commands::daemon::start().await,
            DaemonAction::Stop => commands::daemon::stop().await,
        },
        Some(Commands::Capture) => commands::capture::run().await,
        Some(Commands::Status) => commands::status::run().await,
        Some(Commands::Suggest { intent }) => commands::suggest::run(&intent).await,
        None => {
            // No subcommand provided - show help
            println!("Herald v{}", env!("CARGO_PKG_VERSION"));
            println!("AI-powered screen capture assistant for macOS");
            println!();
            println!("Use --help for usage information.");
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn test_cli_parses_daemon_start() {
        let cli = Cli::try_parse_from(["herald", "daemon", "start"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Commands::Daemon {
                action: DaemonAction::Start
            })
        ));
    }

    #[test]
    fn test_cli_parses_daemon_stop() {
        let cli = Cli::try_parse_from(["herald", "daemon", "stop"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Commands::Daemon {
                action: DaemonAction::Stop
            })
        ));
    }

    #[test]
    fn test_cli_parses_capture() {
        let cli = Cli::try_parse_from(["herald", "capture"]).unwrap();
        assert!(matches!(cli.command, Some(Commands::Capture)));
    }

    #[test]
    fn test_cli_parses_status() {
        let cli = Cli::try_parse_from(["herald", "status"]).unwrap();
        assert!(matches!(cli.command, Some(Commands::Status)));
    }

    #[test]
    fn test_cli_parses_suggest_with_intent() {
        let cli = Cli::try_parse_from(["herald", "suggest", "UI design improvement"]).unwrap();
        match cli.command {
            Some(Commands::Suggest { intent }) => {
                assert_eq!(intent, "UI design improvement");
            }
            _ => panic!("Expected Suggest command"),
        }
    }

    #[test]
    fn test_cli_suggest_requires_intent() {
        let result = Cli::try_parse_from(["herald", "suggest"]);
        assert!(result.is_err());
    }

    #[test]
    fn test_cli_no_command_is_none() {
        let cli = Cli::try_parse_from(["herald"]).unwrap();
        assert!(cli.command.is_none());
    }

    #[test]
    fn test_cli_version_flag() {
        // --version should exit with an error (it's a special case in clap)
        let result = Cli::try_parse_from(["herald", "--version"]);
        assert!(result.is_err());
    }

    #[test]
    fn test_cli_help_flag() {
        // --help should exit with an error (it's a special case in clap)
        let result = Cli::try_parse_from(["herald", "--help"]);
        assert!(result.is_err());
    }

    #[test]
    fn test_cli_debug_assertions() {
        // Verify CLI structure is valid
        Cli::command().debug_assert();
    }
}
