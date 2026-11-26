//! Standalone per-operation count bar.

use super::*;

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

    pub(super) fn line(&self) -> Option<String> {
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
