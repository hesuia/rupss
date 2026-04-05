use super::owners::OwnerNameResolver;
use crate::snapshot::{ProcessRow, SortDirection, SortKey, SortState};
use std::cmp::Ordering;

/// Compares two rows using the active sort key and PID as a stable tie-breaker.
///
/// A deterministic tie-breaker keeps the table stable across refreshes.
pub(super) fn compare_process_rows<L: OwnerNameResolver + ?Sized>(
    sort_state: SortState,
    owner_lookup: &L,
    left: &ProcessRow,
    right: &ProcessRow,
) -> Ordering {
    let primary = match sort_state.key {
        SortKey::Pid => left.pid.cmp(&right.pid),
        SortKey::Ppid => left.ppid.cmp(&right.ppid),
        SortKey::Owner => {
            let left_owner = owner_lookup.owner_name(left.owner_uid).to_lowercase();
            let right_owner = owner_lookup.owner_name(right.owner_uid).to_lowercase();
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
pub(super) fn sort_processes<L: OwnerNameResolver + ?Sized>(
    sort_state: SortState,
    owner_lookup: &L,
    processes: &mut [ProcessRow],
) {
    processes.sort_by(|left, right| compare_process_rows(sort_state, owner_lookup, left, right));
}
