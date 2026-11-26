//! Package URL candidates and mirror download retries.

use super::*;

/// Build candidate package download URLs for a repository entry.
///
/// Resolution order:
/// 1) `<expanded_server>/<filename>` for each configured repo server
/// 2) If `%URL%` is a direct `.xp` URL, use it as-is
/// 3) If `%URL%` points to a GitHub repo, derive release URL:
///    `<url>/releases/download/<name>-<version>/<filename>`
pub fn package_download_candidates(
    repo: &Repository,
    arch: &str,
    entry: &RepoEntry,
) -> Vec<String> {
    let mut candidates = Vec::new();

    let Some(filename) = entry.filename.as_deref() else {
        return candidates;
    };

    for server in &repo.server {
        let base = expand_repo_url(server, &repo.name, arch);
        let base = base.trim_end_matches('/');
        candidates.push(format!("{base}/{filename}"));
    }

    if let Some(url) = entry.url.as_deref() {
        let clean = url.trim();
        if clean.ends_with(".xp") {
            candidates.push(clean.to_string());
        } else if clean.starts_with("https://github.com/") && filename.ends_with(".xp") {
            // `%URL%` is the package homepage, not a download source. Only
            // derive a GitHub release URL for native .xp artifacts; standard
            // Arch .pkg.tar.zst packages are always served by the repository
            // mirror and must not be rotated onto a bogus GitHub URL.
            let repo_url = clean.trim_end_matches('/');
            let tag = format!("{}-{}", entry.name, entry.version);
            candidates.push(format!("{repo_url}/releases/download/{tag}/{filename}"));
        }
    }

    candidates
}

/// Download from the first working URL to `dest`.
pub fn download_first_available(urls: &[String], dest: &Path, retries: u32) -> XpmResult<String> {
    download_first_available_with_progress(urls, dest, retries, true, None, None)
}

/// Download and fail over until a mirror serves bytes matching the expected
/// SHA-256. This catches mirrors that return an HTML/error page with HTTP 200.
pub fn download_first_available_checked(
    urls: &[String],
    dest: &Path,
    retries: u32,
    expected_sha256: Option<&str>,
) -> XpmResult<String> {
    download_first_available_with_progress(urls, dest, retries, true, None, expected_sha256)
}

/// Like [`download_first_available`] but without terminal progress updates.
/// Used by the parallel download scheduler, whose workers must not fight over
/// one carriage-return line.
pub fn download_first_available_quiet(
    urls: &[String],
    dest: &Path,
    retries: u32,
) -> XpmResult<String> {
    download_first_available_with_progress(urls, dest, retries, false, None, None)
}

/// Like [`download_first_available_quiet`] but reports aggregate progress to
/// the shared parallel download tracker.
pub fn download_first_available_tracked(
    urls: &[String],
    dest: &Path,
    retries: u32,
    progress: &ParallelDownloadProgress,
    expected_sha256: Option<&str>,
) -> XpmResult<String> {
    download_first_available_with_progress(
        urls,
        dest,
        retries,
        false,
        Some(progress),
        expected_sha256,
    )
}

fn download_first_available_with_progress(
    urls: &[String],
    dest: &Path,
    retries: u32,
    show_progress: bool,
    progress: Option<&ParallelDownloadProgress>,
    expected_sha256: Option<&str>,
) -> XpmResult<String> {
    let mut last_error: Option<XpmError> = None;

    let _ = retries;
    for url in urls {
        // One attempt per mirror: the next candidate is the retry. Repeating a
        // dead/timing-out mirror delays failover and stalls the worker pool.
        match download_with_retries(
            url,
            dest,
            1,
            show_progress,
            progress,
            expected_sha256,
            None,
            true,
        ) {
            Ok(()) => return Ok(url.clone()),
            Err(err) => {
                tracing::warn!(
                    url,
                    error = %err,
                    "download failed or failed verification; trying next mirror"
                );
                last_error = Some(err);
            }
        }
    }

    Err(last_error.unwrap_or_else(|| XpmError::Database("no usable package URL found".to_string())))
}

pub(super) fn download_with_retries(
    url: &str,
    dest: &Path,
    retries: u32,
    show_progress: bool,
    progress: Option<&ParallelDownloadProgress>,
    expected_sha256: Option<&str>,
    timeout: Option<Duration>,
    warn: bool,
) -> XpmResult<()> {
    let attempts = retries.max(1);
    let mut last_error: Option<XpmError> = None;

    for attempt in 1..=attempts {
        match download_once(url, dest, show_progress, progress, expected_sha256, timeout) {
            Ok(()) => return Ok(()),
            Err(err) => {
                if warn {
                    tracing::warn!(url, attempt, attempts, error = %err, "download attempt failed");
                } else {
                    tracing::debug!(url, attempt, attempts, error = %err, "optional download unavailable");
                }
                if non_retryable(&err) {
                    return Err(err);
                }
                last_error = Some(err);
            }
        }
    }

    Err(last_error.unwrap_or_else(|| XpmError::Database(format!("download failed: {url}"))))
}

pub(super) fn http_client() -> &'static reqwest::blocking::Client {
    static CLIENT: OnceLock<reqwest::blocking::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::blocking::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(300))
            .pool_max_idle_per_host(32)
            .tcp_nodelay(true)
            .user_agent("xpm/0.1")
            .build()
            .expect("failed to build shared HTTP client")
    })
}

pub(super) fn non_retryable(err: &XpmError) -> bool {
    let text = err.to_string();
    if text.contains("429 Too Many Requests") || text.contains("408 Request Timeout") {
        return false;
    }
    text.contains("HTTP status client error (400")
        || text.contains("HTTP status client error (401")
        || text.contains("HTTP status client error (403")
        || text.contains("HTTP status client error (404")
        || text.contains("HTTP status client error (410")
}
