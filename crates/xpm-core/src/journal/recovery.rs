//! Crash recovery for transactions left in the `running` state.
//!
//! A transaction journal is written before the filesystem is touched and
//! finalized after the commit. If xpm is killed between those two points, the
//! package files and the local database `version` may already describe the new
//! package while `depends`/`provides` still describe the old one.
//!
//! [`reconcile_running`] completes those records from the cached package file.
//! It only touches packages whose installed `version` already matches the
//! journal target, and it never rewrites `files`, `mtree` or the payload.

use std::collections::HashSet;
use std::fs;
use std::io::Read;
use std::path::Path;

use sha2::{Digest, Sha256};

use super::Journal;
use crate::cache;
use crate::error::XpmResult;
use crate::install_reason::InstallReason;
use crate::local_db;
use crate::package::read_metadata;

/// Result of a recovery pass.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RecoveryReport {
    /// Packages whose metadata record was rewritten (`name version`).
    pub reconciled: Vec<String>,
    /// Packages that could not be reconciled, with the reason.
    pub unresolved: Vec<String>,
}

/// Reconciles every interrupted (`running`) transaction in `db_path`.
///
/// Journals are inspected newest first: when several interrupted transactions
/// touched the same package, only the newest committed version is recovered.
pub fn reconcile_running(db_path: &Path, cache_dir: &Path) -> XpmResult<RecoveryReport> {
    let journal_dir = db_path.join("journal");
    let local_db_dir = db_path.join("local");
    let mut report = RecoveryReport::default();
    let mut handled: HashSet<String> = HashSet::new();

    let mut journals = Journal::list(&journal_dir)?;
    journals.retain(|journal| journal.result == "running");

    for mut journal in journals {
        let mut pending = Vec::new();

        for package in &journal.packages {
            let Some(version) = package.to.as_deref() else {
                continue;
            };
            if local_db::read_version(&local_db_dir, &package.name).as_deref() != Some(version) {
                // The operation never reached the version record.
                continue;
            }
            if !handled.insert(package.name.clone()) {
                continue;
            }

            let Some(pkg_file) = cache::find_package(cache_dir, &package.name, version)? else {
                pending.push(format!("{} {version} (package not in cache)", package.name));
                continue;
            };

            if let Some(expected) = package.sha256.as_deref() {
                if sha256_file(&pkg_file)? != expected {
                    pending.push(format!("{} {version} (checksum mismatch)", package.name));
                    continue;
                }
            }

            let metadata = read_metadata(&pkg_file)?;
            if metadata.meta.name != package.name || metadata.meta.full_version() != version {
                pending.push(format!(
                    "{} {version} (cached package has a different identity)",
                    package.name
                ));
                continue;
            }

            let reason = InstallReason::read_optional(&local_db_dir, &package.name)
                .unwrap_or(InstallReason::Explicit);
            let origin = local_db::read_origin(&local_db_dir, &package.name)
                .or_else(|| package.repo.clone());
            let record = local_db::Record::from_meta(&metadata.meta, reason, origin.as_deref());
            local_db::write_record(&local_db_dir, &record)?;
            report
                .reconciled
                .push(format!("{} {version}", package.name));
        }

        if pending.is_empty() {
            journal.finish("recovered", None)?;
        } else {
            report.unresolved.extend(pending.iter().cloned());
            journal.finish(
                "failed",
                Some(format!(
                    "interrupted; metadata still unresolved: {}",
                    pending.join(", ")
                )),
            )?;
        }
    }

    Ok(report)
}

fn sha256_file(path: &Path) -> XpmResult<String> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}
