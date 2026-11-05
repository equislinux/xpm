//! Shared terminal progress rendering helpers.

use std::io::{self, IsTerminal, Write};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Mutex;

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

/// Aggregate progress for a pool of concurrent package downloads.
///
/// Workers only update atomics; the calling thread renders one stable line.
/// This avoids workers fighting over the terminal while keeping progress
/// visible for every parallel download.
pub struct ParallelDownloadProgress {
    enabled: bool,
    total_jobs: usize,
    completed: AtomicUsize,
    downloaded: AtomicU64,
    total: AtomicU64,
    last_len: Mutex<usize>,
}

impl ParallelDownloadProgress {
    pub fn new(total_jobs: usize) -> Self {
        Self {
            enabled: io::stderr().is_terminal(),
            total_jobs,
            completed: AtomicUsize::new(0),
            downloaded: AtomicU64::new(0),
            total: AtomicU64::new(0),
            last_len: Mutex::new(0),
        }
    }

    pub fn add_total(&self, bytes: u64) {
        self.total.fetch_add(bytes, Ordering::Relaxed);
    }

    pub fn add_downloaded(&self, bytes: u64) {
        self.downloaded.fetch_add(bytes, Ordering::Relaxed);
    }

    pub fn complete_job(&self) {
        self.completed.fetch_add(1, Ordering::Relaxed);
    }

    pub fn is_done(&self) -> bool {
        self.completed.load(Ordering::Relaxed) >= self.total_jobs
    }

    pub fn render(&self) {
        if !self.enabled {
            return;
        }
        let downloaded = self.downloaded.load(Ordering::Relaxed);
        let total = self.total.load(Ordering::Relaxed);
        let completed = self.completed.load(Ordering::Relaxed);
        let style = ProgressStyle::for_download();
        let mut line = format_progress_with(&style, "Downloading packages", downloaded, total);
        line.push_str(&format!("  ({completed}/{})", self.total_jobs));

        let mut last_len = match self.last_len.lock() {
            Ok(guard) => guard,
            Err(poison) => poison.into_inner(),
        };
        let width = line.chars().count();
        let mut stderr = io::stderr();
        let _ = write!(stderr, "\r{line}");
        if *last_len > width {
            let _ = write!(stderr, "{:width$}", "", width = *last_len - width);
        }
        let _ = stderr.flush();
        *last_len = width;
    }

    pub fn finish(&self) {
        if !self.enabled {
            return;
        }
        let mut stderr = io::stderr();
        let _ = writeln!(stderr);
        let _ = stderr.flush();
    }
}

/// Render an ASCII progress bar such as `xxxxxxxxxx----------`.
pub(crate) fn bar(percent: u64, width: usize) -> String {
    let filled = (percent.min(100) as usize).saturating_mul(width) / 100;
    format!("{}{}", "x".repeat(filled), "-".repeat(width - filled))
}

/// Render a byte count in MiB with one decimal place.
pub(crate) fn format_mib(bytes: u64) -> String {
    format!("{} MiB", mib_value(bytes))
}

/// Fixed-width layout for progress lines.
///
/// The package/file label is padded (or truncated) to `label_width` so every
/// bar in a transaction starts at the same column, like pacman's download
/// display.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProgressStyle {
    pub label_width: usize,
    pub bar_width: usize,
}

impl ProgressStyle {
    /// Style for independent downloads; a wide fixed label column keeps
    /// package filenames readable while aligning every bar.
    pub fn for_download() -> Self {
        let terminal = terminal_width().unwrap_or(80);
        let label_width = (terminal / 2).clamp(24, 40);
        let bar_width = terminal.saturating_sub(label_width + 34).clamp(10, 40);
        Self {
            label_width,
            bar_width,
        }
    }

    pub fn render(&self, label: &str, downloaded: u64, total: u64) -> String {
        format_progress_with(self, label, downloaded, total)
    }
}

/// Render one progress line with an aligned label column.
pub fn format_progress_with(
    style: &ProgressStyle,
    label: &str,
    downloaded: u64,
    total: u64,
) -> String {
    let percent = percentage(downloaded, total);
    let label = align_label(label, style.label_width);
    format!(
        "{label} [{}] {percent:>3}%  {}/{} MiB",
        bar(percent, style.bar_width),
        mib_value(downloaded),
        mib_value(total),
    )
}

fn align_label(label: &str, width: usize) -> String {
    let chars: Vec<char> = label.chars().collect();
    if chars.len() > width {
        if width <= 2 {
            return ".".repeat(width);
        }
        let mut truncated: String = chars.into_iter().take(width - 2).collect();
        truncated.push_str("..");
        truncated
    } else {
        format!("{label:<width$}")
    }
}

fn terminal_width() -> Option<usize> {
    if let Ok(value) = std::env::var("COLUMNS") {
        if let Ok(width) = value.parse::<usize>() {
            if width > 0 {
                return Some(width);
            }
        }
    }
    #[cfg(unix)]
    {
        let mut size: libc::winsize = unsafe { std::mem::zeroed() };
        let result = unsafe { libc::ioctl(libc::STDERR_FILENO, libc::TIOCGWINSZ, &mut size) };
        if result == 0 && size.ws_col > 0 {
            return Some(size.ws_col as usize);
        }
    }
    None
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
        assert_eq!(bar(52, 20), "xxxxxxxxxx----------");
        assert_eq!(bar(100, 20), "xxxxxxxxxxxxxxxxxxxx");
        assert_eq!(bar(150, 20), "xxxxxxxxxxxxxxxxxxxx");
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
        let line = ProgressStyle {
            label_width: 11,
            bar_width: 20,
        }
        .render("pkg.tar.zst", 13 * 1024 * 1024, 26 * 1024 * 1024);
        assert_eq!(
            line,
            "pkg.tar.zst [xxxxxxxxxx----------]  50%  13.0/26.0 MiB"
        );
    }

    #[test]
    fn parallel_progress_aggregates_bytes_and_completed_jobs() {
        let progress = ParallelDownloadProgress::new(2);
        progress.add_total(300);
        progress.add_downloaded(100);
        progress.complete_job();
        assert!(!progress.is_done());
        progress.add_downloaded(200);
        progress.complete_job();
        assert!(progress.is_done());
    }

    #[test]
    fn progress_line_handles_zero_total() {
        let line = ProgressStyle {
            label_width: 11,
            bar_width: 20,
        }
        .render("empty", 0, 0);
        assert_eq!(line, "empty       [--------------------]   0%  0.0/0.0 MiB");
    }

    #[test]
    fn aligned_labels_put_bars_in_the_same_column() {
        let style = ProgressStyle {
            label_width: 16,
            bar_width: 12,
        };
        let short = style.render("vim", 1, 2);
        let long = style.render("linux-firmware", 1, 2);
        assert_eq!(short.find('['), long.find('['));
        assert_eq!(short.find('['), Some(17));
    }

    #[test]
    fn long_labels_are_truncated_to_the_column() {
        let style = ProgressStyle {
            label_width: 10,
            bar_width: 10,
        };
        let line = style.render("a-very-long-package-name", 1, 2);
        assert!(line.starts_with("a-very-l.. ["));
        assert_eq!(line.find('['), Some(11));
    }

    #[test]
    fn style_respects_terminal_width_floor() {
        let style = ProgressStyle::for_download();
        assert!(style.label_width >= 12);
        assert!(style.bar_width >= 10);
    }
}
