//! Port definitions for Hexagonal Architecture
//!
//! These traits define the boundaries between the core domain and external adapters.

pub mod capture;
pub mod storage;
pub mod ai;

pub use capture::CapturePort;
pub use storage::StoragePort;
pub use ai::AIProviderPort;
