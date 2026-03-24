use crate::format::truncate_owned;
use crate::snapshot::{HistoryPoint, ProcessRow, Snapshot, SystemSummary};
use procfs::prelude::Current;
use procfs::process::{Process, all_processes};
use procfs::{Meminfo, page_size, ticks_per_second};
use std::collections::HashMap;
use std::time::Instant;

const MAX_NAME_LEN: usize = 32;
const MAX_COMMAND_LEN: usize = 96;
const KIB: u64 = 1024;

#[derive(Debug, Clone, Copy)]
pub struct CpuSample {
    pub total_time_ticks: u64,
    pub captured_at: Instant,
}

#[derive(Debug, Clone)]
pub struct DetailedMemorySample {
    pub uss_bytes: Option<u64>,
    pub pss_bytes: Option<u64>,
    pub swap_bytes: Option<u64>,
}

pub trait SystemCollector {
    fn collect_base_snapshot(
        &self,
        previous_cpu: &HashMap<i32, CpuSample>,
        now: Instant,
    ) -> Result<(Snapshot, HashMap<i32, CpuSample>, HistoryPoint), procfs::ProcError>;

    fn collect_visible_memory_details(&self, pids: &[i32]) -> HashMap<i32, DetailedMemorySample>;
}

#[derive(Debug, Clone, Copy)]
pub struct ProcfsCollector {
    page_size: u64,
    ticks_per_second: u64,
}

impl ProcfsCollector {
    pub fn new() -> Self {
        Self {
            page_size: page_size(),
            ticks_per_second: ticks_per_second(),
        }
    }

    fn collect_one_process(
        &self,
        process: Process,
        previous_cpu: &HashMap<i32, CpuSample>,
        now: Instant,
    ) -> Option<(ProcessRow, CpuSample)> {
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
        let mut processes = Vec::new();
        let mut next_cpu = HashMap::new();
        let mut total_process_rss = 0u64;
        let mut total_process_swap = 0u64;

        for process in all_processes()? {
            let Ok(process) = process else {
                continue;
            };

            let Some((row, cpu_sample)) = self.collect_one_process(process, previous_cpu, now)
            else {
                continue;
            };

            total_process_rss = total_process_rss.saturating_add(row.rss_bytes);
            total_process_swap = total_process_swap.saturating_add(row.base_swap_bytes);
            next_cpu.insert(row.pid, cpu_sample);
            processes.push(row);
        }

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
        let mut details = HashMap::new();

        for pid in pids {
            let Ok(process) = Process::new(*pid) else {
                continue;
            };
            let Ok(rollup) = process.smaps_rollup() else {
                continue;
            };
            let Some(memory_map) = rollup.memory_map_rollup.0.first() else {
                continue;
            };
            let map = &memory_map.extension.map;
            let uss_bytes = map
                .get("Private_Clean")
                .copied()
                .unwrap_or(0)
                .saturating_add(map.get("Private_Dirty").copied().unwrap_or(0));
            let pss_bytes = map.get("Pss").copied();
            let swap_bytes = map.get("Swap").copied();

            details.insert(
                *pid,
                DetailedMemorySample {
                    uss_bytes: Some(uss_bytes),
                    pss_bytes,
                    swap_bytes,
                },
            );
        }

        details
    }
}

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
