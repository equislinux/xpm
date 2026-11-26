//! Single-transfer implementation: HTTP/file copy, atomic writes and temp names.

use super::*;

pub(super) fn download_once(
    url: &str,
    dest: &Path,
    show_progress: bool,
    tracker: Option<&ParallelDownloadProgress>,
    expected_sha256: Option<&str>,
    timeout: Option<Duration>,
) -> XpmResult<()> {
    if let Some(path) = local_file_path(url) {
        tracing::debug!(url, source = %path.display(), "syncing database from local file mirror");
        copy_local_file(&path, dest, tracker)?;
        if let Some(expected) = expected_sha256 {
            if let Err(err) = verify_sha256(dest, expected) {
                let _ = fs::remove_file(dest);
                return Err(err);
            }
        }
        return Ok(());
    }

    let mut request = http_client().get(url);
    if let Some(timeout) = timeout {
        request = request.timeout(timeout);
    }
    let mut response = request
        .send()
        .and_then(|r| r.error_for_status())
        .map_err(|e| XpmError::Database(format!("{url}: {e}")))?;

    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }

    let total = response.content_length();
    if let (Some(total), Some(tracker)) = (total, tracker) {
        tracker.add_total(total);
    }
    let temp_path = unique_temp_path(dest);
    let result = (|| -> XpmResult<()> {
        let mut file = fs::File::create(&temp_path)?;
        let mut downloaded: u64 = 0;
        let mut buffer = [0u8; 65536];
        let mut next_report: u64 = 25;
        let mut progress = show_progress.then(|| DownloadProgress::new(dest));
        let mut last_percent: Option<u64> = None;

        loop {
            let n = response
                .read(&mut buffer)
                .map_err(|e| XpmError::Database(format!("{url}: {e}")))?;
            if n == 0 {
                break;
            }

            file.write_all(&buffer[..n])?;
            downloaded += n as u64;
            if let Some(tracker) = tracker {
                tracker.add_downloaded(n as u64);
            }

            if let Some(total) = total {
                let percent = percentage(downloaded, total);
                if last_percent != Some(percent) {
                    if let Some(progress) = progress.as_mut() {
                        progress.update(downloaded, total);
                    }
                    last_percent = Some(percent);
                }
                if percent >= next_report {
                    tracing::debug!(url, downloaded, total, percent, "download progress");
                    next_report = (next_report + 25).min(100);
                }
            }
        }

        file.flush()?;
        // Verify before publishing the file: a concurrent downloader or a
        // broken mirror must never replace a valid cache entry with bad bytes.
        if let Some(expected) = expected_sha256 {
            verify_sha256(&temp_path, expected)?;
        }
        fs::rename(&temp_path, dest)?;
        if let (Some(total), Some(progress)) = (total, progress.as_mut()) {
            progress.finish(downloaded, total);
        }
        tracing::debug!(url, downloaded, dest = %dest.display(), "download complete");
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp_path);
    }
    result?;
    Ok(())
}

fn local_file_path(url: &str) -> Option<PathBuf> {
    let raw = url.strip_prefix("file://")?;
    if raw.is_empty() {
        return None;
    }
    Some(PathBuf::from(raw))
}

fn copy_local_file(
    source: &Path,
    dest: &Path,
    progress: Option<&ParallelDownloadProgress>,
) -> XpmResult<()> {
    let data = fs::read(source)?;
    if let Some(progress) = progress {
        progress.add_total(data.len() as u64);
        progress.add_downloaded(data.len() as u64);
    }
    write_atomically(dest, &data)?;

    if io::stderr().is_terminal() {
        let label = dest
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "download".to_string());
        eprintln!("{label} (local copy, {})", format_mib(data.len() as u64));
    }

    Ok(())
}

fn write_atomically(dest: &Path, bytes: &[u8]) -> XpmResult<()> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }

    let temp_path = unique_temp_path(dest);
    let result = (|| -> XpmResult<()> {
        let mut file = fs::File::create(&temp_path)?;
        file.write_all(bytes)?;
        file.flush()?;
        fs::rename(&temp_path, dest)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp_path);
    }
    result
}

/// Unique temp name so concurrent xpm/chroot processes can download the same
/// package without sharing a `.part` file.
fn unique_temp_path(dest: &Path) -> PathBuf {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    dest.with_extension(format!("part.{}.{nanos}", std::process::id()))
}
