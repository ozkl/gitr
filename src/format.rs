//! Human-readable formatting shared by all views.

/// File size with binary units: "820 bytes", "52.4 KB", "3.1 MB", "1.25 GB".
pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    if bytes < 1024 {
        return format!("{bytes} {}", if bytes == 1 { "byte" } else { "bytes" });
    }
    let mut value = bytes as f64 / 1024.0;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    // Two decimals for GB and above, where small differences still matter.
    let decimals = if unit >= 2 || value < 10.0 { 2 } else { 1 };
    let text = format!("{value:.decimals$}");
    // "3.10 MB" -> "3.1 MB", "2.00 KB" -> "2 KB"
    let text = if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_owned()
    } else {
        text
    };
    format!("{text} {}", UNITS[unit])
}

/// "just now", "5 mins ago", "2 hours ago", "3 days ago" for an age in seconds,
/// plus how many seconds until that text changes.
pub fn time_ago(secs: i64) -> (String, u64) {
    let secs = secs.max(0);
    let plural = |n: i64, unit: &str| format!("{n} {unit}{} ago", if n == 1 { "" } else { "s" });
    let (text, step) = match secs {
        0..60 => ("just now".to_owned(), 60),
        60..3600 => (plural(secs / 60, "min"), 60),
        3600..86400 => (plural(secs / 3600, "hour"), 3600),
        _ => (plural(secs / 86400, "day"), 86400),
    };
    (text, (step - secs % step) as u64)
}

#[cfg(test)]
mod tests {
    use super::{human_size, time_ago};

    #[test]
    fn ages() {
        assert_eq!(time_ago(5), ("just now".to_owned(), 55));
        assert_eq!(time_ago(61).0, "1 min ago");
        assert_eq!(time_ago(150), ("2 mins ago".to_owned(), 30));
        assert_eq!(time_ago(7200).0, "2 hours ago");
        assert_eq!(time_ago(90000).0, "1 day ago");
    }

    #[test]
    fn formats_sizes() {
        assert_eq!(human_size(0), "0 bytes");
        assert_eq!(human_size(1), "1 byte");
        assert_eq!(human_size(820), "820 bytes");
        assert_eq!(human_size(2048), "2 KB");
        assert_eq!(human_size(53_642), "52.4 KB");
        assert_eq!(human_size(3_250_585), "3.1 MB");
        assert_eq!(human_size(1_342_177_280), "1.25 GB");
        assert_eq!(human_size(5 * 1024u64.pow(4)), "5 TB");
    }
}
