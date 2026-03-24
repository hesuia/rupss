mod app;
mod collector;
mod format;
mod history;
mod snapshot;
mod tui;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    app::run()?;
    Ok(())
}
