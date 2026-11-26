//! Aggregate progress for concurrent downloads.

use super::*;

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
