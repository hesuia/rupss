# Repository Guidelines

`rupss` is a Rust-based TUI application for monitoring Linux system performance.
Lightweight and efficient, it collects data from `/proc` and renders it in a terminal UI.

## Project Structure & Module Organization
- `src/main.rs` boots the app and calls `app::run()`.
- `src/app.rs` owns the main event loop and high-level app state.
- `src/app/` contains UI behavior and state helpers (input, navigation, sorting, tree view, etc.).
- `src/collector.rs` reads process/memory data from `/proc` (Linux-only).
- `src/tui.rs` renders the terminal UI (Ratatui/Crossterm).
- `src/snapshot.rs`, `src/history.rs`, `src/format.rs` hold the core data model, history buffers, and formatting utilities.
- Tests are unit tests colocated with modules under `#[cfg(test)]` (no separate `tests/` directory).

## Main Dependencies
- `ratatui` + `crossterm`: terminal rendering and input/event handling.
- `procfs`: process and memory data sourced from `/proc`.
- `users`: for mapping UIDs to usernames.
- `anyhow`/`thiserror`: error propagation with context and typed errors where useful.
- other utilities:
  - `compact_str` for efficient string handling.
  - `strum` for string interning (e.g., process names).
## Build, Test, and Development Commands
- `cargo run` builds and launches the TUI in debug mode.
- `cargo build` builds a debug binary.
- `cargo build --release` builds an optimized binary at `target/release/rupss`.
- `cargo check` runs a fast type-check without producing a binary.
- `cargo test` runs the unit test suite.
- `cargo fmt` formats code with Rustfmt; run before pushing and ensure no formatting changes are pending.
- `cargo clippy` runs lints; prefer keeping the code warning-free.

## TUI Behavior (High Level)
- Supports a flat list view and an expandable process tree view.
- Process table is keyboard-driven and supports sorting (PID/owner/name/RSS/swap/CPU%, etc.).
- Some details come from `smaps_rollup` and may be unavailable for non-owned processes.

## Coding Style & Naming Conventions
- Use standard Rustfmt formatting (4-space indentation; no manual alignment).
- Prefer explicit error handling over panics: use `Result`/`Option` + `?`; avoid `unwrap()`/`expect()` outside tests.
- Keep module boundaries sharp: `/proc` parsing in `collector`, rendering in `tui`, UI logic in `app/`.
- Naming follows Rust conventions: `CamelCase` types, `snake_case` functions/vars, `SCREAMING_SNAKE_CASE` constants.
- Add doc comments (`///`) to public items and complex logic; internal helper functions can have inline comments as needed.

## rust coding conventions:
- Use `Result<T, E>` and `Option<T>` (+ `?`) for error handling and optional values instead of panicking.
  - Actively utilize methods like `map`, `and_then`, `unwrap_or`, etc. to handle these types in a functional style.
- Use pattern matching (`match`, `if let`, `let else`) to handle different cases explicitly, especially for enums and error handling.avoid complex nested `if` statements; prefer these constructions, early returns or pattern matching.
- Avoid `unwrap()` and `expect()` in production code; handle errors gracefully.
- Use iterators and combinators (`map`, `filter`, `fold`, `filter_map` etc.) for collection processing instead of manual loops where appropriate.
  - Split closures to prevent them from becoming too large.
- Prefer immutable data structures and minimize mutable state. Use `mut` only when necessary.
  - In `app.rs`, it's acceptable to use `mut` to some extent, but avoid overusing.
  - You can use `mut` for parts closely related to TUI, but for UI-independent parts, avoid using `mut` as much as possible and extract them into functions or methods so they can be tested.
- follow standard Rust coding conventions to write code that is easy to read and maintain.

## Testing Guidelines
- Add focused unit tests for non-UI logic (parsing, sorting, tree operations, formatting).
- Use descriptive `snake_case` test names (e.g., `sorts_by_rss_descending`).

## Commit & Pull Request Guidelines
- Commit subjects are short and imperative; common prefixes include `refactor:` and `BugFix:` when helpful.
- Keep PRs small and scoped. Include:
  - What changed and why
  - How to test (e.g., `cargo test`, then `cargo run` on Linux)
  - Screenshots/recordings if UI behavior changes

## Security & Environment Notes
- Requirements: Linux + a Rust toolchain that supports the 2024 edition.
- Linux-only: relies on `/proc` and `/proc/<pid>/smaps_rollup`; access may be denied for non-owned processes. Handle failures gracefully and keep the UI responsive.
