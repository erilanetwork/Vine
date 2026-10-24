use std::time::Duration;

/// Parses durations like `30m`, `12h` or `1d12h`. Units are s, m, h, d and w; a bare number
/// isn't accepted, so `/ban Steve 5` can't be mistaken for five seconds.
pub fn parse_duration(text: &str) -> Option<Duration> {
    let mut total: u64 = 0;
    let mut number = String::new();
    for c in text.trim().chars() {
        if c.is_ascii_digit() {
            number.push(c);
            continue;
        }
        let unit = match c.to_ascii_lowercase() {
            's' => 1,
            'm' => 60,
            'h' => 3_600,
            'd' => 86_400,
            'w' => 604_800,
            _ => return None,
        };
        let value: u64 = number.parse().ok()?;
        number.clear();
        total = total.checked_add(value.checked_mul(unit)?)?;
    }
    (number.is_empty() && total > 0).then(|| Duration::from_secs(total))
}

/// Formats a duration as `1d 2h 3m`, skipping zero parts. Under a minute shows seconds.
pub fn format_duration(duration: Duration) -> String {
    let secs = duration.as_secs();
    if secs < 60 {
        return format!("{secs}s");
    }
    let parts = [
        (secs / 86_400, "d"),
        (secs / 3_600 % 24, "h"),
        (secs / 60 % 60, "m"),
    ];
    parts
        .iter()
        .filter(|(value, _)| *value > 0)
        .map(|(value, unit)| format!("{value}{unit}"))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_duration() {
        assert_eq!(parse_duration("30m"), Some(Duration::from_secs(1_800)));
        assert_eq!(parse_duration("1d12h"), Some(Duration::from_secs(129_600)));
        assert_eq!(parse_duration("2W"), Some(Duration::from_secs(1_209_600)));
        assert_eq!(parse_duration("5"), None);
        assert_eq!(parse_duration("griefing"), None);
        assert_eq!(parse_duration("0m"), None);
        assert_eq!(parse_duration(""), None);
    }

    #[test]
    fn test_format_duration() {
        assert_eq!(format_duration(Duration::from_secs(42)), "42s");
        assert_eq!(format_duration(Duration::from_secs(129_660)), "1d 12h 1m");
    }
}
