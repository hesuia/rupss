use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortKey {
    Rss,
    Swap,
    Pss,
    Cpu,
}

#[derive(Debug, Clone, Copy)]
pub struct HistoryPoint {
    pub rss_bytes: u64,
    pub swap_bytes: u64,
}

#[derive(Debug, Clone)]
pub struct SystemSummary {
    pub mem_total: u64,
    pub mem_available: Option<u64>,
    pub mem_free: u64,
    pub mem_used: u64,
    pub swap_total: u64,
    pub swap_free: u64,
    pub swap_used: u64,
    pub total_process_rss: u64,
    pub total_process_swap: u64,
    pub process_count: usize,
}

#[derive(Debug, Clone)]
pub struct ProcessRow {
    pub pid: i32,
    pub ppid: i32,
    pub owner_uid: u32,
    pub threads: u64,
    pub name: String,
    pub command: String,
    pub rss_bytes: u64,
    pub uss_bytes: Option<u64>,
    pub pss_bytes: Option<u64>,
    pub base_swap_bytes: u64,
    pub detailed_swap_bytes: Option<u64>,
    pub cpu_percent: f32,
}

impl ProcessRow {
    pub fn visible_swap_bytes(&self) -> u64 {
        self.detailed_swap_bytes.unwrap_or(self.base_swap_bytes)
    }
}

#[derive(Debug, Clone)]
pub struct Snapshot {
    pub captured_at: Instant,
    pub system: SystemSummary,
    pub processes: Vec<ProcessRow>,
}
