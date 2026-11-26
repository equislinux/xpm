//! Progress timing, bar layout and line rendering.

use super::*;

/// Percentage of `downloaded` relative to `total`, clamped to 0..=100.
///
/// A zero (or unknown) total reports zero instead of dividing by zero.
pub fn percentage(downloaded: u64, total: u64) -> u64 {
    if total == 0 {
        return 0;
    }

    downloaded.saturating_mul(100).div_euclid(total).min(100)
}

/// Fill glyph used by a progress phase.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BarFill {
    /// Network/download phases: `xxxxxxxxxx----------`.
    Cross,
    /// Verification, build, install and removal phases: `//////////----------`.
    Slash,
}

impl BarFill {
    fn glyph(self) -> char {
        match self {
            BarFill::Cross => 'x',
            BarFill::Slash => '/',
        }
    }
}

/// Render an ASCII progress bar with the requested fill glyph.
pub(crate) fn bar(percent: u64, width: usize, fill: BarFill) -> String {
    let filled = (percent.min(100) as usize).saturating_mul(width) / 100;
    format!(
        "{}{}",
        fill.glyph().to_string().repeat(filled),
        "-".repeat(width - filled)
    )
}

/// Render a byte count in MiB with one decimal place.
pub fn format_mib(bytes: u64) -> String {
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
    pub fill: BarFill,
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
            fill: BarFill::Cross,
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
        bar(percent, style.bar_width, style.fill),
        mib_value(downloaded),
        mib_value(total),
    )
}

pub(super) fn align_label(label: &str, width: usize) -> String {
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
