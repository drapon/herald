//! Activity port definitions
//!
//! Defines the domain model and port traits for activity tracking functionality.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;

// ============================================================================
// Domain Models
// ============================================================================

/// Activity data extracted from a screenshot
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Activity {
    /// Database ID (auto-increment)
    pub id: Option<i64>,
    /// Reference to the capture this activity was extracted from
    pub capture_id: i64,
    /// Unix timestamp of the activity
    pub timestamp: i64,
    /// Name of the application being used
    pub application_name: String,
    /// Description of what the user was doing
    pub activity_description: String,
    /// Category of the activity
    pub category: ActivityCategory,
    /// Record creation timestamp
    pub created_at: i64,
}

/// Category of an activity
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ActivityCategory {
    /// Software development, coding
    Coding,
    /// Writing documentation, notes
    Documentation,
    /// Email, chat, messaging
    Communication,
    /// Web browsing
    Browsing,
    /// Video calls, meetings
    Meeting,
    /// Design work, graphics
    Design,
    /// Other activities
    #[serde(untagged)]
    Other(String),
}

impl ActivityCategory {
    /// Convert category to string representation
    pub fn as_str(&self) -> &str {
        match self {
            ActivityCategory::Coding => "coding",
            ActivityCategory::Documentation => "documentation",
            ActivityCategory::Communication => "communication",
            ActivityCategory::Browsing => "browsing",
            ActivityCategory::Meeting => "meeting",
            ActivityCategory::Design => "design",
            ActivityCategory::Other(s) => s.as_str(),
        }
    }

    /// Parse a string into an ActivityCategory
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "coding" => ActivityCategory::Coding,
            "documentation" => ActivityCategory::Documentation,
            "communication" => ActivityCategory::Communication,
            "browsing" => ActivityCategory::Browsing,
            "meeting" => ActivityCategory::Meeting,
            "design" => ActivityCategory::Design,
            other => ActivityCategory::Other(other.to_string()),
        }
    }
}

impl std::fmt::Display for ActivityCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Statistics for activities grouped by application
#[derive(Debug, Clone)]
pub struct ApplicationStats {
    /// Name of the application
    pub application_name: String,
    /// Number of activities recorded for this application
    pub count: usize,
    /// Estimated time spent in minutes
    pub estimated_minutes: u64,
}

/// Statistics for activities grouped by category
#[derive(Debug, Clone)]
pub struct CategoryStats {
    /// Category of activities
    pub category: ActivityCategory,
    /// Number of activities recorded for this category
    pub count: usize,
    /// Estimated time spent in minutes
    pub estimated_minutes: u64,
}

// ============================================================================
// Errors
// ============================================================================

/// Errors that can occur during activity storage operations
#[derive(Debug, Error)]
pub enum ActivityStorageError {
    /// Database operation failed
    #[error("Database error: {0}")]
    DatabaseError(String),

    /// Record not found
    #[error("Activity not found: id={0}")]
    NotFound(i64),

    /// Connection error
    #[error("Connection error: {0}")]
    ConnectionError(String),
}

/// Errors that can occur during activity analysis
#[derive(Debug, Error)]
pub enum AnalysisError {
    /// Failed to read the image file
    #[error("Image read error: {0}")]
    ImageReadError(String),

    /// AI provider returned an error
    #[error("AI provider error: {0}")]
    AIError(String),

    /// Failed to parse the AI response
    #[error("Response parse error: {0}")]
    ParseError(String),
}

// ============================================================================
// Analysis Types
// ============================================================================

/// Capture information for analysis
#[derive(Debug, Clone)]
pub struct CaptureForAnalysis {
    /// Capture ID from the database
    pub id: i64,
    /// Unix timestamp of the capture
    pub timestamp: i64,
    /// Path to the image file
    pub file_path: String,
}

/// Result of analyzing a screenshot
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisResult {
    /// Name of the application detected
    pub application_name: String,
    /// Description of the activity
    pub activity_description: String,
    /// Category of the activity
    pub category: ActivityCategory,
}

// ============================================================================
// Port Traits
// ============================================================================

/// Port for activity storage operations
#[async_trait]
pub trait ActivityStoragePort: Send + Sync {
    /// Save an activity (insert or update if capture_id exists)
    async fn save_activity(&self, activity: Activity) -> Result<i64, ActivityStorageError>;

    /// Get activities within a date range
    async fn get_activities_by_date_range(
        &self,
        from: i64,
        to: i64,
    ) -> Result<Vec<Activity>, ActivityStorageError>;

    /// Get today's activities
    async fn get_today_activities(&self) -> Result<Vec<Activity>, ActivityStorageError>;

    /// Get IDs of captures that haven't been analyzed yet
    async fn get_unanalyzed_capture_ids(&self) -> Result<Vec<i64>, ActivityStorageError>;

    /// Get captures that haven't been analyzed yet (with full metadata for analysis)
    async fn get_unanalyzed_captures(
        &self,
        limit: usize,
    ) -> Result<Vec<CaptureForAnalysis>, ActivityStorageError>;

    /// Get statistics grouped by application
    async fn get_stats_by_application(
        &self,
        from: i64,
        to: i64,
    ) -> Result<Vec<ApplicationStats>, ActivityStorageError>;

    /// Get statistics grouped by category
    async fn get_stats_by_category(
        &self,
        from: i64,
        to: i64,
    ) -> Result<Vec<CategoryStats>, ActivityStorageError>;
}

/// Port for activity analysis operations
#[async_trait]
pub trait ActivityAnalyzerPort: Send + Sync {
    /// Analyze a single capture
    async fn analyze(&self, capture: CaptureForAnalysis) -> Result<AnalysisResult, AnalysisError>;

    /// Analyze multiple captures in batch
    async fn analyze_batch(
        &self,
        captures: Vec<CaptureForAnalysis>,
    ) -> Vec<Result<(i64, AnalysisResult), AnalysisError>>;
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // === ActivityCategory Tests ===

    #[test]
    fn test_activity_category_as_str() {
        assert_eq!(ActivityCategory::Coding.as_str(), "coding");
        assert_eq!(ActivityCategory::Documentation.as_str(), "documentation");
        assert_eq!(ActivityCategory::Communication.as_str(), "communication");
        assert_eq!(ActivityCategory::Browsing.as_str(), "browsing");
        assert_eq!(ActivityCategory::Meeting.as_str(), "meeting");
        assert_eq!(ActivityCategory::Design.as_str(), "design");
        assert_eq!(
            ActivityCategory::Other("gaming".to_string()).as_str(),
            "gaming"
        );
    }

    #[test]
    fn test_activity_category_from_str() {
        assert_eq!(
            ActivityCategory::from_str("coding"),
            ActivityCategory::Coding
        );
        assert_eq!(
            ActivityCategory::from_str("documentation"),
            ActivityCategory::Documentation
        );
        assert_eq!(
            ActivityCategory::from_str("communication"),
            ActivityCategory::Communication
        );
        assert_eq!(
            ActivityCategory::from_str("browsing"),
            ActivityCategory::Browsing
        );
        assert_eq!(
            ActivityCategory::from_str("meeting"),
            ActivityCategory::Meeting
        );
        assert_eq!(
            ActivityCategory::from_str("design"),
            ActivityCategory::Design
        );
    }

    #[test]
    fn test_activity_category_from_str_case_insensitive() {
        assert_eq!(
            ActivityCategory::from_str("CODING"),
            ActivityCategory::Coding
        );
        assert_eq!(
            ActivityCategory::from_str("Coding"),
            ActivityCategory::Coding
        );
        assert_eq!(
            ActivityCategory::from_str("DOCUMENTATION"),
            ActivityCategory::Documentation
        );
    }

    #[test]
    fn test_activity_category_from_str_unknown() {
        let category = ActivityCategory::from_str("gaming");
        assert_eq!(category, ActivityCategory::Other("gaming".to_string()));

        let category = ActivityCategory::from_str("research");
        assert_eq!(category, ActivityCategory::Other("research".to_string()));
    }

    #[test]
    fn test_activity_category_display() {
        assert_eq!(format!("{}", ActivityCategory::Coding), "coding");
        assert_eq!(
            format!("{}", ActivityCategory::Other("custom".to_string())),
            "custom"
        );
    }

    #[test]
    fn test_activity_category_equality() {
        assert_eq!(ActivityCategory::Coding, ActivityCategory::Coding);
        assert_ne!(ActivityCategory::Coding, ActivityCategory::Documentation);
        assert_eq!(
            ActivityCategory::Other("test".to_string()),
            ActivityCategory::Other("test".to_string())
        );
        assert_ne!(
            ActivityCategory::Other("test1".to_string()),
            ActivityCategory::Other("test2".to_string())
        );
    }

    // === Activity Tests ===

    #[test]
    fn test_activity_creation() {
        let activity = Activity {
            id: Some(1),
            capture_id: 100,
            timestamp: 1704326400,
            application_name: "Visual Studio Code".to_string(),
            activity_description: "Editing Rust code".to_string(),
            category: ActivityCategory::Coding,
            created_at: 1704326400,
        };

        assert_eq!(activity.id, Some(1));
        assert_eq!(activity.capture_id, 100);
        assert_eq!(activity.application_name, "Visual Studio Code");
        assert_eq!(activity.category, ActivityCategory::Coding);
    }

    #[test]
    fn test_activity_serialization() {
        let activity = Activity {
            id: Some(1),
            capture_id: 100,
            timestamp: 1704326400,
            application_name: "VS Code".to_string(),
            activity_description: "Coding".to_string(),
            category: ActivityCategory::Coding,
            created_at: 1704326400,
        };

        let json = serde_json::to_string(&activity).expect("Failed to serialize");
        assert!(json.contains("\"category\":\"coding\""));
        assert!(json.contains("\"application_name\":\"VS Code\""));
    }

    #[test]
    fn test_activity_deserialization() {
        let json = r#"{
            "id": 1,
            "capture_id": 100,
            "timestamp": 1704326400,
            "application_name": "VS Code",
            "activity_description": "Coding",
            "category": "coding",
            "created_at": 1704326400
        }"#;

        let activity: Activity = serde_json::from_str(json).expect("Failed to deserialize");
        assert_eq!(activity.id, Some(1));
        assert_eq!(activity.category, ActivityCategory::Coding);
    }

    #[test]
    fn test_activity_deserialization_other_category() {
        let json = r#"{
            "id": 1,
            "capture_id": 100,
            "timestamp": 1704326400,
            "application_name": "Game",
            "activity_description": "Playing",
            "category": "gaming",
            "created_at": 1704326400
        }"#;

        let activity: Activity = serde_json::from_str(json).expect("Failed to deserialize");
        assert_eq!(
            activity.category,
            ActivityCategory::Other("gaming".to_string())
        );
    }

    // === Statistics Tests ===

    #[test]
    fn test_application_stats() {
        let stats = ApplicationStats {
            application_name: "VS Code".to_string(),
            count: 50,
            estimated_minutes: 120,
        };

        assert_eq!(stats.application_name, "VS Code");
        assert_eq!(stats.count, 50);
        assert_eq!(stats.estimated_minutes, 120);
    }

    #[test]
    fn test_category_stats() {
        let stats = CategoryStats {
            category: ActivityCategory::Coding,
            count: 100,
            estimated_minutes: 240,
        };

        assert_eq!(stats.category, ActivityCategory::Coding);
        assert_eq!(stats.count, 100);
        assert_eq!(stats.estimated_minutes, 240);
    }

    // === Error Tests ===

    #[test]
    fn test_activity_storage_error_display() {
        let err = ActivityStorageError::DatabaseError("Connection timeout".to_string());
        assert!(err.to_string().contains("Connection timeout"));

        let err = ActivityStorageError::NotFound(42);
        assert!(err.to_string().contains("42"));

        let err = ActivityStorageError::ConnectionError("Failed to connect".to_string());
        assert!(err.to_string().contains("Failed to connect"));
    }

    #[test]
    fn test_analysis_error_display() {
        let err = AnalysisError::ImageReadError("File not found".to_string());
        assert!(err.to_string().contains("File not found"));

        let err = AnalysisError::AIError("Rate limit exceeded".to_string());
        assert!(err.to_string().contains("Rate limit exceeded"));

        let err = AnalysisError::ParseError("Invalid JSON".to_string());
        assert!(err.to_string().contains("Invalid JSON"));
    }

    // === Analysis Types Tests ===

    #[test]
    fn test_capture_for_analysis() {
        let capture = CaptureForAnalysis {
            id: 1,
            timestamp: 1704326400,
            file_path: "/path/to/capture.png".to_string(),
        };

        assert_eq!(capture.id, 1);
        assert_eq!(capture.timestamp, 1704326400);
        assert!(capture.file_path.ends_with(".png"));
    }

    #[test]
    fn test_analysis_result() {
        let result = AnalysisResult {
            application_name: "Chrome".to_string(),
            activity_description: "Reading documentation".to_string(),
            category: ActivityCategory::Documentation,
        };

        assert_eq!(result.application_name, "Chrome");
        assert_eq!(result.category, ActivityCategory::Documentation);
    }

    #[test]
    fn test_analysis_result_serialization() {
        let result = AnalysisResult {
            application_name: "Chrome".to_string(),
            activity_description: "Browsing".to_string(),
            category: ActivityCategory::Browsing,
        };

        let json = serde_json::to_string(&result).expect("Failed to serialize");
        assert!(json.contains("\"category\":\"browsing\""));

        let parsed: AnalysisResult = serde_json::from_str(&json).expect("Failed to deserialize");
        assert_eq!(parsed.category, ActivityCategory::Browsing);
    }
}
