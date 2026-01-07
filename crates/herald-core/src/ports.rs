//! Port definitions for Hexagonal Architecture
//!
//! These traits define the boundaries between the core domain and external adapters.

pub mod activity;
pub mod ai;
pub mod capture;
pub mod storage;

pub use activity::{
    Activity, ActivityAnalyzerPort, ActivityCategory, ActivityStorageError, ActivityStoragePort,
    AnalysisError, AnalysisResult, ApplicationStats, CaptureForAnalysis, CategoryStats,
};
pub use ai::AIProviderPort;
pub use capture::CapturePort;
pub use storage::StoragePort;
