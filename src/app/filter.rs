use crate::{collector::VisibleDetailRequest, snapshot::ProcessRow};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TextFilterField {
    Name,
    Command,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MetricFilterField {
    Rss,
    Swap,
    Cpu,
    Uss,
    Pss,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MetricFilterOperator {
    Equal,
    GreaterThan,
    GreaterThanOrEqual,
    LessThan,
    LessThanOrEqual,
}

impl MetricFilterOperator {
    pub(crate) fn cycle(self, delta: isize) -> Self {
        let all = [
            Self::GreaterThanOrEqual,
            Self::GreaterThan,
            Self::Equal,
            Self::LessThan,
            Self::LessThanOrEqual,
        ];
        let current = all.iter().position(|op| *op == self).unwrap_or(0) as isize;
        let len = all.len() as isize;
        let mut next = (current + delta) % len;
        if next < 0 {
            next += len;
        }
        all[next as usize]
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Equal => "=",
            Self::GreaterThan => ">",
            Self::GreaterThanOrEqual => ">=",
            Self::LessThan => "<",
            Self::LessThanOrEqual => "<=",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum MetricFilterValue {
    Bytes(u64),
    Percent(f32),
}

#[derive(Debug, Clone, PartialEq)]
pub(super) enum FilterPredicate {
    Pid(i32),
    Ppid(i32),
    Text {
        fields: Vec<TextFilterField>,
        query: String,
    },
    Metric {
        field: MetricFilterField,
        operator: MetricFilterOperator,
        value: MetricFilterValue,
    },
}

#[derive(Debug, Clone, Default, PartialEq)]
pub(super) struct ProcessFilter {
    predicates: Vec<FilterPredicate>,
}

impl ProcessFilter {
    #[allow(dead_code)]
    pub(super) fn from_text_query(query: &str) -> Self {
        let query = query.trim();
        if query.is_empty() {
            return Self::default();
        }

        Self {
            predicates: query.split_whitespace().map(parse_filter_token).collect(),
        }
    }

    pub(super) fn is_active(&self) -> bool {
        !self.predicates.is_empty()
    }

    pub(super) fn detail_request(&self) -> VisibleDetailRequest {
        self.predicates
            .iter()
            .fold(VisibleDetailRequest::default(), |mut request, predicate| {
                if let FilterPredicate::Metric { field, .. } = predicate {
                    match field {
                        MetricFilterField::Uss => request.uss = true,
                        MetricFilterField::Pss => request.pss = true,
                        MetricFilterField::Rss
                        | MetricFilterField::Swap
                        | MetricFilterField::Cpu => {}
                    }
                }
                request
            })
    }

    pub(super) fn matches(&self, row: &ProcessRow) -> bool {
        self.predicates
            .iter()
            .all(|predicate| predicate.matches(row))
    }

    pub(super) fn matching_indexes(&self, processes: &[ProcessRow]) -> Vec<usize> {
        processes
            .iter()
            .enumerate()
            .filter_map(|(index, row)| self.matches(row).then_some(index))
            .collect()
    }
}

impl FilterPredicate {
    fn matches(&self, row: &ProcessRow) -> bool {
        match self {
            Self::Pid(pid) => row.pid == *pid,
            Self::Ppid(ppid) => row.ppid == *ppid,
            Self::Text { fields, query } => fields.iter().any(|field| {
                text_filter_value(*field, row)
                    .to_lowercase()
                    .contains(query)
            }),
            Self::Metric {
                field,
                operator,
                value,
            } => metric_matches(metric_filter_value(*field, row), *operator, *value),
        }
    }
}

fn text_filter_value(field: TextFilterField, row: &ProcessRow) -> &str {
    match field {
        TextFilterField::Name => &row.name,
        TextFilterField::Command => &row.command,
    }
}

fn metric_filter_value(field: MetricFilterField, row: &ProcessRow) -> Option<MetricFilterValue> {
    match field {
        MetricFilterField::Rss => Some(MetricFilterValue::Bytes(row.rss_bytes)),
        MetricFilterField::Swap => Some(MetricFilterValue::Bytes(row.swap_bytes)),
        MetricFilterField::Cpu => Some(MetricFilterValue::Percent(row.cpu_percent)),
        MetricFilterField::Uss => row.uss_bytes.map(MetricFilterValue::Bytes),
        MetricFilterField::Pss => row.pss_bytes.map(MetricFilterValue::Bytes),
    }
}

fn metric_matches(
    value: Option<MetricFilterValue>,
    operator: MetricFilterOperator,
    threshold: MetricFilterValue,
) -> bool {
    let Some(value) = value else {
        return false;
    };

    match (value, threshold) {
        (MetricFilterValue::Bytes(value), MetricFilterValue::Bytes(threshold)) => {
            ordered_metric_matches(value, operator, threshold)
        }
        (MetricFilterValue::Percent(value), MetricFilterValue::Percent(threshold)) => {
            ordered_metric_matches(value, operator, threshold)
        }
        _ => false,
    }
}

fn ordered_metric_matches<T: PartialOrd + PartialEq>(
    value: T,
    operator: MetricFilterOperator,
    threshold: T,
) -> bool {
    match operator {
        MetricFilterOperator::Equal => value == threshold,
        MetricFilterOperator::GreaterThan => value > threshold,
        MetricFilterOperator::GreaterThanOrEqual => value >= threshold,
        MetricFilterOperator::LessThan => value < threshold,
        MetricFilterOperator::LessThanOrEqual => value <= threshold,
    }
}

#[allow(dead_code)]
fn parse_filter_token(token: &str) -> FilterPredicate {
    parse_metric_filter_token(token).unwrap_or_else(|| text_predicate(token))
}

#[allow(dead_code)]
fn text_predicate(query: &str) -> FilterPredicate {
    FilterPredicate::Text {
        fields: vec![TextFilterField::Name, TextFilterField::Command],
        query: query.to_lowercase(),
    }
}

#[allow(dead_code)]
fn parse_metric_filter_token(token: &str) -> Option<FilterPredicate> {
    let (field_text, operator, value_text) = split_metric_filter_token(token)?;
    let field = parse_metric_field(field_text)?;
    let value = match field {
        MetricFilterField::Cpu => MetricFilterValue::Percent(parse_cpu_value(value_text)?),
        MetricFilterField::Rss
        | MetricFilterField::Swap
        | MetricFilterField::Uss
        | MetricFilterField::Pss => MetricFilterValue::Bytes(parse_bytes_value(value_text)?),
    };

    Some(FilterPredicate::Metric {
        field,
        operator,
        value,
    })
}

#[allow(dead_code)]
fn split_metric_filter_token(token: &str) -> Option<(&str, MetricFilterOperator, &str)> {
    [
        (">=", MetricFilterOperator::GreaterThanOrEqual),
        ("<=", MetricFilterOperator::LessThanOrEqual),
        (">", MetricFilterOperator::GreaterThan),
        ("<", MetricFilterOperator::LessThan),
        ("=", MetricFilterOperator::Equal),
    ]
    .into_iter()
    .find_map(|(operator_text, operator)| {
        let (field, value) = token.split_once(operator_text)?;
        (!field.is_empty() && !value.is_empty()).then_some((field, operator, value))
    })
}

#[allow(dead_code)]
fn parse_metric_field(field: &str) -> Option<MetricFilterField> {
    match field.to_ascii_lowercase().as_str() {
        "rss" => Some(MetricFilterField::Rss),
        "swap" => Some(MetricFilterField::Swap),
        "cpu" => Some(MetricFilterField::Cpu),
        "uss" => Some(MetricFilterField::Uss),
        "pss" => Some(MetricFilterField::Pss),
        _ => None,
    }
}

fn parse_cpu_value(value: &str) -> Option<f32> {
    let number = value.strip_suffix('%').unwrap_or(value);
    let parsed = number.parse::<f32>().ok()?;
    parsed.is_finite().then_some(parsed)
}

fn parse_bytes_value(value: &str) -> Option<u64> {
    let number_len = value
        .char_indices()
        .take_while(|(_, ch)| ch.is_ascii_digit() || *ch == '.')
        .map(|(index, ch)| index + ch.len_utf8())
        .last()?;
    let number = value[..number_len].parse::<f64>().ok()?;
    if !number.is_finite() || number < 0.0 {
        return None;
    }

    let unit = value[number_len..].to_ascii_lowercase();
    let multiplier = match unit.as_str() {
        "" | "b" => 1.0,
        "kb" => 1024.0,
        "mb" => 1024.0 * 1024.0,
        "gb" => 1024.0 * 1024.0 * 1024.0,
        _ => return None,
    };
    let bytes = number * multiplier;

    (bytes <= u64::MAX as f64).then_some(bytes.round() as u64)
}

pub(super) fn try_build_filter_from_modal(
    modal: &super::state::FilterModalState,
) -> (ProcessFilter, Option<String>) {
    let mut predicates = Vec::new();
    let mut first_error: Option<String> = None;

    let trimmed_pid = modal.pid.trim();
    if !trimmed_pid.is_empty() {
        match trimmed_pid.parse::<i32>() {
            Ok(pid) => predicates.push(FilterPredicate::Pid(pid)),
            Err(_) => {
                if first_error.is_none() {
                    first_error = Some("pid is not a number".to_string());
                }
            }
        }
    }

    let trimmed_ppid = modal.ppid.trim();
    if !trimmed_ppid.is_empty() {
        match trimmed_ppid.parse::<i32>() {
            Ok(ppid) => predicates.push(FilterPredicate::Ppid(ppid)),
            Err(_) => {
                if first_error.is_none() {
                    first_error = Some("ppid is not a number".to_string());
                }
            }
        }
    }

    let trimmed_name = modal.name.trim();
    if !trimmed_name.is_empty() {
        predicates.push(FilterPredicate::Text {
            fields: vec![TextFilterField::Name],
            query: trimmed_name.to_lowercase(),
        });
    }

    let trimmed_command = modal.command.trim();
    if !trimmed_command.is_empty() {
        predicates.push(FilterPredicate::Text {
            fields: vec![TextFilterField::Command],
            query: trimmed_command.to_lowercase(),
        });
    }

    if let Some(predicate) = parse_metric_from_modal_row(
        MetricFilterField::Rss,
        modal.rss_op,
        &modal.rss_value,
        &mut first_error,
    ) {
        predicates.push(predicate);
    }
    if let Some(predicate) = parse_metric_from_modal_row(
        MetricFilterField::Swap,
        modal.swap_op,
        &modal.swap_value,
        &mut first_error,
    ) {
        predicates.push(predicate);
    }
    if let Some(predicate) = parse_metric_from_modal_row(
        MetricFilterField::Cpu,
        modal.cpu_op,
        &modal.cpu_value,
        &mut first_error,
    ) {
        predicates.push(predicate);
    }
    if let Some(predicate) = parse_metric_from_modal_row(
        MetricFilterField::Uss,
        modal.uss_op,
        &modal.uss_value,
        &mut first_error,
    ) {
        predicates.push(predicate);
    }
    if let Some(predicate) = parse_metric_from_modal_row(
        MetricFilterField::Pss,
        modal.pss_op,
        &modal.pss_value,
        &mut first_error,
    ) {
        predicates.push(predicate);
    }

    (ProcessFilter { predicates }, first_error)
}

fn parse_metric_from_modal_row(
    field: MetricFilterField,
    operator: MetricFilterOperator,
    raw: &str,
    first_error: &mut Option<String>,
) -> Option<FilterPredicate> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    let value = match field {
        MetricFilterField::Cpu => parse_cpu_value(trimmed).map(MetricFilterValue::Percent),
        MetricFilterField::Rss
        | MetricFilterField::Swap
        | MetricFilterField::Uss
        | MetricFilterField::Pss => parse_bytes_value(trimmed).map(MetricFilterValue::Bytes),
    };
    match value {
        Some(value) => Some(FilterPredicate::Metric {
            field,
            operator,
            value,
        }),
        None => {
            first_error
                .get_or_insert_with(|| format!("invalid {} value", metric_field_label(field)));
            None
        }
    }
}

fn metric_field_label(field: MetricFilterField) -> &'static str {
    match field {
        MetricFilterField::Rss => "rss",
        MetricFilterField::Swap => "swap",
        MetricFilterField::Cpu => "cpu",
        MetricFilterField::Uss => "uss",
        MetricFilterField::Pss => "pss",
    }
}

#[cfg(test)]
mod tests {
    use super::{ProcessFilter, parse_bytes_value};
    use crate::snapshot::ProcessRow;

    fn sample_row(pid: i32, name: &str, command: &str) -> ProcessRow {
        ProcessRow {
            pid,
            ppid: 1,
            owner_uid: 0,
            threads: 1,
            name: name.to_string(),
            command: command.to_string(),
            rss_bytes: 0,
            uss_bytes: None,
            pss_bytes: None,
            swap_bytes: 0,
            cpu_percent: 0.0,
        }
    }

    #[test]
    fn matches_name() {
        let filter = ProcessFilter::from_text_query("bash");
        let rows = vec![
            sample_row(1, "bash", "/usr/bin/bash"),
            sample_row(2, "zsh", "/usr/bin/zsh"),
        ];

        assert_eq!(filter.matching_indexes(&rows), vec![0]);
    }

    #[test]
    fn matches_command() {
        let filter = ProcessFilter::from_text_query("worker");
        let rows = vec![
            sample_row(1, "python", "python app.py"),
            sample_row(2, "python", "python worker.py"),
        ];

        assert_eq!(filter.matching_indexes(&rows), vec![1]);
    }

    #[test]
    fn text_match_is_case_insensitive() {
        let filter = ProcessFilter::from_text_query("POSTGRES");
        let rows = vec![sample_row(1, "postgres", "/usr/bin/postgres")];

        assert_eq!(filter.matching_indexes(&rows), vec![0]);
    }

    #[test]
    fn empty_filter_matches_all_rows() {
        let filter = ProcessFilter::from_text_query("");
        let rows = vec![
            sample_row(1, "bash", "/usr/bin/bash"),
            sample_row(2, "zsh", "/usr/bin/zsh"),
        ];

        assert_eq!(filter.matching_indexes(&rows), vec![0, 1]);
    }

    #[test]
    fn filters_by_rss_threshold() {
        let mut small = sample_row(1, "small", "small");
        small.rss_bytes = 99 * 1024 * 1024;
        let mut large = sample_row(2, "large", "large");
        large.rss_bytes = 100 * 1024 * 1024;
        let filter = ProcessFilter::from_text_query("rss>=100MB");

        assert_eq!(filter.matching_indexes(&[small, large]), vec![1]);
    }

    #[test]
    fn parses_binary_memory_units() {
        assert_eq!(parse_bytes_value("7B"), Some(7));
        assert_eq!(parse_bytes_value("7KB"), Some(7 * 1024));
        assert_eq!(parse_bytes_value("7MB"), Some(7 * 1024 * 1024));
        assert_eq!(parse_bytes_value("7GB"), Some(7 * 1024 * 1024 * 1024));
    }

    #[test]
    fn metric_filter_supports_comparison_operators() {
        let mut low = sample_row(1, "low", "low");
        low.swap_bytes = 9 * 1024 * 1024;
        let mut high = sample_row(2, "high", "high");
        high.swap_bytes = 10 * 1024 * 1024;

        assert_eq!(
            ProcessFilter::from_text_query("swap<10MB")
                .matching_indexes(&[low.clone(), high.clone()]),
            vec![0]
        );
        assert_eq!(
            ProcessFilter::from_text_query("swap=10MB").matching_indexes(&[low, high]),
            vec![1]
        );
    }

    #[test]
    fn cpu_percent_suffix_is_optional() {
        let mut idle = sample_row(1, "idle", "idle");
        idle.cpu_percent = 4.9;
        let mut busy = sample_row(2, "busy", "busy");
        busy.cpu_percent = 5.0;
        let rows = vec![idle, busy];

        assert_eq!(
            ProcessFilter::from_text_query("cpu>=5").matching_indexes(&rows),
            vec![1]
        );
        assert_eq!(
            ProcessFilter::from_text_query("cpu>=5%").matching_indexes(&rows),
            vec![1]
        );
    }

    #[test]
    fn uss_and_pss_filters_ignore_unknown_values() {
        let mut unknown = sample_row(1, "unknown", "unknown");
        unknown.uss_bytes = None;
        unknown.pss_bytes = None;
        let mut known = sample_row(2, "known", "known");
        known.uss_bytes = Some(100 * 1024 * 1024);
        known.pss_bytes = Some(101 * 1024 * 1024);

        assert_eq!(
            ProcessFilter::from_text_query("uss>=100MB")
                .matching_indexes(&[unknown.clone(), known.clone()]),
            vec![1]
        );
        assert_eq!(
            ProcessFilter::from_text_query("pss>=101MB").matching_indexes(&[unknown, known]),
            vec![1]
        );
    }

    #[test]
    fn text_and_metric_tokens_are_combined_with_and() {
        let mut postgres = sample_row(1, "postgres", "postgres: checkpointer");
        postgres.rss_bytes = 100 * 1024 * 1024;
        let mut firefox = sample_row(2, "firefox", "firefox");
        firefox.rss_bytes = 200 * 1024 * 1024;

        let filter = ProcessFilter::from_text_query("postgres rss>=100MB");

        assert_eq!(filter.matching_indexes(&[postgres, firefox]), vec![0]);
    }

    #[test]
    fn reports_detailed_memory_needed_for_uss_and_pss_filters() {
        let filter = ProcessFilter::from_text_query("rss>=1MB uss>=2MB pss>=3MB");

        assert!(filter.detail_request().uss);
        assert!(filter.detail_request().pss);
    }
}
