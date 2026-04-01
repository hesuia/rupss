use crate::snapshot::{ProcessRow, SortDirection, SortKey, SortState};
use std::{borrow::Cow, cmp::Ordering, collections::HashMap};

/// Compares two rows using the active sort key and PID as a stable tie-breaker.
///
/// A deterministic tie-breaker keeps the table stable across refreshes.
pub(super) fn compare_process_rows(
    sort_state: SortState,
    username_cache: &HashMap<u32, String>,
    left: &ProcessRow,
    right: &ProcessRow,
) -> Ordering {
    let primary = match sort_state.key {
        SortKey::Pid => left.pid.cmp(&right.pid),
        SortKey::Ppid => left.ppid.cmp(&right.ppid),
        SortKey::Owner => {
            let left_owner = owner_display_name(username_cache, left.owner_uid);
            let right_owner = owner_display_name(username_cache, right.owner_uid);
            left_owner.cmp(&right_owner)
        }
        SortKey::Name => left.name.to_lowercase().cmp(&right.name.to_lowercase()),
        SortKey::Command => left
            .command
            .to_lowercase()
            .cmp(&right.command.to_lowercase()),
        SortKey::Rss => left.rss_bytes.cmp(&right.rss_bytes),
        SortKey::Swap => left.visible_swap_bytes().cmp(&right.visible_swap_bytes()),
        SortKey::Cpu => left
            .cpu_percent
            .partial_cmp(&right.cpu_percent)
            .unwrap_or(Ordering::Equal),
    };

    let primary = match sort_state.direction {
        SortDirection::Ascending => primary,
        SortDirection::Descending => primary.reverse(),
    };

    primary.then_with(|| left.pid.cmp(&right.pid))
}

/// Sorts the process list in place using the current table policy.
///
/// Sorting is done after each refresh and whenever the user changes sort key.
pub(super) fn sort_processes(
    sort_state: SortState,
    username_cache: &HashMap<u32, String>,
    processes: &mut [ProcessRow],
) {
    processes.sort_by(|left, right| compare_process_rows(sort_state, username_cache, left, right));
}

pub(super) fn owner_display_name<'a>(
    username_cache: &'a HashMap<u32, String>,
    uid: u32,
) -> Cow<'a, str> {
    username_cache
        .get(&uid)
        .map(|name| Cow::Borrowed(name.as_str()))
        .unwrap_or_else(|| Cow::Owned(format!("uid:{}", uid)))
}
