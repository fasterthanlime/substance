//! Formatting utilities for displaying analysis results
//!
//! This module provides functions for formatting bytes, sizes, percentages,
//! and other values in human-readable formats.

use std::time::Duration;

/// Format bytes into human-readable units (B, KiB, MiB, GiB)
///
/// # Examples
/// ```
/// use substance::formatting::format_bytes;
/// 
/// assert_eq!(format_bytes(512), "512 B");
/// assert_eq!(format_bytes(1536), "1.50 KiB");
/// assert_eq!(format_bytes(1048576), "1.00 MiB");
/// ```
pub fn format_bytes(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = 1024 * KIB;
    const GIB: u64 = 1024 * MIB;

    if bytes >= GIB {
        format!("{:.2} GiB", bytes as f64 / GIB as f64)
    } else if bytes >= MIB {
        format!("{:.2} MiB", bytes as f64 / MIB as f64)
    } else if bytes >= KIB {
        format!("{:.2} KiB", bytes as f64 / KIB as f64)
    } else {
        format!("{bytes} B")
    }
}

/// Format a size difference in bytes with sign
///
/// # Examples
/// ```
/// use substance::formatting::format_size_diff;
/// 
/// assert_eq!(format_size_diff(1024), "+1.00 KiB");
/// assert_eq!(format_size_diff(-2048), "-2.00 KiB");
/// assert_eq!(format_size_diff(0), "no change");
/// ```
pub fn format_size_diff(diff: i64) -> String {
    if diff == 0 {
        "no change".to_string()
    } else {
        let abs_diff = diff.unsigned_abs();
        let formatted = format_bytes(abs_diff);
        if diff > 0 {
            format!("+{formatted}")
        } else {
            format!("-{formatted}")
        }
    }
}

/// Format a size difference with appropriate styling for terminal output
///
/// This function is only available with the "cli" feature enabled.
#[cfg(feature = "cli")]
pub fn format_size_diff_styled(diff: i64) -> String {
    use owo_colors::OwoColorize;
    
    let base = format_size_diff(diff);
    if diff > 0 {
        base.red().to_string()
    } else if diff < 0 {
        base.green().to_string()
    } else {
        base.bright_black().to_string()
    }
}

/// Format a percentage value
///
/// # Examples
/// ```
/// use substance::formatting::format_percentage;
/// 
/// assert_eq!(format_percentage(0.5), "0.5%");
/// assert_eq!(format_percentage(25.123), "25.1%");
/// assert_eq!(format_percentage(100.0), "100.0%");
/// ```
pub fn format_percentage(value: f64) -> String {
    format!("{value:.1}%")
}

/// Format a percentage change with sign
///
/// # Examples
/// ```
/// use substance::formatting::format_percentage_change;
/// 
/// assert_eq!(format_percentage_change(10.5), "+10.5%");
/// assert_eq!(format_percentage_change(-5.25), "-5.3%");
/// assert_eq!(format_percentage_change(0.0), "0.0%");
/// ```
pub fn format_percentage_change(value: f64) -> String {
    if value > 0.0 {
        format!("+{value:.1}%")
    } else {
        format!("{value:.1}%")
    }
}

/// Format a duration in seconds
///
/// # Examples
/// ```
/// use std::time::Duration;
/// use substance::formatting::format_duration;
/// 
/// assert_eq!(format_duration(&Duration::from_secs(45)), "45.00s");
/// assert_eq!(format_duration(&Duration::from_millis(1500)), "1.50s");
/// ```
pub fn format_duration(duration: &Duration) -> String {
    format!("{:.2}s", duration.as_secs_f64())
}

/// Format a duration change with sign
///
/// # Examples
/// ```
/// use substance::formatting::format_duration_diff;
/// 
/// assert_eq!(format_duration_diff(1.5), "+1.50s");
/// assert_eq!(format_duration_diff(-0.75), "-0.75s");
/// assert_eq!(format_duration_diff(0.0), "0.00s");
/// ```
pub fn format_duration_diff(diff: f64) -> String {
    if diff > 0.0 {
        format!("+{diff:.2}s")
    } else {
        format!("{diff:.2}s")
    }
}

/// Format a count with appropriate singular/plural form
///
/// # Examples
/// ```
/// use substance::formatting::format_count;
/// 
/// assert_eq!(format_count(0, "item"), "0 items");
/// assert_eq!(format_count(1, "item"), "1 item");
/// assert_eq!(format_count(5, "item"), "5 items");
/// ```
pub fn format_count(count: usize, singular: &str) -> String {
    if count == 1 {
        format!("{count} {singular}")
    } else {
        format!("{count} {singular}s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_bytes() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(1023), "1023 B");
        assert_eq!(format_bytes(1024), "1.00 KiB");
        assert_eq!(format_bytes(1536), "1.50 KiB");
        assert_eq!(format_bytes(1048576), "1.00 MiB");
        assert_eq!(format_bytes(1073741824), "1.00 GiB");
    }

    #[test]
    fn test_format_size_diff() {
        assert_eq!(format_size_diff(0), "no change");
        assert_eq!(format_size_diff(1024), "+1.00 KiB");
        assert_eq!(format_size_diff(-2048), "-2.00 KiB");
        assert_eq!(format_size_diff(1048576), "+1.00 MiB");
        assert_eq!(format_size_diff(-1048576), "-1.00 MiB");
    }

    #[test]
    fn test_format_percentage() {
        assert_eq!(format_percentage(0.0), "0.0%");
        assert_eq!(format_percentage(50.0), "50.0%");
        assert_eq!(format_percentage(100.0), "100.0%");
        assert_eq!(format_percentage(123.456), "123.5%");
    }

    #[test]
    fn test_format_percentage_change() {
        assert_eq!(format_percentage_change(0.0), "0.0%");
        assert_eq!(format_percentage_change(10.0), "+10.0%");
        assert_eq!(format_percentage_change(-5.5), "-5.5%");
    }

    #[test]
    fn test_format_duration() {
        assert_eq!(format_duration(&Duration::from_secs(0)), "0.00s");
        assert_eq!(format_duration(&Duration::from_secs(1)), "1.00s");
        assert_eq!(format_duration(&Duration::from_millis(1500)), "1.50s");
        assert_eq!(format_duration(&Duration::from_millis(500)), "0.50s");
    }

    #[test]
    fn test_format_count() {
        assert_eq!(format_count(0, "file"), "0 files");
        assert_eq!(format_count(1, "file"), "1 file");
        assert_eq!(format_count(2, "file"), "2 files");
        assert_eq!(format_count(100, "symbol"), "100 symbols");
    }
}