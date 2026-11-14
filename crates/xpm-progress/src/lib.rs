//! Shared terminal progress rendering helpers.

use std::io::{self, IsTerminal, Write};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

const MIB: f64 = 1024.0 * 1024.0;

/// Percentage of `downloaded` relative to `total`, clamped to 0..=100.
///
/// A zero (or unknown) total reports zero instead of dividing by zero.
pub fn percentage(downloaded: u64, total: u64) -> u64 {
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

// ── Progress settings and per-phase counters ─────────────────────────────────

/// Process-wide progress settings.
struct ProgressSettings {
    enabled: bool,
    tty: bool,
    verbose: bool,
}

static SETTINGS: OnceLock<Mutex<ProgressSettings>> = OnceLock::new();

fn settings() -> &'static Mutex<ProgressSettings> {
    SETTINGS.get_or_init(|| {
        Mutex::new(ProgressSettings {
            enabled: false,
            tty: false,
            verbose: false,
        })
    })
}

fn with_settings<T>(f: impl FnOnce(&mut ProgressSettings) -> T) -> T {
    let mut guard = match settings().lock() {
        Ok(guard) => guard,
        Err(poison) => poison.into_inner(),
    };
    f(&mut guard)
}

/// Enable progress rendering for this process.
pub fn init(enabled: bool, _no_color: bool, verbose: bool) {
    let enabled = enabled && std::env::var_os("XPM_NO_PROGRESS").is_none();
    with_settings(|state| {
        state.enabled = enabled;
        state.tty = io::stderr().is_terminal();
        state.verbose = verbose;
    });
}

/// Whether detailed per-package lines (downloads, extraction) should print.
pub fn verbose() -> bool {
    with_settings(|state| state.verbose)
}

/// Print a persistent message.
pub fn message(msg: impl AsRef<str>) {
    eprintln!("{}", msg.as_ref());
}

/// Print a warning.
pub fn warn(msg: impl AsRef<str>) {
    eprintln!("warning: {}", msg.as_ref());
}

/// Finish any active progress state. Safe to call multiple times.
pub fn finish_all() {
    with_settings(|state| {
        if state.enabled && state.tty {
            let _ = writeln!(io::stderr());
        }
        state.enabled = false;
    });
}

/// A standalone count-based bar for one transaction phase.
///
/// Each instance owns its line; there is deliberately no combined
/// download/verify/extract renderer. Network phases use [`BarFill::Cross`] and
/// verification/install/remove phases use [`BarFill::Slash`].
pub struct OperationProgress {
    label: String,
    fill: BarFill,
    total: usize,
    done: AtomicUsize,
    enabled: bool,
    tty: bool,
    last_len: Mutex<usize>,
}

impl OperationProgress {
    pub fn new(label: impl Into<String>, total: usize, fill: BarFill) -> Self {
        let (enabled, tty) = with_settings(|state| (state.enabled, state.tty));
        let progress = Self {
            label: label.into(),
            fill,
            total,
            done: AtomicUsize::new(0),
            enabled,
            tty,
            last_len: Mutex::new(0),
        };
        progress.draw(false);
        progress
    }

    pub fn advance(&self) {
        let done = self.done.fetch_add(1, Ordering::Relaxed) + 1;
        if done <= self.total {
            self.draw(false);
        }
    }

    pub fn finish(&self) {
        if self.total > 0 {
            self.done.store(self.total, Ordering::Relaxed);
            self.draw(true);
        }
    }

    fn line(&self) -> Option<String> {
        let total = self.total;
        if total == 0 {
            return None;
        }
        let done = self.done.load(Ordering::Relaxed).min(total);
        let percent = percentage(done as u64, total as u64);
        let width = 20usize;
        Some(format!(
            "{} [{}] {:>3}%  {}/{}",
            self.label,
            bar(percent, width, self.fill),
            percent,
            done,
            total
        ))
    }

    fn draw(&self, force: bool) {
        if !self.enabled {
            return;
        }
        let Some(line) = self.line() else {
            return;
        };
        let mut stderr = io::stderr();
        if self.tty {
            let _ = write!(stderr, "\r\x1b[2K{line}");
        } else if force {
            let _ = writeln!(stderr, "{line}");
        }
        let _ = stderr.flush();
        if self.tty {
            let mut last = match self.last_len.lock() {
                Ok(guard) => guard,
                Err(poison) => poison.into_inner(),
            };
            *last = line.chars().count();
        }
    }
}

// ── Phase progress ───────────────────────────────────────────────────────────

/// One bar per phase step, rendered with the same aligned label/bar layout as
/// the package progress lines.
///
/// This is the shared implementation used by `xpm-aur` for its own phases
/// (`resolve`, `checkout`, `state`, ...), so the helper never grows a second
/// progress style.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PhaseProgress {
    label_width: usize,
    bar_width: usize,
    fill: BarFill,
}

impl PhaseProgress {
    /// Build a layout for the labels in one transaction.
    pub fn new<'a>(labels: impl IntoIterator<Item = &'a str>) -> Self {
        let longest = labels
            .into_iter()
            .map(|label| label.chars().count())
            .max()
            .unwrap_or(12);
        Self {
            label_width: longest.clamp(12, 32),
            bar_width: 20,
            fill: BarFill::Slash,
        }
    }

    /// Network setup phases (`sync`, `resolve`): cross fill.
    pub fn new_network<'a>(labels: impl IntoIterator<Item = &'a str>) -> Self {
        let mut progress = Self::new(labels);
        progress.fill = BarFill::Cross;
        progress
    }

    /// Build/install phases (`checkout`, `building`, `installing`): slash fill.
    pub fn new_build<'a>(labels: impl IntoIterator<Item = &'a str>) -> Self {
        Self::new(labels)
    }

    /// Render `label [////----]  50%  phase`.
    pub fn render(&self, label: &str, step: u64, total: u64, phase: &str) -> String {
        let percent = percentage(step, total);
        let label = align_label(label, self.label_width);
        format!(
            "{label} [{}] {percent:>3}%  {phase}",
            bar(percent, self.bar_width, self.fill)
        )
    }

    pub fn print(&self, label: &str, step: u64, total: u64, phase: &str) {
        eprintln!("{}", self.render(label, step, total, phase));
    }
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
        assert_eq!(bar(0, 20, BarFill::Slash), "--------------------");
        assert_eq!(bar(52, 20, BarFill::Slash), "//////////----------");
        assert_eq!(bar(100, 20, BarFill::Slash), "////////////////////");
        assert_eq!(bar(150, 20, BarFill::Slash), "////////////////////");
        assert_eq!(bar(0, 0, BarFill::Slash), "");
    }

    #[test]
    fn mib_rendering_uses_one_decimal() {
        assert_eq!(format_mib(0), "0.0 MiB");
        assert_eq!(format_mib(1024 * 1024), "1.0 MiB");
        assert_eq!(format_mib(24_641_536), "23.5 MiB");
    }

    #[test]
    fn download_bars_use_cross_fill() {
        assert_eq!(bar(0, 20, BarFill::Cross), "--------------------");
        assert_eq!(bar(52, 20, BarFill::Cross), "xxxxxxxxxx----------");
        assert_eq!(bar(100, 20, BarFill::Cross), "xxxxxxxxxxxxxxxxxxxx");
        assert!(!bar(50, 20, BarFill::Cross).contains('#'));
        let line = ProgressStyle {
            label_width: 11,
            bar_width: 20,
            fill: BarFill::Cross,
        }
        .render("pkg.tar.zst", 13 * 1024 * 1024, 26 * 1024 * 1024);
        assert_eq!(
            line,
            "pkg.tar.zst [xxxxxxxxxx----------]  50%  13.0/26.0 MiB"
        );
    }

    #[test]
    fn operation_bars_use_slash_fill() {
        assert_eq!(bar(52, 20, BarFill::Slash), "//////////----------");
        assert!(!bar(50, 20, BarFill::Slash).contains('x'));
        let progress = OperationProgress::new("Verifying signatures", 2, BarFill::Slash);
        let line = progress.line().expect("operation line");
        assert!(line.contains("[--------------------]"));
        progress.advance();
        progress.advance();
        let line = progress.line().expect("operation line");
        assert!(line.contains("[////////////////////]"));
    }

    #[test]
    fn phase_progress_selects_fill_by_phase_type() {
        let network = PhaseProgress::new_network(["sync"]);
        assert!(network.render("sync", 1, 2, "databases").contains('x'));
        let build = PhaseProgress::new_build(["pkg"]);
        assert!(build.render("pkg", 1, 2, "building").contains('/'));
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
            fill: BarFill::Cross,
        }
        .render("empty", 0, 0);
        assert_eq!(line, "empty       [--------------------]   0%  0.0/0.0 MiB");
    }

    #[test]
    fn aligned_labels_put_bars_in_the_same_column() {
        let style = ProgressStyle {
            label_width: 16,
            bar_width: 12,
            fill: BarFill::Cross,
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
            fill: BarFill::Cross,
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

    #[test]
    fn phase_bars_share_one_column() {
        let progress = PhaseProgress::new(["foo", "a-very-long-package"]);
        let short = progress.render("foo", 1, 3, "building");
        let long = progress.render("a-very-long-package", 1, 3, "building");
        assert_eq!(short.find('['), long.find('['));
    }

    #[test]
    fn phase_long_labels_are_truncated() {
        let progress = PhaseProgress::new(["x"]);
        let line = progress.render("a-very-long-package-name", 1, 3, "building");
        assert!(line.starts_with("a-very-lon.."));
        assert_eq!(line.find('['), Some(13));
    }

    #[test]
    fn phase_zero_total_is_zero_percent() {
        let progress = PhaseProgress::new(["x"]);
        let line = progress.render("pkg", 3, 0, "idle");
        assert!(line.contains("  0%"));
    }
}
