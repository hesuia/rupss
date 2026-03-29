use crate::{
    format::truncate_owned,
    snapshot::{HistoryPoint, ProcessRow, Snapshot, SystemSummary},
};
use procfs::{
    Meminfo, page_size,
    prelude::Current,
    process::{Process, all_processes},
    ticks_per_second,
};
use std::{collections::HashMap, time::Instant};

const MAX_NAME_LEN: usize = 32;
const MAX_COMMAND_LEN: usize = 96;
const KIB: u64 = 1024;

/// Previous CPU counters for one process.
///
/// CPU usage is calculated from the difference between two samples, so the app
/// stores the last observed tick count and timestamp per PID. This struct is
/// intentionally small to keep the cache lightweight.
#[derive(Debug, Clone, Copy)]
pub struct CpuSample {
    /// Sum of user and system ticks from `/proc/<pid>/stat`.
    pub total_time_ticks: u64,
    /// Time when the tick count was captured.
    pub captured_at: Instant,
}

/// Memory details resolved lazily from `smaps_rollup`.
///
/// These values are not available from cheap sources like `/proc/<pid>/status`,
/// so they are collected only for visible rows to keep the tool lightweight.
#[derive(Debug, Clone)]
pub struct DetailedMemorySample {
    /// Unique set size in bytes.
    pub uss_bytes: Option<u64>,
    /// Proportional set size in bytes.
    pub pss_bytes: Option<u64>,
    /// Swap usage reported by `smaps_rollup`, in bytes.
    pub swap_bytes: Option<u64>,
}

/// Abstracts process and memory collection behind a swappable interface.
///
/// The current implementation uses `procfs`, but the rest of the application
/// only depends on this trait so a custom parser can replace it later.
/// This is the key seam that keeps parsing logic isolated from UI and state.
pub trait SystemCollector {
    /// Collects the lightweight full-process snapshot used for every refresh.
    ///
    /// Expected work:
    /// - Scan all processes and gather cheap fields (RSS, swap, owner, threads, command).
    /// - Seed CPU usage by capturing the current tick count per PID.
    /// - Aggregate totals used by the top charts and the summary panel.
    ///
    /// Returns:
    /// - The new `Snapshot`.
    /// - An updated CPU cache keyed by PID.
    /// - A `HistoryPoint` to append to the graph buffers.
    fn collect_base_snapshot(
        &self,
        previous_cpu: &HashMap<i32, CpuSample>,
        now: Instant,
    ) -> Result<(Snapshot, HashMap<i32, CpuSample>, HistoryPoint), procfs::ProcError>;

    /// Collects heavier memory details only for the currently visible PIDs.
    ///
    /// Implementations should keep this path narrow because it may read
    /// expensive sources such as `smaps_rollup`. The caller decides visibility.
    fn collect_visible_memory_details(&self, pids: &[i32]) -> HashMap<i32, DetailedMemorySample>;
}

/// `procfs`-backed collector implementation for Linux.
///
/// Uses `/proc` as the source of truth and performs no caching beyond CPU samples.
#[derive(Debug, Clone, Copy)]
pub struct ProcfsCollector {
    page_size: u64,
    ticks_per_second: u64,
}

impl ProcfsCollector {
    /// Creates a collector using the current machine's page size and tick rate.
    ///
    /// These are used to convert `stat.rss` pages and `stat` ticks into bytes/percentages.
    pub fn new() -> Self {
        Self {
            page_size: page_size(),
            ticks_per_second: ticks_per_second(),
        }
    }

    /// Collects a single process row and CPU seed from a live `Process` handle.
    ///
    /// This function intentionally avoids retaining the `Process` object to prevent
    /// holding many open `/proc/<pid>` file descriptors.
    fn collect_one_process(
        &self,
        process: Process,
        previous_cpu: &HashMap<i32, CpuSample>,
        now: Instant,
    ) -> Option<(ProcessRow, CpuSample)> {
        // Drop the `Process` object quickly after copying the fields we need so we do not
        // keep many `/proc/<pid>` directory file descriptors open.
        let pid = process.pid;
        let stat = process.stat().ok()?;
        let status = process.status().ok()?;
        let total_time_ticks = stat.utime.saturating_add(stat.stime);
        let cpu_percent = previous_cpu
            .get(&pid)
            .map(|previous| {
                calculate_cpu_percent(previous, total_time_ticks, now, self.ticks_per_second)
            })
            .unwrap_or(0.0);

        let owner_uid = process.uid().unwrap_or(status.ruid);
        let rss_bytes = status
            .vmrss
            .map(|value| value * KIB)
            .unwrap_or(stat.rss * self.page_size);
        let swap_bytes = status.vmswap.unwrap_or(0) * KIB;

        let name = truncate_owned(&status.name, MAX_NAME_LEN);
        let command = process
            .cmdline()
            .ok()
            .filter(|parts| !parts.is_empty())
            .map(|parts| truncate_owned(&parts.join(" "), MAX_COMMAND_LEN))
            .unwrap_or_else(|| name.clone());

        let row = ProcessRow {
            pid,
            ppid: status.ppid,
            owner_uid,
            threads: status.threads,
            name,
            command,
            rss_bytes,
            uss_bytes: None,
            pss_bytes: None,
            base_swap_bytes: swap_bytes,
            detailed_swap_bytes: None,
            cpu_percent,
        };

        Some((
            row,
            CpuSample {
                total_time_ticks,
                captured_at: now,
            },
        ))
    }
}

impl SystemCollector for ProcfsCollector {
    fn collect_base_snapshot(
        &self,
        previous_cpu: &HashMap<i32, CpuSample>,
        now: Instant,
    ) -> Result<(Snapshot, HashMap<i32, CpuSample>, HistoryPoint), procfs::ProcError> {
        let meminfo = Meminfo::current()?;
        let (processes, next_cpu, total_process_rss, total_process_swap) = all_processes()?
            .filter_map(|proc| proc.ok())
            .filter_map(|proc| self.collect_one_process(proc, previous_cpu, now))
            .fold(
                (Vec::new(), HashMap::new(), 0u64, 0u64),
                |(mut rows, mut cpu_cache, total_rss, total_swap), (row, cpu_sample)| {
                    let total_rss = total_rss.saturating_add(row.rss_bytes);
                    let total_swap = total_swap.saturating_add(row.base_swap_bytes);
                    cpu_cache.insert(row.pid, cpu_sample);
                    rows.push(row);
                    (rows, cpu_cache, total_rss, total_swap)
                },
            );
        let mem_available = meminfo.mem_available;
        let mem_used = meminfo
            .mem_total
            .saturating_sub(mem_available.unwrap_or(meminfo.mem_free));
        let swap_used = meminfo.swap_total.saturating_sub(meminfo.swap_free);

        let snapshot = Snapshot {
            captured_at: now,
            system: SystemSummary {
                mem_total: meminfo.mem_total,
                mem_available,
                mem_free: meminfo.mem_free,
                mem_used,
                swap_total: meminfo.swap_total,
                swap_free: meminfo.swap_free,
                swap_used,
                total_process_rss,
                total_process_swap,
                process_count: processes.len(),
            },
            processes,
        };

        Ok((
            snapshot,
            next_cpu,
            HistoryPoint {
                rss_bytes: total_process_rss,
                swap_bytes: total_process_swap,
            },
        ))
    }

    fn collect_visible_memory_details(&self, pids: &[i32]) -> HashMap<i32, DetailedMemorySample> {
        pids.iter()
            .filter_map(|pid| {
                // Detailed memory metrics are intentionally loaded only for the visible rows.
                let process = Process::new(*pid).ok()?;
                let rollup = process.smaps_rollup().ok()?;
                // let memory_map = rollup.memory_map_rollup.0.first()?;
                // let map = &memory_map.extension.map;
                let map = &rollup.memory_map_rollup.0.first()?.extension.map;
                let uss_bytes = map
                    .get("Private_Clean")
                    .copied()
                    .unwrap_or(0)
                    .saturating_add(map.get("Private_Dirty").copied().unwrap_or(0));

                Some((
                    *pid,
                    DetailedMemorySample {
                        uss_bytes: Some(uss_bytes),
                        pss_bytes: map.get("Pss").copied(),
                        swap_bytes: map.get("Swap").copied(),
                    },
                ))
            })
            .collect()
    }
}

/// Converts a change in process CPU ticks into a human-readable percentage.
///
/// The result is a per-process percentage, not normalized by CPU count.
fn calculate_cpu_percent(
    previous: &CpuSample,
    total_time_ticks: u64,
    now: Instant,
    ticks_per_second: u64,
) -> f32 {
    let elapsed = now
        .saturating_duration_since(previous.captured_at)
        .as_secs_f32();
    if elapsed <= f32::EPSILON || ticks_per_second == 0 {
        return 0.0;
    }

    let delta_ticks = total_time_ticks.saturating_sub(previous.total_time_ticks) as f32;
    (delta_ticks / ticks_per_second as f32) / elapsed * 100.0
}

#[cfg(test)]
mod tests {
    use super::{CpuSample, calculate_cpu_percent};
    use std::time::{Duration, Instant};

    #[test]
    fn calculates_cpu_from_tick_delta() {
        let start = Instant::now();
        let previous = CpuSample {
            total_time_ticks: 100,
            captured_at: start,
        };
        let cpu = calculate_cpu_percent(&previous, 150, start + Duration::from_secs(1), 100);
        assert!((cpu - 50.0).abs() < f32::EPSILON);
    }

    #[test]
    fn guards_zero_elapsed_time() {
        let now = Instant::now();
        let previous = CpuSample {
            total_time_ticks: 100,
            captured_at: now,
        };
        assert_eq!(calculate_cpu_percent(&previous, 200, now, 100), 0.0);
    }
}
