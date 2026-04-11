use crate::{app::ViewMode, collector::VisibleDetailRequest};
use strum::{EnumCount, EnumIter, EnumString, IntoEnumIterator, IntoStaticStr};

#[derive(Debug, Clone, Copy, PartialEq, Eq, EnumCount, EnumIter, EnumString, IntoStaticStr)]
#[strum(serialize_all = "snake_case")]
pub(crate) enum ProcessColumn {
    Pid,
    Ppid,
    Owner,
    Thread,
    Name,
    Command,
    Rss,
    Uss,
    Pss,
    Swap,
    Cpu,
}

impl ProcessColumn {
    pub(crate) fn id(self) -> &'static str {
        self.into()
    }

    pub(crate) fn try_from_id(id: &str) -> Option<Self> {
        id.parse().ok()
    }

    pub(crate) fn title(self) -> &'static str {
        match self {
            Self::Pid => "PID",
            Self::Ppid => "PPID",
            Self::Owner => "OWNER",
            Self::Thread => "THREAD",
            Self::Name => "NAME",
            Self::Command => "COMMAND",
            Self::Rss => "RSS",
            Self::Uss => "USS",
            Self::Pss => "PSS",
            Self::Swap => "SWAP",
            Self::Cpu => "CPU",
        }
    }

    pub(crate) const fn index(self) -> usize {
        match self {
            Self::Pid => 0,
            Self::Ppid => 1,
            Self::Owner => 2,
            Self::Thread => 3,
            Self::Name => 4,
            Self::Command => 5,
            Self::Rss => 6,
            Self::Uss => 7,
            Self::Pss => 8,
            Self::Swap => 9,
            Self::Cpu => 10,
        }
    }

    pub(crate) fn width(self, view_mode: ViewMode) -> u16 {
        match view_mode {
            ViewMode::Flat => match self {
                Self::Pid => 7,
                Self::Ppid => 7,
                Self::Owner => 12,
                Self::Thread => 8,
                Self::Name => 24,
                Self::Command => 24,
                Self::Rss => 12,
                Self::Uss => 12,
                Self::Pss => 12,
                Self::Swap => 12,
                Self::Cpu => 8,
            },
            ViewMode::Tree => match self {
                Self::Pid => 7,
                Self::Ppid => 7,
                Self::Owner => 12,
                Self::Thread => 8,
                Self::Name => 32,
                Self::Command => 16,
                Self::Rss => 12,
                Self::Uss => 12,
                Self::Pss => 12,
                Self::Swap => 12,
                Self::Cpu => 8,
            },
        }
    }

    pub(crate) fn is_toggleable(self) -> bool {
        self != Self::Name
    }

    pub(crate) fn from_sort_key(sort_key: crate::snapshot::SortKey) -> Option<Self> {
        match sort_key {
            crate::snapshot::SortKey::Pid => Some(Self::Pid),
            crate::snapshot::SortKey::Ppid => Some(Self::Ppid),
            crate::snapshot::SortKey::Owner => Some(Self::Owner),
            crate::snapshot::SortKey::Name => Some(Self::Name),
            crate::snapshot::SortKey::Command => Some(Self::Command),
            crate::snapshot::SortKey::Rss => Some(Self::Rss),
            crate::snapshot::SortKey::Swap => Some(Self::Swap),
            crate::snapshot::SortKey::Cpu => Some(Self::Cpu),
        }
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ColumnConfigV1 {
    pub(crate) version: u8,
    pub(crate) order: Vec<String>,
    pub(crate) hidden: Vec<String>,
}

impl Default for ColumnConfigV1 {
    fn default() -> Self {
        Self {
            version: 1,
            order: ProcessColumn::iter().map(|c| c.id().to_string()).collect(),
            hidden: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ColumnLayout {
    order: Vec<ProcessColumn>,
    visible: [bool; ProcessColumn::COUNT],
}

impl Default for ColumnLayout {
    fn default() -> Self {
        Self::new_default()
    }
}

impl ColumnLayout {
    pub(crate) fn new_default() -> Self {
        Self {
            order: ProcessColumn::iter().collect(),
            visible: [true; ProcessColumn::COUNT],
        }
    }

    pub(crate) fn ordered_columns(&self) -> &[ProcessColumn] {
        &self.order
    }

    pub(crate) fn column_at(&self, index: usize) -> Option<ProcessColumn> {
        self.order.get(index).copied()
    }

    pub(crate) fn is_visible(&self, column: ProcessColumn) -> bool {
        self.visible[column.index()]
    }

    pub(crate) fn visible_columns(&self) -> impl Iterator<Item = ProcessColumn> + '_ {
        self.order
            .iter()
            .copied()
            .filter(|column| self.is_visible(*column))
    }

    pub(crate) fn visible_count(&self) -> usize {
        self.visible.iter().filter(|visible| **visible).count()
    }

    pub(crate) fn toggle(&mut self, column: ProcessColumn) -> bool {
        if !column.is_toggleable() || (self.visible_count() == 1 && self.is_visible(column)) {
            return false;
        }

        let index = column.index();
        self.visible[index] = !self.visible[index];
        true
    }

    pub(crate) fn move_in_order(&mut self, index: usize, delta: isize) -> Option<usize> {
        let target = if delta < 0 {
            index.checked_sub(delta.unsigned_abs())?
        } else {
            index.checked_add(delta as usize)?
        };
        if target >= self.order.len() {
            return None;
        }

        self.order.swap(index, target);
        Some(target)
    }

    pub(crate) fn detail_request(&self) -> VisibleDetailRequest {
        VisibleDetailRequest {
            uss: self.is_visible(ProcessColumn::Uss),
            pss: self.is_visible(ProcessColumn::Pss),
        }
    }

    #[allow(dead_code)]
    pub(crate) fn from_config(config: &ColumnConfigV1) -> Self {
        let mut layout = Self {
            order: Vec::new(),
            visible: [true; ProcessColumn::COUNT],
        };
        layout.apply_config(config);
        layout
    }

    #[allow(dead_code)]
    pub(crate) fn to_config_v1(&self) -> ColumnConfigV1 {
        ColumnConfigV1 {
            version: 1,
            order: self.order.iter().map(|c| c.id().to_string()).collect(),
            hidden: ProcessColumn::iter()
                .filter(|column| !self.is_visible(*column))
                .map(|column| column.id().to_string())
                .collect(),
        }
    }

    fn apply_config(&mut self, config: &ColumnConfigV1) {
        // Order: keep only known columns, remove duplicates, then append missing defaults.
        let mut seen = [false; ProcessColumn::COUNT];
        for id in &config.order {
            let Some(column) = ProcessColumn::try_from_id(id) else {
                continue;
            };
            if seen[column.index()] {
                continue;
            }
            seen[column.index()] = true;
            self.order.push(column);
        }
        for column in ProcessColumn::iter() {
            if !seen[column.index()] {
                self.order.push(column);
            }
        }

        // Visibility: start from all visible, then apply hidden list.
        self.visible = [true; ProcessColumn::COUNT];
        for id in &config.hidden {
            let Some(column) = ProcessColumn::try_from_id(id) else {
                continue;
            };
            if column == ProcessColumn::Name {
                continue;
            }
            self.visible[column.index()] = false;
        }

        // Enforce invariants.
        self.visible[ProcessColumn::Name.index()] = true;
        if self.visible_count() == 0 {
            self.visible[ProcessColumn::Name.index()] = true;
            self.visible[ProcessColumn::Rss.index()] = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ColumnConfigV1, ColumnLayout, ProcessColumn};
    use strum::{EnumCount, IntoEnumIterator};

    #[test]
    fn config_sanitizes_unknown_and_duplicates_and_missing() {
        let config = ColumnConfigV1 {
            version: 1,
            order: vec!["pid".into(), "pid".into(), "unknown".into(), "name".into()],
            hidden: vec![],
        };
        let layout = ColumnLayout::from_config(&config);

        assert_eq!(layout.ordered_columns()[0], ProcessColumn::Pid);
        assert_eq!(layout.ordered_columns()[1], ProcessColumn::Name);
        assert_eq!(layout.ordered_columns().len(), ProcessColumn::COUNT);
    }

    #[test]
    fn config_forces_name_visible() {
        let config = ColumnConfigV1 {
            version: 1,
            order: vec![],
            hidden: vec!["name".into()],
        };
        let layout = ColumnLayout::from_config(&config);

        assert!(layout.is_visible(ProcessColumn::Name));
    }

    #[test]
    fn config_never_hides_all_columns() {
        let config = ColumnConfigV1 {
            version: 1,
            order: vec![],
            hidden: ProcessColumn::iter().map(|c| c.id().to_string()).collect(),
        };
        let layout = ColumnLayout::from_config(&config);

        assert!(layout.visible_count() >= 1);
        assert!(layout.is_visible(ProcessColumn::Name));
    }

    #[test]
    fn to_config_roundtrips_visibility_and_order_semantics() {
        let mut layout = ColumnLayout::new_default();
        assert!(layout.move_in_order(0, 1).is_some());
        assert!(layout.toggle(ProcessColumn::Uss));
        let config = layout.to_config_v1();
        let loaded = ColumnLayout::from_config(&config);

        assert_eq!(loaded.ordered_columns()[0], ProcessColumn::Ppid);
        assert!(!loaded.is_visible(ProcessColumn::Uss));
    }
}
