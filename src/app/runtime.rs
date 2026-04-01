use super::{AppState, CrosstermTerminal, EVENT_POLL, KeyAction, TICK_RATE};
use crate::{error::AppError, tui};
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};
use std::{io, time::Instant};

/// Runs the TUI application until the user quits.
///
/// This sets up the terminal, runs the event loop, and ensures the terminal is
/// restored even when the loop exits due to user input.
pub fn run() -> Result<(), AppError> {
    let mut terminal = setup_terminal()?;
    let result = run_app(&mut terminal);
    match restore_terminal(&mut terminal) {
        Ok(()) => result,
        Err(error) => Err(error),
    }
}

fn run_app(terminal: &mut CrosstermTerminal) -> Result<(), AppError> {
    let mut app = AppState::default();
    app.refresh();

    let mut last_tick = Instant::now();
    loop {
        terminal
            .draw(|frame| tui::render(frame, &mut app))
            .map_err(AppError::Render)?;

        if event::poll(EVENT_POLL).map_err(AppError::PollEvents)? {
            match event::read().map_err(AppError::ReadEvent)? {
                Event::Key(key) => {
                    if matches!(key.kind, KeyEventKind::Press) {
                        match app.handle_key(key.code) {
                            KeyAction::Quit => return Ok(()),
                            KeyAction::Continue => {}
                        }
                    }
                }
                Event::Mouse(mouse) => app.handle_mouse(mouse),
                _ => {}
            }
        }

        if last_tick.elapsed() >= TICK_RATE {
            app.refresh();
            last_tick = Instant::now();
        }
    }
}

/// Switches the terminal into raw mode and enters the alternate screen.
fn setup_terminal() -> Result<CrosstermTerminal, AppError> {
    enable_raw_mode().map_err(AppError::SetupTerminal)?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture).map_err(AppError::SetupTerminal)?;
    let backend = CrosstermBackend::new(stdout);
    Terminal::new(backend).map_err(AppError::SetupTerminal)
}

/// Restores the terminal back to the normal shell state.
fn restore_terminal(terminal: &mut CrosstermTerminal) -> Result<(), AppError> {
    disable_raw_mode().map_err(AppError::RestoreTerminal)?;
    execute!(
        terminal.backend_mut(),
        DisableMouseCapture,
        LeaveAlternateScreen
    )
    .map_err(AppError::RestoreTerminal)?;
    terminal.show_cursor().map_err(AppError::RestoreTerminal)?;
    Ok(())
}
