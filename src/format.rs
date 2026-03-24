use std::borrow::Cow;

/// Formats a byte count with binary units such as `KiB`, `MiB`, and `GiB`.
///
/// The output is intended for compact table and chart labels.
pub fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    const KIB_BASE_VALUE: f64 = 1024.0;

    let mut value = bytes as f64;
    let mut unit_idx = 0usize;
    while value >= KIB_BASE_VALUE && unit_idx < UNITS.len() - 1 {
        value /= KIB_BASE_VALUE;
        unit_idx += 1;
    }

    if unit_idx == 0 {
        format!("{} {}", bytes, UNITS[unit_idx])
    } else {
        format!("{value:.1} {}", UNITS[unit_idx])
    }
}

/// Formats an optional byte count, returning `N/A` when no value is available.
///
/// This is used for `smaps_rollup` values that may not be accessible.
pub fn format_option_bytes(value: Option<u64>) -> Cow<'static, str> {
    match value {
        Some(bytes) => Cow::Owned(format_bytes(bytes)),
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
    use super::{format_bytes, truncate_owned};

    #[test]
    fn formats_binary_units() {
        assert_eq!(format_bytes(200), "200 B");
        assert_eq!(format_bytes(2048), "2.0 KiB");

        assert_eq!(format_bytes(5 * 1024 * 1024), "5.0 MiB");
        assert_eq!(format_bytes(5 * 1024 * 1024 * 1024), "5.0 GiB");
        assert_eq!(format_bytes(5 * 1024 * 1024 * 1024 * 1024), "5.0 TiB");
    }

    #[test]
    fn truncates_without_panicking() {
        assert_eq!(truncate_owned("abcdef", 4), "abcd");
        assert_eq!(truncate_owned("abc", 8), "abc");
    }
}
