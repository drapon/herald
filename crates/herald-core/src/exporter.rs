//! Activity exporter
//!
//! Exports activity data to various formats (JSONL, Markdown).

use crate::ports::activity::Activity;
use chrono::{Local, TimeZone, Utc};
use serde::Serialize;

/// Export options for activity data
#[derive(Debug, Clone, Default)]
pub struct ExportOptions {
    /// Include a summary section at the end
    pub include_summary: bool,
    /// Group activities by date
    pub group_by_date: bool,
}

/// Activity exporter for various output formats
pub struct ActivityExporter;

/// Serializable activity for JSON export
#[derive(Serialize)]
struct JsonActivity {
    capture_id: i64,
    timestamp: i64,
    timestamp_iso: String,
    application_name: String,
    activity_description: String,
    category: String,
}

impl ActivityExporter {
    /// Export activities to JSONL (JSON Lines) format
    ///
    /// Each activity is serialized as a single JSON object on its own line.
    /// This format is ideal for log processing and streaming.
    ///
    /// # Arguments
    /// * `activities` - Slice of activities to export
    ///
    /// # Returns
    /// JSONL formatted string
    pub fn to_jsonl(activities: &[Activity]) -> String {
        if activities.is_empty() {
            return String::new();
        }

        activities
            .iter()
            .filter_map(|a| {
                let json_activity = JsonActivity {
                    capture_id: a.capture_id,
                    timestamp: a.timestamp,
                    timestamp_iso: format_timestamp_iso(a.timestamp),
                    application_name: a.application_name.clone(),
                    activity_description: a.activity_description.clone(),
                    category: format!("{}", a.category),
                };
                serde_json::to_string(&json_activity).ok()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Export activities to Markdown format
    ///
    /// Creates a human-readable Markdown document with activity records.
    ///
    /// # Arguments
    /// * `activities` - Slice of activities to export
    /// * `options` - Export options for customization
    ///
    /// # Returns
    /// Markdown formatted string
    pub fn to_markdown(activities: &[Activity], options: ExportOptions) -> String {
        if activities.is_empty() {
            return "# Activity Report\n\nNo activities recorded.\n".to_string();
        }

        let mut output = String::new();
        output.push_str("# Activity Report\n\n");

        if options.group_by_date {
            output.push_str(&Self::format_grouped_by_date(activities));
        } else {
            output.push_str(&Self::format_flat_list(activities));
        }

        if options.include_summary {
            output.push_str(&Self::format_summary(activities));
        }

        output
    }

    /// Format activities as a flat list
    fn format_flat_list(activities: &[Activity]) -> String {
        let mut output = String::new();

        for activity in activities {
            let time = format_timestamp_local(activity.timestamp);
            output.push_str(&format!(
                "- **{}** [{}]\n  {}\n  Category: {}\n\n",
                activity.application_name, time, activity.activity_description, activity.category
            ));
        }

        output
    }

    /// Format activities grouped by date
    fn format_grouped_by_date(activities: &[Activity]) -> String {
        use std::collections::BTreeMap;

        // Group activities by date
        let mut by_date: BTreeMap<String, Vec<&Activity>> = BTreeMap::new();

        for activity in activities {
            let date = format_date(activity.timestamp);
            by_date.entry(date).or_default().push(activity);
        }

        let mut output = String::new();

        for (date, day_activities) in by_date.iter().rev() {
            output.push_str(&format!("## {}\n\n", date));

            for activity in day_activities {
                let time = format_time_only(activity.timestamp);
                output.push_str(&format!(
                    "- **{}** [{}]\n  {}\n  Category: {}\n\n",
                    activity.application_name,
                    time,
                    activity.activity_description,
                    activity.category
                ));
            }
        }

        output
    }

    /// Format summary section
    fn format_summary(activities: &[Activity]) -> String {
        use std::collections::HashMap;

        let mut output = String::new();
        output.push_str("---\n\n## Summary\n\n");

        // Count by category
        let mut by_category: HashMap<String, usize> = HashMap::new();
        for activity in activities {
            *by_category
                .entry(format!("{}", activity.category))
                .or_default() += 1;
        }

        output.push_str("### By Category\n\n");
        output.push_str("| Category | Count |\n");
        output.push_str("|----------|-------|\n");

        let mut categories: Vec<_> = by_category.into_iter().collect();
        categories.sort_by(|a, b| b.1.cmp(&a.1));

        for (category, count) in categories {
            output.push_str(&format!("| {} | {} |\n", category, count));
        }

        output.push_str(&format!("\n**Total Activities:** {}\n", activities.len()));

        output
    }
}

/// Format timestamp as ISO 8601 string
fn format_timestamp_iso(timestamp: i64) -> String {
    match Utc.timestamp_opt(timestamp, 0).single() {
        Some(dt) => dt.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
        None => timestamp.to_string(),
    }
}

/// Format timestamp as local time string
fn format_timestamp_local(timestamp: i64) -> String {
    match Utc.timestamp_opt(timestamp, 0).single() {
        Some(dt) => dt
            .with_timezone(&Local)
            .format("%Y-%m-%d %H:%M")
            .to_string(),
        None => timestamp.to_string(),
    }
}

/// Format timestamp as date only
fn format_date(timestamp: i64) -> String {
    match Utc.timestamp_opt(timestamp, 0).single() {
        Some(dt) => dt.with_timezone(&Local).format("%Y-%m-%d").to_string(),
        None => timestamp.to_string(),
    }
}

/// Format timestamp as time only
fn format_time_only(timestamp: i64) -> String {
    match Utc.timestamp_opt(timestamp, 0).single() {
        Some(dt) => dt.with_timezone(&Local).format("%H:%M").to_string(),
        None => timestamp.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::activity::ActivityCategory;

    fn create_test_activities() -> Vec<Activity> {
        vec![
            Activity {
                id: Some(1),
                capture_id: 100,
                timestamp: 1704326400, // 2024-01-04 00:00:00 UTC
                application_name: "VS Code".to_string(),
                activity_description: "Writing Rust code".to_string(),
                category: ActivityCategory::Coding,
                created_at: 1704326400,
            },
            Activity {
                id: Some(2),
                capture_id: 101,
                timestamp: 1704330000, // 2024-01-04 01:00:00 UTC
                application_name: "Chrome".to_string(),
                activity_description: "Reading documentation".to_string(),
                category: ActivityCategory::Documentation,
                created_at: 1704330000,
            },
            Activity {
                id: Some(3),
                capture_id: 102,
                timestamp: 1704412800, // 2024-01-05 00:00:00 UTC
                application_name: "Slack".to_string(),
                activity_description: "Team discussion".to_string(),
                category: ActivityCategory::Communication,
                created_at: 1704412800,
            },
        ]
    }

    #[test]
    fn test_to_jsonl_empty() {
        let result = ActivityExporter::to_jsonl(&[]);
        assert_eq!(result, "");
    }

    #[test]
    fn test_to_jsonl_single_activity() {
        let activities = vec![Activity {
            id: Some(1),
            capture_id: 100,
            timestamp: 1704326400,
            application_name: "VS Code".to_string(),
            activity_description: "Writing code".to_string(),
            category: ActivityCategory::Coding,
            created_at: 1704326400,
        }];

        let result = ActivityExporter::to_jsonl(&activities);

        // Should be valid JSON
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["application_name"], "VS Code");
        assert_eq!(parsed["category"], "coding");
    }

    #[test]
    fn test_to_jsonl_multiple_activities() {
        let activities = create_test_activities();
        let result = ActivityExporter::to_jsonl(&activities);

        // Should have 3 lines
        let lines: Vec<_> = result.lines().collect();
        assert_eq!(lines.len(), 3);

        // Each line should be valid JSON
        for line in lines {
            let _: serde_json::Value = serde_json::from_str(line).unwrap();
        }
    }

    #[test]
    fn test_to_markdown_empty() {
        let result = ActivityExporter::to_markdown(&[], ExportOptions::default());
        assert!(result.contains("# Activity Report"));
        assert!(result.contains("No activities recorded"));
    }

    #[test]
    fn test_to_markdown_flat_list() {
        let activities = create_test_activities();
        let result = ActivityExporter::to_markdown(
            &activities,
            ExportOptions {
                include_summary: false,
                group_by_date: false,
            },
        );

        assert!(result.contains("# Activity Report"));
        assert!(result.contains("VS Code"));
        assert!(result.contains("Chrome"));
        assert!(result.contains("Slack"));
    }

    #[test]
    fn test_to_markdown_grouped_by_date() {
        let activities = create_test_activities();
        let result = ActivityExporter::to_markdown(
            &activities,
            ExportOptions {
                include_summary: false,
                group_by_date: true,
            },
        );

        assert!(result.contains("# Activity Report"));
        // Should have date headers
        assert!(result.contains("## 2024-01-0"));
    }

    #[test]
    fn test_to_markdown_with_summary() {
        let activities = create_test_activities();
        let result = ActivityExporter::to_markdown(
            &activities,
            ExportOptions {
                include_summary: true,
                group_by_date: false,
            },
        );

        assert!(result.contains("## Summary"));
        assert!(result.contains("### By Category"));
        assert!(result.contains("**Total Activities:** 3"));
    }

    #[test]
    fn test_format_timestamp_iso() {
        let ts = 1704326400; // 2024-01-04 00:00:00 UTC
        let result = format_timestamp_iso(ts);
        assert_eq!(result, "2024-01-04T00:00:00Z");
    }

    #[test]
    fn test_format_date() {
        let ts = 1704326400; // 2024-01-04 00:00:00 UTC
        let result = format_date(ts);
        // Result depends on local timezone
        assert!(result.contains("2024-01-0"));
    }
}
