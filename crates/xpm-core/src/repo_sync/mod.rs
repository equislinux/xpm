//! Remote repository database synchronization.

use std::fs;
use std::io::{self, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use crate::config::{Repository, SigLevel};
use crate::progress::{format_mib, percentage, ParallelDownloadProgress};
use crate::repo_db::RepoEntry;
use crate::signing::{verify_file, VerifyOutcome};
use crate::{XpmError, XpmResult};
use sha2::{Digest, Sha256};

/// Database and signature downloads are small; a stalled mirror must fail
/// over quickly instead of blocking the whole sync for the package timeout.
const METADATA_TIMEOUT: Duration = Duration::from_secs(60);

/// Result metadata for a repository sync operation.
#[derive(Debug, Clone)]
pub struct RepoSyncResult {
    pub mirror: String,
    pub db_downloaded: bool,
    pub files_downloaded: bool,
}

/// Expand `$repo` and `$arch` placeholders in a repository server URL.
pub fn expand_repo_url(server: &str, repo: &str, arch: &str) -> String {
    server.replace("$repo", repo).replace("$arch", arch)
}

/// Download `<repo>.db` and optional `<repo>.files` into `sync_dir`.
///
/// The operation tries each configured mirror until one succeeds for `.db`.
/// The `.files` download is best-effort and does not fail the whole sync.
mod db;
mod download;
mod progress;
mod transfer;
mod verify;

pub use db::sync_repo_databases;
pub use download::{
    download_first_available, download_first_available_checked, download_first_available_quiet,
    download_first_available_tracked, package_download_candidates,
};
pub use verify::{verify_remote_signature, verify_remote_signature_any, verify_sha256};

use download::{download_with_retries, http_client};
use progress::DownloadProgress;
use transfer::download_once;

#[cfg(test)]
mod tests;
