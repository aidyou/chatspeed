// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

/// The entry point of the application.
/// Initializes logging and runs the application, handling any initialization or runtime errors.
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = chatspeed_lib::run().await;
    Ok(())
}
