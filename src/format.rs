use std::borrow::Cow;

const BINARY_UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
const SI_UNITS: [&str; 5] = ["B", "kB", "MB", "GB", "TB"];
const KIB_BASE_VALUE: f64 = 1024.0;
const SI_BASE_VALUE: f64 = 1000.0;

/// Formats a byte count with binary units such as `KiB`, `MiB`, and `GiB`.
///
/// The output is intended for compact table and chart labels.
pub fn format_bytes(bytes: u64) -> String {
    format_bytes_with_units(bytes, &BINARY_UNITS, KIB_BASE_VALUE)
}

/// Formats a byte count with SI units such as `kB`, `MB`, and `GB`.
///
/// The output is intended for compact table and chart labels.
#[allow(dead_code)]
pub fn format_bytes_si(bytes: u64) -> String {
    format_bytes_with_units(bytes, &SI_UNITS, SI_BASE_VALUE)
}

/// Formats an optional byte count, returning `N/A` when no value is available.
///
/// This is used for `smaps_rollup` values that may not be accessible.
pub fn format_option_bytes(value: Option<u64>) -> Cow<'static, str> {
    format_option_bytes_with(value, format_bytes)
}

/// Formats an optional byte count with SI units, returning `N/A` when no
/// value is available.
#[allow(dead_code)]
pub fn format_option_bytes_si(value: Option<u64>) -> Cow<'static, str> {
    format_option_bytes_with(value, format_bytes_si)
}

fn format_bytes_with_units(bytes: u64, units: &[&str], base: f64) -> String {
    let mut value = bytes as f64;
    let mut unit_idx = 0usize;
    while value >= base && unit_idx < units.len() - 1 {
        value /= base;
        unit_idx += 1;
    }

    if unit_idx == 0 {
        format!("{} {}", bytes, units[unit_idx])
    } else {
        format!("{value:.1} {}", units[unit_idx])
    }
}

fn format_option_bytes_with(
    value: Option<u64>,
    formatter: impl FnOnce(u64) -> String,
) -> Cow<'static, str> {
    match value {
        Some(bytes) => Cow::Owned(formatter(bytes)),
        None => Cow::Borrowed("N/A"),
    }
}

/// Formats a percentage with one decimal place for table display.
///
/// The output is right-aligned to keep CPU columns visually stable.
pub fn format_percent(value: f32) -> String {
    format!("{value:>5.1}%")
}

/// Truncates a string by character count and returns an owned `String`.
///
/// This is used to cap wide fields such as command lines without breaking UTF-8
/// or truncating in the middle of a multi-byte character.
pub fn truncate_owned(input: &str, max_chars: usize) -> String {
    input.chars().take(max_chars).collect()
}

#[cfg(test)]
mod tests {
    use super::{format_bytes, format_bytes_si, format_option_bytes_si, truncate_owned};

    #[test]
    fn formats_binary_units() {
        assert_eq!(format_bytes(200), "200 B");
        assert_eq!(format_bytes(2048), "2.0 KiB");

        assert_eq!(format_bytes(5 * 1024 * 1024), "5.0 MiB");
        assert_eq!(format_bytes(5 * 1024 * 1024 * 1024), "5.0 GiB");
        assert_eq!(format_bytes(5 * 1024 * 1024 * 1024 * 1024), "5.0 TiB");
    }

    #[test]
    fn formats_si_units() {
        assert_eq!(format_bytes_si(200), "200 B");
        assert_eq!(format_bytes_si(2_000), "2.0 kB");
        assert_eq!(format_bytes_si(5_000_000), "5.0 MB");
        assert_eq!(format_bytes_si(5_000_000_000), "5.0 GB");
        assert_eq!(format_bytes_si(5_000_000_000_000), "5.0 TB");
    }

    #[test]
    fn formats_optional_si_units() {
        assert_eq!(format_option_bytes_si(Some(2_000)).as_ref(), "2.0 kB");
        assert_eq!(format_option_bytes_si(None).as_ref(), "N/A");
    }

    #[test]
    fn truncates_without_panicking() {
        assert_eq!(truncate_owned("abcdef", 4), "abcd");
        assert_eq!(truncate_owned("abc", 8), "abc");
    }
}
