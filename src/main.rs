mod app;
mod collector;
mod error;
mod format;
mod history;
mod snapshot;
mod tui;

use anyhow::Context;

/// Starts the TUI application and restores the terminal on exit.
fn main() -> anyhow::Result<()> {
    app::run().context("application error")?;
    Ok(())
}
