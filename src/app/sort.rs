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
        SortKey::Swap => left.swap_bytes.cmp(&right.swap_bytes),
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

#[cfg(test)]
mod tests {
    use super::compare_process_rows;
    use crate::app::owners::OwnerNameResolver;
    use crate::snapshot::{ProcessRow, SortDirection, SortKey, SortState};
    use std::collections::HashMap;

    struct TestOwnerLookup {
        owners: HashMap<u32, String>,
    }

    impl OwnerNameResolver for TestOwnerLookup {
        fn owner_name(&self, uid: u32) -> String {
            self.owners
                .get(&uid)
                .cloned()
                .unwrap_or_else(|| uid.to_string())
        }
    }

    fn sample_row(pid: i32) -> ProcessRow {
        ProcessRow {
            pid,
            ppid: 1,
            owner_uid: 0,
            threads: 1,
            name: "name".to_string(),
            command: "cmd".to_string(),
            rss_bytes: pid as u64,
            uss_bytes: None,
            pss_bytes: Some(pid as u64),
            swap_bytes: pid as u64,
            cpu_percent: pid as f32,
        }
    }

    #[test]
    fn sort_prefers_highest_metric() {
        let owners = TestOwnerLookup {
            owners: HashMap::new(),
        };
        let ordering = compare_process_rows(
            SortState::new(SortKey::Cpu, SortDirection::Descending),
            &owners,
            &sample_row(10),
            &sample_row(20),
        );
        assert_eq!(ordering, std::cmp::Ordering::Greater);
    }

    #[test]
    fn sort_by_pid_is_ascending() {
        let owners = TestOwnerLookup {
            owners: HashMap::new(),
        };
        let ordering = compare_process_rows(
            SortState::new(SortKey::Pid, SortDirection::Ascending),
            &owners,
            &sample_row(10),
            &sample_row(20),
        );
        assert_eq!(ordering, std::cmp::Ordering::Less);
    }

    #[test]
    fn sort_by_owner_is_case_insensitive() {
        let mut owner_names = HashMap::new();
        owner_names.insert(1000, "Alice".to_string());
        owner_names.insert(1001, "bob".to_string());
        let owners = TestOwnerLookup {
            owners: owner_names,
        };

        let mut left = sample_row(10);
        left.owner_uid = 1000;
        let mut right = sample_row(20);
        right.owner_uid = 1001;

        let ordering = compare_process_rows(
            SortState::new(SortKey::Owner, SortDirection::Ascending),
            &owners,
            &left,
            &right,
        );
        assert_eq!(ordering, std::cmp::Ordering::Less);
    }
}
