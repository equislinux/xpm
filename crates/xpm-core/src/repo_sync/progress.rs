//! Single-line download progress rendering.

use super::*;

/// Overwrites one stderr line while a download is in progress.
pub(super) struct DownloadProgress {
    enabled: bool,
    label: String,
    style: crate::progress::ProgressStyle,
    widest: usize,
    active: bool,
}

impl DownloadProgress {
    pub(super) fn new(dest: &Path) -> Self {
        let label = dest
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "download".to_string());

        Self {
            enabled: io::stderr().is_terminal(),
            label,
            style: crate::progress::ProgressStyle::for_download(),
            widest: 0,
            active: false,
        }
    }

    pub(super) fn update(&mut self, downloaded: u64, total: u64) {
        if !self.enabled {
            return;
        }

        let line = self.style.render(&self.label, downloaded, total);
        self.widest = self.widest.max(line.chars().count());
        let mut stderr = io::stderr();
        let _ = write!(stderr, "\r{line}");
        let _ = stderr.flush();
        self.active = true;
    }

    pub(super) fn finish(&mut self, downloaded: u64, total: u64) {
        if !self.enabled || !self.active {
            return;
        }

        let line = self.style.render(&self.label, downloaded, total);
        let padding = self.widest.saturating_sub(line.chars().count());
        let mut stderr = io::stderr();
        let _ = write!(stderr, "\r{line}{:padding$}\n", "", padding = padding);
        let _ = stderr.flush();
        self.active = false;
    }
}

impl Drop for DownloadProgress {
    fn drop(&mut self) {
        if !self.active {
            return;
        }

        let mut stderr = io::stderr();
        let _ = write!(stderr, "\r{:width$}\r", "", width = self.widest);
        let _ = stderr.flush();
    }
}
