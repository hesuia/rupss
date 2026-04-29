# rupss

`rupss` is a lightweight Rust TUI for monitoring Linux system and per-process
memory usage. It reads from `/proc` and renders a live terminal UI with:

- Specializes in displaying RSS, USS, PSS, and Swap usage for processes, along with CPU% and other metadata.
- RSS and USS are 
- USS/PSS/swap details from `smaps_rollup` (loaded only for visible rows)
- Flat view and an expandable process tree view

## Requirements

- Linux (uses `/proc` and `smaps_rollup`)
- A Rust toolchain that supports the 2024 edition

## Install / Build

Clone the repo and build:

```sh
cargo build --release
```

Run:

```sh
./target/release/rupss
```

Or run directly with Cargo:

```sh
cargo run
```

## Controls

- Quit: `q`
- Move selection: `Up/Down` or `j/k`
- Page: `PgUp/PgDn`
- Jump: `Home/End`
- Toggle flat/tree view: `t`
- Tree expand/collapse: `Left/Right`
- Mouse: click a row to select; in tree mode, click the name column to toggle
- Sorting (press the same key again to toggle asc/desc):
  - `i`: PID
  - `p`: PPID
  - `o`: owner
  - `n`: name
  - `m`: command
  - `r`: RSS
  - `s`: swap
  - `c`: CPU%

## Notes

- USS/PSS and detailed swap come from `/proc/<pid>/smaps_rollup`. When access is
  denied (common for processes you do not own), the UI shows a dim placeholder.
- CPU% is computed from tick deltas between consecutive samples and is per
  process (not normalized by CPU count).

## Possible Next Features

- Persist UI preferences such as visible columns, column order, sort state, and view mode between runs.
- Add process search and filtering by PID, name, owner, or command substring.
- Add a pause/resume refresh toggle and configurable refresh interval.
- Add process actions such as sending signals or changing priority, with confirmation prompts.
- Add an in-app help overlay for shortcuts and navigation.

## License

MIT (see `LICENSE`).
