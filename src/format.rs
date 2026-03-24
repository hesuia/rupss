pub fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0usize;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }

    if unit == 0 {
        format!("{} {}", bytes, UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

pub fn format_option_bytes(value: Option<u64>) -> String {
    value.map(format_bytes).unwrap_or_else(|| "N/A".to_string())
}

pub fn format_percent(value: f32) -> String {
    format!("{value:>5.1}%")
}

pub fn truncate_owned(input: &str, max_chars: usize) -> String {
    let mut result = String::new();
    for (idx, ch) in input.chars().enumerate() {
        if idx >= max_chars {
            break;
        }
        result.push(ch);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::{format_bytes, truncate_owned};

    #[test]
    fn formats_binary_units() {
        assert_eq!(format_bytes(123), "123 B");
        assert_eq!(format_bytes(2048), "2.0 KiB");
        assert_eq!(format_bytes(3 * 1024 * 1024), "3.0 MiB");
    }

    #[test]
    fn truncates_without_panicking() {
        assert_eq!(truncate_owned("abcdef", 4), "abcd");
        assert_eq!(truncate_owned("abc", 8), "abc");
    }
}
