//! Parallel package download prefetch and signature verification.

use super::*;

struct DownloadJob {
    index: usize,
    urls: Vec<String>,
    dest: PathBuf,
    sha256: Option<String>,
}

/// Download all resolved packages before the sequential verification and
/// transaction phases. Network I/O is parallel; signatures, hashes, metadata
/// and transaction ordering stay sequential.
pub(super) fn prefetch_downloads(
    resolved: &[(Repository, RepoEntry, bool)],
    installed: &HashMap<String, String>,
    needed: bool,
    arch: &str,
    cache_dir: &Path,
    parallel_downloads: u32,
) -> Result<Vec<Option<String>>> {
    let mut jobs = Vec::new();
    let mut mirrors = vec![None; resolved.len()];

    for (index, (repo, entry, _)) in resolved.iter().enumerate() {
        if needed
            && installed
                .get(&entry.name)
                .is_some_and(|current| Version::parse(current) == Version::parse(&entry.version))
        {
            continue;
        }
        let dest = cache_dir.join(entry.resolved_filename(arch));
        let mut urls = package_download_candidates(repo, arch, entry);
        if dest.is_file() {
            let cache_valid = match entry.sha256sum.as_deref() {
                Some(sum) => verify_sha256(&dest, sum).is_ok(),
                None => fs::metadata(&dest).is_ok_and(|meta| meta.len() > 0),
            };
            if cache_valid {
                // pacman reuses its package cache; xpm should not re-download
                // every package on every phase or repeated install.
                mirrors[index] = urls.first().cloned();
                continue;
            }
        }
        let server_count = repo.server.len();
        if server_count > 1 && urls.len() >= server_count {
            // Spread concurrent workers across the configured mirrors, but
            // keep derived fallback URLs (native .xp) at the end so they can
            // never be tried before a real repository server.
            urls[..server_count].rotate_left(index % server_count);
        }
        jobs.push(DownloadJob {
            index,
            urls,
            dest,
            sha256: entry.sha256sum.clone(),
        });
    }

    if jobs.is_empty() {
        return Ok(mirrors);
    }

    let workers = (parallel_downloads.max(1) as usize).min(jobs.len());
    if workers == 1 {
        for job in jobs {
            let mirror =
                download_first_available_checked(&job.urls, &job.dest, 3, job.sha256.as_deref())
                    .with_context(|| {
                        format!(
                            "failed to download '{}' from any mirror",
                            job.dest.display()
                        )
                    })?;
            mirrors[job.index] = Some(mirror);
        }
        return Ok(mirrors);
    }

    let results: Mutex<Vec<Option<Result<String, String>>>> =
        Mutex::new((0..jobs.len()).map(|_| None).collect());
    let next = AtomicUsize::new(0);
    let progress = ParallelDownloadProgress::new(jobs.len());
    thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| loop {
                let index = next.fetch_add(1, Ordering::Relaxed);
                let Some(job) = jobs.get(index) else {
                    break;
                };
                let result = download_first_available_tracked(
                    &job.urls,
                    &job.dest,
                    3,
                    &progress,
                    job.sha256.as_deref(),
                )
                .map_err(|err| err.to_string());
                progress.complete_job();
                if let Ok(mut slots) = results.lock() {
                    slots[index] = Some(result);
                }
            });
        }
    });

    let results = results
        .into_inner()
        .unwrap_or_else(|poison| poison.into_inner());
    for (index, result) in results.into_iter().enumerate() {
        match result {
            Some(Ok(mirror)) => mirrors[jobs[index].index] = Some(mirror),
            Some(Err(err)) => {
                anyhow::bail!("failed to download '{}': {err}", jobs[index].dest.display())
            }
            None => anyhow::bail!("download worker did not report a result"),
        }
    }
    Ok(mirrors)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn verify_signatures_parallel(
    resolved: &[(Repository, RepoEntry, bool)],
    mirrors: &[Option<String>],
    installed: &HashMap<String, String>,
    needed: bool,
    arch: &str,
    cache_dir: &Path,
    keyring_path: &Path,
    default_sig_level: SigLevel,
    parallel: u32,
) -> Result<Vec<Option<Result<(), String>>>> {
    struct Job {
        index: usize,
        name: String,
        repo: String,
        dest: PathBuf,
        sig_urls: Vec<String>,
        sig_level: SigLevel,
    }

    let mut jobs = Vec::new();
    for (index, (repo, entry, _)) in resolved.iter().enumerate() {
        if needed
            && installed
                .get(&entry.name)
                .is_some_and(|current| Version::parse(current) == Version::parse(&entry.version))
        {
            continue;
        }
        let Some(mirror) = mirrors[index].clone() else {
            continue;
        };
        let dest = cache_dir.join(entry.resolved_filename(arch));
        let sig_level = repo.sig_level.unwrap_or(default_sig_level);
        let mut sig_urls: Vec<String> = package_download_candidates(repo, arch, entry)
            .into_iter()
            .map(|url| format!("{url}.sig"))
            .collect();
        sig_urls.retain(|url| url != &format!("{mirror}.sig"));
        sig_urls.insert(0, format!("{mirror}.sig"));
        jobs.push(Job {
            index,
            name: entry.name.clone(),
            repo: repo.name.clone(),
            dest,
            sig_urls,
            sig_level,
        });
    }

    let results: Mutex<Vec<Option<Result<(), String>>>> =
        Mutex::new((0..resolved.len()).map(|_| None).collect());
    if jobs.is_empty() {
        return Ok(results
            .into_inner()
            .unwrap_or_else(|poison| poison.into_inner()));
    }

    let workers = (parallel.max(1) as usize).min(jobs.len());
    let verify_progress = Arc::new(OperationProgress::new(
        "Verifying signatures",
        jobs.len(),
        BarFill::Slash,
    ));
    let next = AtomicUsize::new(0);
    thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| loop {
                let job_index = next.fetch_add(1, Ordering::Relaxed);
                let Some(job) = jobs.get(job_index) else {
                    break;
                };
                let result = verify_remote_signature_any(
                    &job.dest,
                    &job.sig_urls,
                    job.sig_level,
                    keyring_path,
                    1,
                )
                .map_err(|err| err.to_string());
                verify_progress.advance();
                if let Ok(mut slots) = results.lock() {
                    slots[job.index] = Some(result.map_err(|err| {
                        format!(
                            "signature verification failed for '{}' from repo '{}': {err}",
                            job.name, job.repo
                        )
                    }));
                }
            });
        }
    });
    verify_progress.finish();

    Ok(results
        .into_inner()
        .unwrap_or_else(|poison| poison.into_inner()))
}
