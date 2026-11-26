//! Full `.MTREE` verification.

use super::*;

pub(super) fn check_mtree(
    root: &Path,
    local_db_dir: &Path,
    package: &str,
    manifest: &[String],
    report: &mut PackageCheck,
) -> XpmResult<()> {
    let backups: HashSet<String> = local_db::read_backup_list(local_db_dir, package)?
        .iter()
        .filter_map(|entry| normalize_entry(entry))
        .collect();

    let Some(raw) = local_db::read_mtree(local_db_dir, package)? else {
        report.metadata_missing = true;
        return Ok(());
    };

    let entries = parse_mtree(&raw).map_err(|err| {
        XpmError::Package(format!("invalid stored .MTREE for '{package}': {err}"))
    })?;

    let mut known = HashSet::new();
    for entry in entries {
        let Some(relative) = normalize_entry(&entry.path.to_string_lossy()) else {
            continue;
        };
        if METADATA_PATHS.contains(&relative.as_str()) {
            continue;
        }
        known.insert(relative.clone());
        verify_entry(root, &entry, relative, &backups, report)?;
    }

    // Manifest entries without a matching .MTREE record cannot be verified.
    for entry in manifest {
        let Some(relative) = normalize_entry(entry) else {
            continue;
        };
        if !known.contains(&relative) {
            report.metadata_issues.push(relative);
        }
    }

    Ok(())
}

fn verify_entry(
    root: &Path,
    entry: &MtreeEntry,
    relative: String,
    backups: &HashSet<String>,
    report: &mut PackageCheck,
) -> XpmResult<()> {
    let is_backup = backups.contains(&relative);
    let path = root.join(&relative);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            report.missing.push(relative);
            return Ok(());
        }
        Err(err) => {
            return Err(XpmError::Io(err));
        }
    };

    let actual_type = if metadata.file_type().is_dir() {
        MtreeFileType::Dir
    } else if metadata.file_type().is_symlink() {
        MtreeFileType::Link
    } else {
        MtreeFileType::File
    };

    if actual_type != entry.file_type {
        report.altered.push(relative);
        return Ok(());
    }

    match entry.file_type {
        MtreeFileType::Dir => {
            if !is_backup && mode(&metadata) != entry.mode & 0o7777 {
                report.altered.push(relative);
            }
        }
        MtreeFileType::File => {
            if !is_backup && mode(&metadata) != entry.mode & 0o7777 {
                report.altered.push(relative.clone());
            }

            match entry.sha256.as_deref() {
                Some(expected) if !is_backup => {
                    let actual = sha256_file(&path)?;
                    if !actual.eq_ignore_ascii_case(expected) {
                        report.altered.push(relative);
                    }
                }
                Some(_) => {}
                None => report.metadata_issues.push(relative),
            }
        }
        MtreeFileType::Link => match entry.link_target.as_deref() {
            Some(expected) => {
                let actual = fs::read_link(&path)?;
                if actual.to_string_lossy() != expected {
                    report.altered.push(relative);
                }
            }
            None => report.metadata_issues.push(relative),
        },
    }

    Ok(())
}

#[cfg(unix)]
fn mode(metadata: &fs::Metadata) -> u32 {
    metadata.permissions().mode() & 0o7777
}

fn sha256_file(path: &Path) -> XpmResult<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 8192];

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
