use super::{
    AppState, ViewMode,
    columns::{ColumnConfigV1, ColumnLayout},
    filter::MetricFilterOperator,
    state::FilterModalState,
};
use crate::snapshot::{SortDirection, SortKey, SortState};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::{fs, io, path::PathBuf};
use strum::IntoEnumIterator;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct AppConfigV1 {
    #[serde(default = "config_version")]
    version: u8,
    #[serde(default)]
    columns: ColumnConfigV1,
    #[serde(default)]
    sort: SortConfigV1,
    #[serde(default)]
    view_mode: String,
    #[serde(default)]
    filter: FilterConfigV1,
}

impl Default for AppConfigV1 {
    fn default() -> Self {
        Self {
            version: config_version(),
            columns: ColumnConfigV1::default(),
            sort: SortConfigV1::default(),
            view_mode: view_mode_id(ViewMode::Flat).to_string(),
            filter: FilterConfigV1::default(),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ConfigStore {
    path: PathBuf,
}

impl ConfigStore {
    pub(crate) fn xdg() -> Option<Self> {
        ProjectDirs::from("", "", "rupss").map(|dirs| Self {
            path: dirs.config_dir().join("config.toml"),
        })
    }

    #[cfg(test)]
    pub(crate) fn new_for_test(path: PathBuf) -> Self {
        Self { path }
    }

    pub(crate) fn load(&self) -> AppConfigV1 {
        fs::read_to_string(&self.path)
            .ok()
            .and_then(|text| toml::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub(crate) fn save(&self, config: &AppConfigV1) -> io::Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let text = toml::to_string_pretty(config).map_err(io::Error::other)?;
        fs::write(&self.path, text)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SortConfigV1 {
    #[serde(default = "default_sort_key")]
    key: String,
    #[serde(default = "default_sort_direction")]
    direction: String,
}

impl Default for SortConfigV1 {
    fn default() -> Self {
        Self {
            key: default_sort_key(),
            direction: default_sort_direction(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct FilterConfigV1 {
    #[serde(default)]
    pid: String,
    #[serde(default)]
    ppid: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    command: String,
    #[serde(default = "default_filter_operator")]
    rss_op: String,
    #[serde(default)]
    rss_value: String,
    #[serde(default = "default_filter_operator")]
    swap_op: String,
    #[serde(default)]
    swap_value: String,
    #[serde(default = "default_filter_operator")]
    cpu_op: String,
    #[serde(default)]
    cpu_value: String,
    #[serde(default = "default_filter_operator")]
    uss_op: String,
    #[serde(default)]
    uss_value: String,
    #[serde(default = "default_filter_operator")]
    pss_op: String,
    #[serde(default)]
    pss_value: String,
}

impl Default for FilterConfigV1 {
    fn default() -> Self {
        let modal = FilterModalState::default();
        Self::from_modal(&modal)
    }
}

impl AppConfigV1 {
    pub(crate) fn from_app(app: &AppState) -> Self {
        Self {
            version: config_version(),
            columns: app.view.columns.to_config_v1(),
            sort: SortConfigV1::from_sort_state(app.view.sort_state),
            view_mode: view_mode_id(app.view.view_mode).to_string(),
            filter: FilterConfigV1::from_modal(&app.view.filter_modal),
        }
    }

    pub(crate) fn apply_to_app(&self, app: &mut AppState) {
        app.view.columns = ColumnLayout::from_config(&self.columns);
        app.view.sort_state = self.sort.to_sort_state();
        app.view.view_mode = parse_view_mode(&self.view_mode);
        self.filter.apply_to_modal(&mut app.view.filter_modal);

        let (filter, error) = super::filter::try_build_filter_from_modal(&app.view.filter_modal);
        app.view.filter = filter;
        app.view.filter_modal.error = error;
        app.view.column_picker_index = app
            .view
            .column_picker_index
            .min(app.view.columns.ordered_columns().len().saturating_sub(1));
    }
}

impl SortConfigV1 {
    fn from_sort_state(sort_state: SortState) -> Self {
        Self {
            key: sort_key_id(sort_state.key).to_string(),
            direction: sort_direction_id(sort_state.direction).to_string(),
        }
    }

    fn to_sort_state(&self) -> SortState {
        let key = parse_sort_key(&self.key);
        let direction =
            parse_sort_direction(&self.direction).unwrap_or_else(|| key.default_direction());
        SortState::new(key, direction)
    }
}

impl FilterConfigV1 {
    fn from_modal(modal: &FilterModalState) -> Self {
        Self {
            pid: modal.pid.clone(),
            ppid: modal.ppid.clone(),
            name: modal.name.clone(),
            command: modal.command.clone(),
            rss_op: metric_operator_id(modal.rss_op).to_string(),
            rss_value: modal.rss_value.clone(),
            swap_op: metric_operator_id(modal.swap_op).to_string(),
            swap_value: modal.swap_value.clone(),
            cpu_op: metric_operator_id(modal.cpu_op).to_string(),
            cpu_value: modal.cpu_value.clone(),
            uss_op: metric_operator_id(modal.uss_op).to_string(),
            uss_value: modal.uss_value.clone(),
            pss_op: metric_operator_id(modal.pss_op).to_string(),
            pss_value: modal.pss_value.clone(),
        }
    }

    fn apply_to_modal(&self, modal: &mut FilterModalState) {
        modal.open = false;
        modal.selected = 0;
        modal.editing = false;
        modal.error = None;
        modal.pid.clone_from(&self.pid);
        modal.ppid.clone_from(&self.ppid);
        modal.name.clone_from(&self.name);
        modal.command.clone_from(&self.command);
        modal.rss_op = parse_metric_operator(&self.rss_op);
        modal.rss_value.clone_from(&self.rss_value);
        modal.swap_op = parse_metric_operator(&self.swap_op);
        modal.swap_value.clone_from(&self.swap_value);
        modal.cpu_op = parse_metric_operator(&self.cpu_op);
        modal.cpu_value.clone_from(&self.cpu_value);
        modal.uss_op = parse_metric_operator(&self.uss_op);
        modal.uss_value.clone_from(&self.uss_value);
        modal.pss_op = parse_metric_operator(&self.pss_op);
        modal.pss_value.clone_from(&self.pss_value);
    }
}

fn config_version() -> u8 {
    1
}

fn default_sort_key() -> String {
    sort_key_id(SortKey::Rss).to_string()
}

fn default_sort_direction() -> String {
    sort_direction_id(SortDirection::Descending).to_string()
}

fn default_filter_operator() -> String {
    metric_operator_id(MetricFilterOperator::GreaterThanOrEqual).to_string()
}

fn sort_key_id(key: SortKey) -> &'static str {
    key.into()
}

fn sort_direction_id(direction: SortDirection) -> &'static str {
    direction.into()
}

fn view_mode_id(view_mode: ViewMode) -> &'static str {
    view_mode.into()
}

fn metric_operator_id(operator: MetricFilterOperator) -> &'static str {
    operator.into()
}

fn parse_sort_key(raw: &str) -> SortKey {
    SortKey::iter()
        .find(|key| sort_key_id(*key) == raw)
        .unwrap_or(SortKey::Rss)
}

fn parse_sort_direction(raw: &str) -> Option<SortDirection> {
    [SortDirection::Ascending, SortDirection::Descending]
        .into_iter()
        .find(|direction| sort_direction_id(*direction) == raw)
}

fn parse_view_mode(raw: &str) -> ViewMode {
    match raw {
        "tree" => ViewMode::Tree,
        _ => ViewMode::Flat,
    }
}

fn parse_metric_operator(raw: &str) -> MetricFilterOperator {
    MetricFilterOperator::iter()
        .find(|operator| metric_operator_id(*operator) == raw)
        .unwrap_or(MetricFilterOperator::GreaterThanOrEqual)
}

#[cfg(test)]
mod tests {
    use super::{AppConfigV1, ConfigStore};
    use crate::{
        app::{AppState, ProcessColumn, ViewMode},
        snapshot::{SortDirection, SortKey, SortState},
    };

    #[test]
    fn config_roundtrips_view_preferences() {
        let mut app = AppState::default();
        app.view.columns.move_in_order(0, 1);
        app.view.columns.toggle(ProcessColumn::Uss);
        app.view.sort_state = SortState::new(SortKey::Pid, SortDirection::Ascending);
        app.view.view_mode = ViewMode::Tree;
        app.view.filter_modal.name = "postgres".to_string();
        app.view.filter_modal.rss_value = "100MB".to_string();

        let serialized = toml::to_string(&AppConfigV1::from_app(&app)).unwrap();
        let loaded: AppConfigV1 = toml::from_str(&serialized).unwrap();

        let mut restored = AppState::default();
        loaded.apply_to_app(&mut restored);

        assert_eq!(
            restored.view.columns.ordered_columns()[0],
            ProcessColumn::Ppid
        );
        assert!(!restored.view.columns.is_visible(ProcessColumn::Uss));
        assert_eq!(
            restored.view.sort_state,
            SortState::new(SortKey::Pid, SortDirection::Ascending)
        );
        assert_eq!(restored.view.view_mode, ViewMode::Tree);
        assert_eq!(restored.view.filter_modal.name, "postgres");
        assert!(restored.view.filter.is_active());
    }

    #[test]
    fn invalid_config_values_fall_back_to_defaults() {
        let config: AppConfigV1 = toml::from_str(
            r#"
            view_mode = "grid"

            [sort]
            key = "missing"
            direction = "sideways"
            "#,
        )
        .unwrap();
        let mut app = AppState::default();
        config.apply_to_app(&mut app);

        assert_eq!(
            app.view.sort_state,
            SortState::new(SortKey::Rss, SortDirection::Descending)
        );
        assert_eq!(app.view.view_mode, ViewMode::Flat);
    }

    #[test]
    fn corrupt_config_loads_as_default() {
        let temp = tempfile_path("rupss-corrupt-config.toml");
        std::fs::write(&temp, "not = [valid").unwrap();
        let config = ConfigStore::new_for_test(temp.clone()).load();

        assert_eq!(
            config.sort.to_sort_state(),
            SortState::new(SortKey::Rss, SortDirection::Descending)
        );
        let _ = std::fs::remove_file(temp);
    }

    fn tempfile_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("{}-{}", name, std::process::id()))
    }
}
