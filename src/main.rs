mod app;
mod collector;
mod format;
mod history;
mod snapshot;
mod tui;

/// Starts the TUI application and restores the terminal on exit.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    app::run()?;
    Ok(())
}
