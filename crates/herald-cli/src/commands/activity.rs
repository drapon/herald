//! Activity command
//!
//! Handles `herald activity` subcommands for viewing and managing activity records.

use anyhow::{Context, Result};
use chrono::{Local, TimeZone, Utc};
use clap::{Subcommand, ValueEnum};
use herald_adapters::{AIActivityAnalyzer, SqliteAdapter};
use herald_core::exporter::{ActivityExporter, ExportOptions};
use herald_core::load_config;
use herald_core::ports::activity::{
    Activity, ActivityAnalyzerPort, ActivityStoragePort, CaptureForAnalysis,
};
use herald_core::ports::ai::AIProviderPort;
use herald_core::ports::StoragePort;
use std::path::PathBuf;
use std::sync::Arc;

/// Activity subcommand actions
#[derive(Subcommand, Debug)]
pub enum ActivityAction {
    /// Analyze unprocessed captures and extract activity information
    Analyze {
        /// Maximum number of captures to analyze (default: 10)
        #[arg(short = 'n', long, default_value = "10")]
        limit: usize,
    },
    /// List activity records
    List {
        /// Number of records to show (default: 20)
        #[arg(short = 'n', long, default_value = "20")]
        limit: usize,

        /// Show activities from today only
        #[arg(long)]
        today: bool,
    },
    /// Show activity statistics
    Stats {
        /// Show stats for today only
        #[arg(long)]
        today: bool,
    },
    /// Export activity data to file
    Export {
        /// Output format (jsonl or markdown)
        #[arg(short, long, default_value = "jsonl")]
        format: ExportFormat,

        /// Output file path (defaults to stdout)
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Export today's activities only
        #[arg(long)]
        today: bool,

        /// Include summary in markdown output
        #[arg(long)]
        summary: bool,

        /// Group by date in markdown output
        #[arg(long)]
        group_by_date: bool,
    },
}

/// Export format options
#[derive(Debug, Clone, ValueEnum)]
pub enum ExportFormat {
    /// JSON Lines format (one JSON object per line)
    Jsonl,
    /// Markdown format
    Markdown,
}

/// Run activity subcommand
pub async fn run(action: ActivityAction) -> Result<()> {
    match action {
        ActivityAction::Analyze { limit } => analyze(limit).await,
        ActivityAction::Export {
            format,
            output,
            today,
            summary,
            group_by_date,
        } => export(format, output, today, summary, group_by_date).await,
        ActivityAction::List { limit, today } => list(limit, today).await,
        ActivityAction::Stats { today } => stats(today).await,
    }
}

/// Analyze unprocessed captures
async fn analyze(limit: usize) -> Result<()> {
    let config = load_config().context("Failed to load configuration")?;
    let db_path = config.storage.data_dir.join("herald.db");

    if !db_path.exists() {
        println!("No database found. Run 'herald daemon start' or 'herald capture' first.");
        return Ok(());
    }

    let storage = SqliteAdapter::new(&db_path)
        .await
        .context("Failed to connect to database")?;

    // Get unanalyzed capture IDs
    let unanalyzed_ids = storage
        .get_unanalyzed_capture_ids()
        .await
        .context("Failed to get unanalyzed captures")?;

    if unanalyzed_ids.is_empty() {
        println!("No unanalyzed captures found.");
        return Ok(());
    }

    let ids_to_analyze: Vec<_> = unanalyzed_ids.into_iter().take(limit).collect();
    println!(
        "Found {} captures to analyze (processing up to {})",
        ids_to_analyze.len(),
        limit
    );

    // Get capture metadata for unanalyzed captures
    let recent_captures = storage
        .get_recent_captures(1000) // Get a large number to find our IDs
        .await
        .context("Failed to get capture metadata")?;

    // Build CaptureForAnalysis from metadata
    let captures_to_analyze: Vec<CaptureForAnalysis> = recent_captures
        .into_iter()
        .filter(|c| c.id.is_some() && ids_to_analyze.contains(&c.id.unwrap()))
        .map(|c| CaptureForAnalysis {
            id: c.id.unwrap(),
            timestamp: c.timestamp,
            file_path: c.file_path,
        })
        .collect();

    if captures_to_analyze.is_empty() {
        println!("Could not find capture files for unanalyzed IDs.");
        return Ok(());
    }

    println!("Analyzing {} captures...", captures_to_analyze.len());
    println!();

    // Run analysis based on configured provider
    let (success_count, error_count) = match config.ai.default_provider.as_str() {
        "gemini" => {
            let api_key = std::env::var("GEMINI_API_KEY")
                .context("GEMINI_API_KEY environment variable not set. Set it to use Gemini AI.")?;
            let provider = Arc::new(herald_adapters::GeminiAdapter::new(
                api_key,
                config.ai.model.clone(),
            ));
            let analyzer = AIActivityAnalyzer::new(provider);
            run_analysis(&analyzer, &storage, captures_to_analyze).await
        }
        _ => {
            // Default to Claude
            let api_key = std::env::var("ANTHROPIC_API_KEY").context(
                "ANTHROPIC_API_KEY environment variable not set. Set it to use Claude AI.",
            )?;
            let provider = Arc::new(herald_adapters::ClaudeAdapter::new(
                api_key,
                config.ai.model.clone(),
            ));
            let analyzer = AIActivityAnalyzer::new(provider);
            run_analysis(&analyzer, &storage, captures_to_analyze).await
        }
    };

    println!();
    println!(
        "Analysis complete: {} succeeded, {} failed",
        success_count, error_count
    );

    Ok(())
}

/// Run analysis on captures using the provided analyzer
async fn run_analysis<P: AIProviderPort + 'static>(
    analyzer: &AIActivityAnalyzer<P>,
    storage: &SqliteAdapter,
    captures: Vec<CaptureForAnalysis>,
) -> (usize, usize) {
    let mut success_count = 0;
    let mut error_count = 0;

    for capture in captures {
        let capture_id = capture.id;
        let timestamp = capture.timestamp;

        print!("  Analyzing capture {}... ", capture_id);

        match analyzer.analyze(capture).await {
            Ok(result) => {
                // Save the activity
                let activity = Activity {
                    id: None,
                    capture_id,
                    timestamp,
                    application_name: result.application_name.clone(),
                    activity_description: result.activity_description.clone(),
                    category: result.category.clone(),
                    created_at: Utc::now().timestamp(),
                };

                match storage.save_activity(activity).await {
                    Ok(_) => {
                        println!("OK - {} ({})", result.application_name, result.category);
                        success_count += 1;
                    }
                    Err(e) => {
                        println!("Failed to save: {}", e);
                        error_count += 1;
                    }
                }
            }
            Err(e) => {
                println!("Failed: {}", e);
                error_count += 1;
            }
        }
    }

    (success_count, error_count)
}

/// List activity records
async fn list(limit: usize, today_only: bool) -> Result<()> {
    let config = load_config().context("Failed to load configuration")?;
    let db_path = config.storage.data_dir.join("herald.db");

    if !db_path.exists() {
        println!("No database found. Run 'herald daemon start' or 'herald capture' first.");
        return Ok(());
    }

    let storage = SqliteAdapter::new(&db_path)
        .await
        .context("Failed to connect to database")?;

    let activities = if today_only {
        storage
            .get_today_activities()
            .await
            .context("Failed to get today's activities")?
    } else {
        // Get all activities within last 7 days
        let now = Utc::now().timestamp();
        let week_ago = now - (7 * 24 * 60 * 60);
        storage
            .get_activities_by_date_range(week_ago, now)
            .await
            .context("Failed to get activities")?
    };

    if activities.is_empty() {
        if today_only {
            println!("No activities recorded today.");
            println!("Run 'herald activity analyze' to analyze recent captures.");
        } else {
            println!("No activities recorded in the last 7 days.");
            println!("Run 'herald activity analyze' to analyze recent captures.");
        }
        return Ok(());
    }

    // Take only the requested limit
    let activities: Vec<_> = activities.into_iter().take(limit).collect();

    println!("Activity Records");
    println!("================");
    println!();

    for activity in &activities {
        let time = format_timestamp(activity.timestamp);
        let category = format!("{}", activity.category);

        println!("[{}] {}", time, activity.application_name);
        println!("  {} ({})", activity.activity_description, category);
        println!();
    }

    println!(
        "Showing {} of {} activities",
        activities.len(),
        activities.len()
    );

    Ok(())
}

/// Show activity statistics
async fn stats(today_only: bool) -> Result<()> {
    let config = load_config().context("Failed to load configuration")?;
    let db_path = config.storage.data_dir.join("herald.db");

    if !db_path.exists() {
        println!("No database found. Run 'herald daemon start' or 'herald capture' first.");
        return Ok(());
    }

    let storage = SqliteAdapter::new(&db_path)
        .await
        .context("Failed to connect to database")?;

    let (from, to, period_label) = if today_only {
        let now = Local::now();
        let start_of_day = now
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_local_timezone(Local)
            .unwrap()
            .timestamp();
        let end_of_day = start_of_day + 24 * 60 * 60;
        (start_of_day, end_of_day, "Today")
    } else {
        let now = Utc::now().timestamp();
        let week_ago = now - (7 * 24 * 60 * 60);
        (week_ago, now, "Last 7 days")
    };

    // Get statistics
    let app_stats = storage
        .get_stats_by_application(from, to)
        .await
        .context("Failed to get application statistics")?;

    let category_stats = storage
        .get_stats_by_category(from, to)
        .await
        .context("Failed to get category statistics")?;

    if app_stats.is_empty() && category_stats.is_empty() {
        println!("No activity data for {}.", period_label.to_lowercase());
        println!("Run 'herald activity analyze' to analyze recent captures.");
        return Ok(());
    }

    println!("Activity Statistics ({})", period_label);
    println!("==================================");
    println!();

    // Display category statistics
    println!("By Category:");
    println!("-----------");
    let total_minutes: u64 = category_stats.iter().map(|s| s.estimated_minutes).sum();
    for stat in &category_stats {
        let percentage = if total_minutes > 0 {
            (stat.estimated_minutes as f64 / total_minutes as f64) * 100.0
        } else {
            0.0
        };
        println!(
            "  {:15} {:4} records ({:3} min, {:5.1}%)",
            format!("{}", stat.category),
            stat.count,
            stat.estimated_minutes,
            percentage
        );
    }
    println!();

    // Display application statistics
    println!("By Application:");
    println!("--------------");
    for stat in app_stats.iter().take(10) {
        println!(
            "  {:25} {:4} records ({:3} min)",
            truncate_string(&stat.application_name, 25),
            stat.count,
            stat.estimated_minutes
        );
    }

    if app_stats.len() > 10 {
        println!("  ... and {} more applications", app_stats.len() - 10);
    }

    println!();
    println!(
        "Total: {} activities, {} minutes",
        category_stats.iter().map(|s| s.count).sum::<usize>(),
        total_minutes
    );

    Ok(())
}

/// Export activity data to file or stdout
async fn export(
    format: ExportFormat,
    output: Option<PathBuf>,
    today_only: bool,
    include_summary: bool,
    group_by_date: bool,
) -> Result<()> {
    let config = load_config().context("Failed to load configuration")?;
    let db_path = config.storage.data_dir.join("herald.db");

    if !db_path.exists() {
        println!("No database found. Run 'herald daemon start' or 'herald capture' first.");
        return Ok(());
    }

    let storage = SqliteAdapter::new(&db_path)
        .await
        .context("Failed to connect to database")?;

    let activities = if today_only {
        storage
            .get_today_activities()
            .await
            .context("Failed to get today's activities")?
    } else {
        // Get all activities within last 7 days
        let now = Utc::now().timestamp();
        let week_ago = now - (7 * 24 * 60 * 60);
        storage
            .get_activities_by_date_range(week_ago, now)
            .await
            .context("Failed to get activities")?
    };

    if activities.is_empty() {
        eprintln!("No activities to export.");
        return Ok(());
    }

    let content = match format {
        ExportFormat::Jsonl => ActivityExporter::to_jsonl(&activities),
        ExportFormat::Markdown => ActivityExporter::to_markdown(
            &activities,
            ExportOptions {
                include_summary,
                group_by_date,
            },
        ),
    };

    match output {
        Some(path) => {
            std::fs::write(&path, &content)
                .with_context(|| format!("Failed to write to {}", path.display()))?;
            eprintln!(
                "Exported {} activities to {}",
                activities.len(),
                path.display()
            );
        }
        None => {
            println!("{}", content);
        }
    }

    Ok(())
}

/// Truncate a string to a maximum length, adding "..." if truncated
fn truncate_string(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len.saturating_sub(3)])
    }
}

/// Format timestamp as local time string
fn format_timestamp(timestamp: i64) -> String {
    Utc.timestamp_opt(timestamp, 0)
        .single()
        .map(|dt| {
            dt.with_timezone(&Local)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_else(|| timestamp.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_timestamp() {
        // 2024-01-04 00:00:00 UTC
        let ts = 1704326400;
        let formatted = format_timestamp(ts);
        // Result depends on local timezone, but should contain date
        assert!(formatted.contains("2024-01-0"));
    }
}
