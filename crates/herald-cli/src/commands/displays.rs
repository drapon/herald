//! Displays command
//!
//! Handles `herald displays` command to list available displays.

use anyhow::{Context, Result};
use herald_adapters::ScreenCaptureKitAdapter;
use herald_core::ports::CapturePort;

/// List available displays
///
/// Shows all connected displays with their index, resolution, and usage examples.
pub async fn run() -> Result<()> {
    let capture_adapter = ScreenCaptureKitAdapter::new();

    let displays = capture_adapter
        .get_displays()
        .await
        .context("Failed to get display information")?;

    if displays.is_empty() {
        println!("No displays found.");
        return Ok(());
    }

    println!("Available Displays");
    println!("==================");
    println!();

    for display in &displays {
        println!(
            "  Display {}: {}x{}",
            display.index, display.width, display.height
        );
    }

    println!();
    println!("Configuration Examples");
    println!("----------------------");
    println!();
    println!("  # Capture all displays combined (default)");
    println!("  # display = (omit this option)");
    println!();

    if !displays.is_empty() {
        println!("  # Capture single display");
        println!("  display = 0");
        println!();
    }

    if displays.len() > 1 {
        let indices: Vec<String> = displays.iter().map(|d| d.index.to_string()).collect();
        println!("  # Capture multiple displays as separate files");
        println!("  display = [{}]", indices.join(", "));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    // Integration tests are in e2e_tests.rs
}
