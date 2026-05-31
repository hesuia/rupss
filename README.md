# rupss

`rupss` is a lightweight Rust TUI for monitoring Linux system and per-process
performance. It reads live data from `/proc` and renders a compact terminal UI
focused on memory, CPU, and process inspection.

## Features

- Flat list and expandable process tree views
- Sorting by PID, PPID, owner, name, command, RSS, swap, and CPU%
- Column picker with persistent in-session layout changes
- Filter modal for PID, PPID, name, command, RSS, swap, CPU, USS, and PSS
- Process monitor overlay for a single process with history charts
- USS/PSS data loaded only for visible rows to keep the UI responsive

## Requirements

- Linux
- A Rust toolchain that supports the 2024 edition

## Build and Run

```sh
cargo build --release
./target/release/rupss
```

Or run it directly in debug mode:

```sh
cargo run
```

## Controls

- `q`: quit
- `j`/`k` or arrow keys: move selection
- `PgUp`/`PgDn`: page up and down
- `Home`/`End`: jump to the top or bottom
- `t`: toggle flat/tree view
- `Left`/`Right`: collapse or expand in tree view
- `v`: open the column picker
- `s`: open the sort picker
- `f`: open the filter modal
- `p`: pause and resume refresh
- `Enter`: open the process monitor for the selected row
- Mouse: click a row to select it; in tree view, click the name column to expand or collapse

### Sort Picker

- `j`/`k` or arrow keys: move the selection
- `Enter` or `Space`: apply the selected sort key
- `v`: jump to the column picker
- `s`: close the sort picker

### Filter Modal

- `j`/`k` or arrow keys: move between filter rows
- `Enter`: toggle edit mode for the selected row
- While editing, type to update the active field
- `h`/`l`: cycle metric operators
- `Backspace`: delete the last character
- `Ctrl+U`: clear the active field
- `Esc`: close the modal

## Notes

- USS, PSS, and detailed swap information come from `/proc/<pid>/smaps_rollup`.
  Access can be denied for processes you do not own, so the UI falls back to a
  dim placeholder.
- CPU% is computed from tick deltas between consecutive samples and is not
  normalized by CPU count.
- The process monitor overlay keeps its own rolling history and is updated while
  the selected process is still visible.

## License

MIT, see [LICENSE](LICENSE).
