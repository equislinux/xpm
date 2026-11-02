//! Shared terminal progress rendering helpers.

/// Width of the ASCII progress bar, in characters.
pub(crate) const BAR_WIDTH: usize = 20;

const MIB: f64 = 1024.0 * 1024.0;

/// Percentage of `downloaded` relative to `total`, clamped to 0..=100.
///
/// A zero (or unknown) total reports zero instead of dividing by zero.
pub(crate) fn percentage(downloaded: u64, total: u64) -> u64 {
    if total == 0 {
        return 0;
    }

    downloaded.saturating_mul(100).div_euclid(total).min(100)
}

/// Render an ASCII progress bar such as `##########----------`.
pub(crate) fn bar(percent: u64, width: usize) -> String {
    let filled = (percent.min(100) as usize).saturating_mul(width) / 100;
    format!("{}{}", "#".repeat(filled), "-".repeat(width - filled))
}

/// Render a byte count in MiB with one decimal place.
pub(crate) fn format_mib(bytes: u64) -> String {
    format!("{} MiB", mib_value(bytes))
}

/// Render one progress line, e.g.
/// `pkg.tar.zst [##########----------]  52%  12.3/23.5 MiB`.
pub(crate) fn format_progress(label: &str, downloaded: u64, total: u64) -> String {
    let percent = percentage(downloaded, total);
    format!(
        "{label} [{}] {percent:>3}%  {}/{} MiB",
        bar(percent, BAR_WIDTH),
        mib_value(downloaded),
        mib_value(total),
    )
}

fn mib_value(bytes: u64) -> String {
    format!("{:.1}", bytes as f64 / MIB)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentage_reports_ratio_and_clamps() {
        assert_eq!(percentage(0, 100), 0);
        assert_eq!(percentage(52, 100), 52);
        assert_eq!(percentage(100, 100), 100);
        assert_eq!(percentage(150, 100), 100);
    }

    #[test]
    fn percentage_handles_zero_total() {
        assert_eq!(percentage(0, 0), 0);
        assert_eq!(percentage(42, 0), 0);
    }

    #[test]
    fn bar_fills_proportionally_and_clamps() {
        assert_eq!(bar(0, 20), "--------------------");
        assert_eq!(bar(52, 20), "##########----------");
        assert_eq!(bar(100, 20), "####################");
        assert_eq!(bar(150, 20), "####################");
        assert_eq!(bar(0, 0), "");
    }

    #[test]
    fn mib_rendering_uses_one_decimal() {
        assert_eq!(format_mib(0), "0.0 MiB");
        assert_eq!(format_mib(1024 * 1024), "1.0 MiB");
        assert_eq!(format_mib(24_641_536), "23.5 MiB");
    }

    #[test]
    fn progress_line_matches_pacman_style() {
        let line = format_progress("pkg.tar.zst", 13 * 1024 * 1024, 26 * 1024 * 1024);
        assert_eq!(
            line,
            "pkg.tar.zst [##########----------]  50%  13.0/26.0 MiB"
        );
    }

    #[test]
    fn progress_line_handles_zero_total() {
        let line = format_progress("empty", 0, 0);
        assert_eq!(line, "empty [--------------------]   0%  0.0/0.0 MiB");
    }
}
