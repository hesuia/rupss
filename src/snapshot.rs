use compact_str::{CompactString, format_compact};
use std::time::Instant;
use strum::{AsRefStr, IntoStaticStr};

/// Sort keys supported by the process table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, AsRefStr, IntoStaticStr)]
#[strum(serialize_all = "snake_case")]
pub enum SortKey {
    /// Sort by process ID.
    Pid,
    /// Sort by parent process ID.
    Ppid,
    /// Sort by owner name.
    Owner,
    /// Sort by short process name.
    Name,
    /// Sort by full command line.
    Command,
    /// Sort by resident set size.
    Rss,
    /// Sort by swap usage.
    Swap,
    /// Sort by calculated CPU percentage.
    Cpu,
}

impl SortKey {
    /// Returns the default sort direction for this key.
    pub fn default_direction(&self) -> SortDirection {
        use SortKey::*;
        match self {
            Pid | Ppid | Owner | Name | Command => SortDirection::Ascending,
            Rss | Swap | Cpu => SortDirection::Descending,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, AsRefStr, IntoStaticStr)]
pub enum SortDirection {
    /// Sort in ascending order (e.g. lowest to highest).
    #[strum(serialize = "asc")]
    Ascending,
    /// Sort in descending order (e.g. highest to lowest).
    #[strum(serialize = "desc")]
    Descending,
}

impl SortDirection {
    /// Toggles the sort direction between ascending and descending.
    pub fn toggle(self) -> Self {
        match self {
            SortDirection::Ascending => SortDirection::Descending,
            SortDirection::Descending => SortDirection::Ascending,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SortState {
    /// The key currently used for sorting the process table.
    pub key: SortKey,
    /// The direction of sorting (ascending or descending).
    pub direction: SortDirection,
}

impl SortState {
    /// Creates a new `SortState` with the given key and default descending direction.
    pub fn new(key: SortKey, direction: SortDirection) -> Self {
        Self { key, direction }
    }

    /// Creates a new `SortState` with the given key and its default direction.
    #[allow(dead_code)]
    fn default_for_key(key: SortKey) -> Self {
        Self::new(key, key.default_direction())
    }

    /// Toggles the sort direction between ascending and descending.
    pub fn toggle_direction(&mut self) {
        self.direction = self.direction.toggle();
    }

    /// Toggles the sort key. If the new key is the same as the current key, also toggles the direction.
    pub fn toggle_key(&mut self, new_key: SortKey) {
        if self.key == new_key {
            self.toggle_direction();
        } else {
            self.key = new_key;
            self.direction = new_key.default_direction();
        }
    }

    /// Returns a human-readable label for the current sort state, combining the key and direction.
    pub fn label(&self) -> CompactString {
        format_compact!("{} {}", self.key.as_ref(), self.direction.as_ref())
    }
}

/// One history sample used by the top charts.
#[derive(Debug, Clone, Copy)]
pub struct HistoryPoint {
    /// Sum of RSS for all scanned processes, in bytes.
    pub rss_bytes: u64,
    /// Sum of swap for all scanned processes, in bytes.
    pub swap_bytes: u64,
}

/// Aggregated machine-wide and process-wide memory summary for the status panel.
///
/// Values are derived from `/proc/meminfo` plus a full scan of processes.
#[derive(Debug, Clone)]
pub struct SystemSummary {
    /// Total physical memory reported by `/proc/meminfo`, in bytes.
    pub mem_total: u64,
    /// Estimated memory available without swapping, in bytes.
    pub mem_available: Option<u64>,
    /// Free physical memory, in bytes.
    pub mem_free: u64,
    /// Used physical memory derived from total and available/free, in bytes.
    pub mem_used: u64,
    /// Total configured swap, in bytes.
    pub swap_total: u64,
    /// Free swap, in bytes.
    pub swap_free: u64,
    /// Used swap derived from total and free, in bytes.
    pub swap_used: u64,
    /// Sum of process RSS values currently observed, in bytes.
    pub total_process_rss: u64,
    /// Sum of process swap values currently observed, in bytes.
    pub total_process_swap: u64,
    /// Number of processes included in the snapshot.
    pub process_count: usize,
}

/// One row in the process table.
///
/// Lightweight fields such as RSS and base swap are collected every refresh.
/// Heavier values such as USS and PSS are filled only for the visible rows.
/// This keeps the table responsive while still offering detailed memory stats.
#[derive(Debug, Clone)]
pub struct ProcessRow {
    /// Process ID.
    pub pid: i32,
    /// Parent process ID.
    pub ppid: i32,
    /// Real or effective owner UID used for display.
    pub owner_uid: u32,
    /// Thread count.
    pub threads: u64,
    /// Short process name from `/proc/<pid>/status`.
    pub name: String,
    /// Joined command line, truncated for display.
    pub command: String,
    /// Resident set size in bytes.
    pub rss_bytes: u64,
    /// Unique set size in bytes, populated from `smaps_rollup` when visible.
    pub uss_bytes: Option<u64>,
    /// Proportional set size in bytes, populated from `smaps_rollup` when visible.
    pub pss_bytes: Option<u64>,
    /// Cheap swap estimate from `/proc/<pid>/status`, in bytes.
    pub base_swap_bytes: u64,
    /// Swap value from `smaps_rollup`, in bytes, when available.
    pub detailed_swap_bytes: Option<u64>,
    /// CPU percentage calculated from two samples.
    pub cpu_percent: f32,
}

impl ProcessRow {
    /// Returns the best swap value currently available for display.
    ///
    /// `smaps_rollup` data wins when it has already been loaded for this row.
    pub fn visible_swap_bytes(&self) -> u64 {
        self.detailed_swap_bytes.unwrap_or(self.base_swap_bytes)
    }
}

/// Immutable view of one collection cycle.
///
/// `Snapshot` is the boundary between collection and rendering: UI components
/// do not touch `/proc` directly and only read from this struct.
#[derive(Debug, Clone)]
pub struct Snapshot {
    /// Time when this snapshot was captured.
    pub captured_at: Instant,
    /// Aggregated summary for the status panel and graphs.
    pub system: SystemSummary,
    /// Process rows shown in the main table.
    pub processes: Vec<ProcessRow>,
}
