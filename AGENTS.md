# Repository Guidelines

`rupss` is a Rust-based TUI application for monitoring Linux system performance.
Lightweight and efficient, it collects data from `/proc` and renders it in a terminal UI.

## Project Structure & Module Organization
- `src/` contains all Rust source code. Each module is split by responsibility:
  - `app.rs`: state and event loop
  - `collector.rs`: procfs collection. can be extended to other sources in the future.
  - `tui.rs`: rendering tui
  - `snapshot.rs`: data model
  - `history.rs`: ring buffer for historical, fixed-size data
  - `format.rs`: formatting utilities(e.g., human-readable byte sizes)
  - `main.rs`: entrypoint
- Tests are unit tests colocated in each module under `#[cfg(test)]`.
- No static assets or external config files are required to run locally.

## Basic Commands
- `cargo build` compiles the binary.
- `cargo run` launches the TUI.
- `cargo check` type-checks quickly without producing a binary. Use this for validating changes.
- `cargo test` runs unit tests.
- `cargo fmt` formats code according to Rust style guidelines.
- `cargo clippy` runs lints to catch common mistakes and enforce idiomatic Rust.

## Coding Style & Naming Conventions
- Follow standard Rust formatting (`rustfmt`). Use 4-space indentation.
- Public APIs should have clear Rustdoc comments (`///`) explaining purpose,
  inputs, outputs, and any non-obvious behavior.
- Naming: `CamelCase` for types, `snake_case` for functions/variables,
  `SCREAMING_SNAKE_CASE` for constants. Keep module names short and topical
  (e.g., `collector`, `snapshot`, `tui`).

## rust coding conventions:
- Use `Result<T, E>` and `Option<T>` (+ `?`) for error handling and optional values instead of panicking.
  - Actively utilize methods like `map`, `and_then`, `unwrap_or`, etc. to handle these types in a functional style.
- Use pattern matching (`match`, `if let`, `let else`) to handle different cases explicitly, especially for enums and error handling.avoid complex nested `if` statements; prefer these constructions, early returns or pattern matching.
- Avoid `unwrap()` and `expect()` in production code; handle errors gracefully.
- Use iterators and combinators (`map`, `filter`, `fold`, `filter_map` etc.) for collection processing instead of manual loops where appropriate.
- Prefer immutable data structures and minimize mutable state. Use `mut` only when necessary.
  - In `app.rs`, it's acceptable to use `mut` to some extent, but avoid overusing or using it unnecessarily.
- Also, follow standard Rust coding conventions to write code that is easy to read and maintain.

## Testing Guidelines
Once changes are made, ensure they are correct.
Unless testing is absolutely essential or the code relates to TUI, we generally add tests for all functions and methods, especially those with complex logic or edge cases.
- Tests use Rust’s built-in test framework (`cargo test`).
- Prefer focused unit tests placed next to the code under test.
- Naming: descriptive test function names using `snake_case`
  (e.g., `calculates_cpu_from_tick_delta`).

## Commit & Pull Request Guidelines
- Commit messages in history are short, imperative, and specific
  (e.g., "refactor: extract snapshot logic into separate module", "bugfix: handle missing smaps_rollup gracefully").
- Keep commits small and scoped to a single change.
- PRs should include:
  - A concise summary of behavior changes.
  - How to test (commands and expected outcome).
  - Any user-visible UI changes called out explicitly.

## Configuration & Environment Notes
- Linux-only: relies on `/proc` and `smaps_rollup`.
- Running as non-root may limit access to some process details; the UI should
  degrade gracefully in those cases.
