# Repository Guidelines

## Project Structure & Module Organization
- `src/` contains all Rust source code. Each module is split by responsibility:
  - `app.rs` (state and event loop), `collector.rs` (procfs collection),
    `tui.rs` (rendering), `snapshot.rs` (data model), `history.rs` (ring buffer),
    `format.rs` (display formatting), `main.rs` (entrypoint).
- Tests are unit tests colocated in each module under `#[cfg(test)]`.
- No static assets or external config files are required to run locally.

## Build, Test, and Development Commands
- `cargo build` compiles the binary.
- `cargo run` launches the TUI.
- `cargo check` type-checks quickly without producing a binary. validates code changes 
- `cargo test` runs unit tests.

## Coding Style & Naming Conventions
- Follow standard Rust formatting (`rustfmt`). Use 4-space indentation.
- Public APIs should have clear Rustdoc comments (`///`) explaining purpose,
  inputs, outputs, and any non-obvious behavior.
- Naming: `CamelCase` for types, `snake_case` for functions/variables,
  `SCREAMING_SNAKE_CASE` for constants. Keep module names short and topical
  (e.g., `collector`, `snapshot`, `tui`).

## Testing Guidelines
Once changes are made, ensure they are correct.
Unless testing is absolutely essential or the code relates to TUI, we generally add tests for all functions and methods, especially those with complex logic or edge cases.
- Tests use Rust’s built-in test framework (`cargo test`).
- Prefer focused unit tests placed next to the code under test.
- Naming: descriptive test function names using `snake_case`
  (e.g., `calculates_cpu_from_tick_delta`).

## Commit & Pull Request Guidelines
- Commit messages in history are short, imperative, and specific
  (e.g., "Refactor terminal type definitions", "Enhance documentation").
- Keep commits small and scoped to a single change.
- PRs should include:
  - A concise summary of behavior changes.
  - How to test (commands and expected outcome).
  - Any user-visible UI changes called out explicitly.

## Configuration & Environment Notes
- Linux-only: relies on `/proc` and `smaps_rollup`.
- Running as non-root may limit access to some process details; the UI should
  degrade gracefully in those cases.
