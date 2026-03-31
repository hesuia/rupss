use thiserror::Error;

/// Errors returned while collecting process and memory data from `/proc`.
#[derive(Debug, Error)]
pub enum CollectorError {
    /// Failed to read machine-wide memory information.
    #[error("failed to read /proc/meminfo")]
    ReadMeminfo(#[source] procfs::ProcError),
    /// Failed to enumerate the current process list.
    #[error("failed to enumerate /proc processes")]
    ListProcesses(#[source] procfs::ProcError),
}

/// Errors that stop the interactive terminal application.
#[derive(Debug, Error)]
pub enum AppError {
    /// Failed to switch the terminal into TUI mode.
    #[error("failed to set up terminal")]
    SetupTerminal(#[source] std::io::Error),
    /// Failed to restore the terminal back to the shell state.
    #[error("failed to restore terminal")]
    RestoreTerminal(#[source] std::io::Error),
    /// Failed to render the TUI frame.
    #[error("failed to draw terminal UI")]
    Render(#[source] std::io::Error),
    /// Failed to poll for terminal input events.
    #[error("failed to poll terminal events")]
    PollEvents(#[source] std::io::Error),
    /// Failed to read a terminal input event.
    #[error("failed to read terminal event")]
    ReadEvent(#[source] std::io::Error),
}
