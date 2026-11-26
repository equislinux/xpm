//! Local/sync database readers and transaction manifests shared by commands.

use super::*;

/// A package installed or reinstalled during a transaction.
pub(super) struct InstalledRecord {
    pub(super) name: String,
    pub(super) repo: String,
    pub(super) depends: Vec<String>,
    pub(super) provides: Vec<String>,
    pub(super) groups: Vec<String>,
    pub(super) reason: InstallReason,
}

/// Build a resolver candidate from a sync database entry.
pub(super) fn resolver_candidate(repo: &str, entry: &RepoEntry) -> PackageCandidate {
    PackageCandidate {
        name: entry.name.clone(),
        version: Version::parse(&entry.version),
        repo: Some(repo.to_string()),
        depends: entry
            .depends
            .iter()
            .map(|d| DepConstraint::parse(d))
            .collect(),
        conflicts: entry
            .conflicts
            .iter()
            .map(|c| DepConstraint::parse(c))
            .collect(),
        provides: entry
            .provides
            .iter()
            .map(|p| DepConstraint::parse(p))
            .collect(),
        replaces: entry
            .replaces
            .iter()
            .map(|r| DepConstraint::parse(r))
            .collect(),
        optdepends: entry.opt_depends.clone(),
    }
}

pub(super) fn load_installed_pkgs(local_db_dir: &Path) -> Result<Vec<depcheck::InstalledPkg>> {
    let installed = read_installed_versions(local_db_dir)?;
    let mut packages = Vec::with_capacity(installed.len());

    for (name, version) in installed {
        let depends = local_db::read_depends(local_db_dir, &name)?
            .unwrap_or_default()
            .into_iter()
            .map(|dep| DepConstraint::parse(&dep))
            .collect();
        let provides = local_db::read_provides(local_db_dir, &name)?
            .into_iter()
            .map(|provide| DepConstraint::parse(&provide))
            .collect();
        packages.push(depcheck::InstalledPkg {
            name: name.clone(),
            version: Version::parse(&version),
            depends,
            provides,
            reason: InstallReason::read(local_db_dir, &name),
        });
    }

    Ok(packages)
}

pub(super) fn assume_installed_pkgs(
    candidates: &[PackageCandidate],
) -> Vec<depcheck::InstalledPkg> {
    candidates
        .iter()
        .map(|candidate| depcheck::InstalledPkg {
            name: candidate.name.clone(),
            version: candidate.version.clone(),
            depends: Vec::new(),
            provides: Vec::new(),
            reason: InstallReason::Explicit,
        })
        .collect()
}

pub(super) fn load_installed_candidates(local_db_dir: &Path) -> Result<Vec<PackageCandidate>> {
    let installed = read_installed_versions(local_db_dir)?;
    let mut candidates = Vec::with_capacity(installed.len());

    for (name, version) in installed {
        let provides = local_db::read_provides(local_db_dir, &name)?
            .into_iter()
            .map(|provide| DepConstraint::parse(&provide))
            .collect();
        candidates.push(PackageCandidate {
            name,
            version: Version::parse(&version),
            repo: None,
            depends: Vec::new(),
            conflicts: Vec::new(),
            provides,
            replaces: Vec::new(),
            optdepends: Vec::new(),
        });
    }

    Ok(candidates)
}

pub(super) fn read_installed_versions(local_db_dir: &Path) -> Result<HashMap<String, String>> {
    let mut installed = HashMap::new();

    if !local_db_dir.exists() {
        return Ok(installed);
    }

    for entry in std::fs::read_dir(local_db_dir)
        .with_context(|| format!("failed to read {}", local_db_dir.display()))?
    {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if !file_type.is_dir() {
            continue;
        }

        let pkg_name = entry.file_name().to_string_lossy().into_owned();
        let version_path = entry.path().join("version");
        if !version_path.exists() {
            continue;
        }

        let version = std::fs::read_to_string(&version_path)
            .with_context(|| format!("failed to read {}", version_path.display()))?
            .trim()
            .to_string();

        if !version.is_empty() {
            installed.insert(pkg_name, version);
        }
    }

    Ok(installed)
}

pub(super) fn read_latest_remote_entries(
    config: &XpmConfig,
    sync_dir: &Path,
) -> Result<HashMap<String, (Repository, xpm_core::repo_db::RepoEntry)>> {
    let mut latest = HashMap::new();

    for repo in &config.repositories {
        let db_path = sync_dir.join(format!("{}.db", repo.name));
        if !db_path.exists() {
            continue;
        }

        let db = parse_sync_db_cached(&db_path, &repo.name)
            .with_context(|| format!("failed to parse sync db {}", db_path.display()))?;

        // Preserve repository priority order: first repo that contains a package wins.
        let mut best_by_name: HashMap<String, xpm_core::repo_db::RepoEntry> = HashMap::new();
        for entry in db.entries {
            match best_by_name.get(&entry.name) {
                Some(existing) => {
                    if Version::cmp_versions(&entry.version, &existing.version).is_gt() {
                        best_by_name.insert(entry.name.clone(), entry);
                    }
                }
                None => {
                    best_by_name.insert(entry.name.clone(), entry);
                }
            }
        }

        for (name, entry) in best_by_name {
            latest.entry(name).or_insert_with(|| (repo.clone(), entry));
        }
    }

    Ok(latest)
}

pub(super) fn install_target(meta: &PackageMeta, version: &str) -> depcheck::InstallTarget {
    depcheck::InstallTarget {
        name: meta.name.clone(),
        version: Version::parse(version),
        depends: meta
            .depends
            .iter()
            .map(|dep| DepConstraint::parse(dep))
            .collect(),
        provides: meta
            .provides
            .iter()
            .map(|provide| DepConstraint::parse(provide))
            .collect(),
        conflicts: meta
            .conflicts
            .iter()
            .map(|conflict| DepConstraint::parse(conflict))
            .collect(),
    }
}

pub(super) fn target_manifest(
    local_db_dir: &Path,
    name: &str,
    metadata: &PackageMetadata,
) -> Result<conflict::TargetManifest> {
    let old_files = if local_db_dir.join(name).is_dir() {
        local_db::read_files(local_db_dir, name)?
    } else {
        Vec::new()
    };
    Ok(conflict::TargetManifest {
        name: name.to_string(),
        files: metadata.files.clone(),
        backups: metadata.meta.backup.clone(),
        old_files,
    })
}

pub(super) fn load_installed_manifests(
    local_db_dir: &Path,
) -> Result<Vec<conflict::InstalledManifest>> {
    let mut manifests = Vec::new();
    if !local_db_dir.exists() {
        return Ok(manifests);
    }

    for entry in std::fs::read_dir(local_db_dir)
        .with_context(|| format!("failed to read {}", local_db_dir.display()))?
    {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        manifests.push(conflict::InstalledManifest {
            files: local_db::read_files(local_db_dir, &name)?,
            backups: local_db::read_backup_list(local_db_dir, &name)?,
            name,
        });
    }
    Ok(manifests)
}

pub(super) fn check_transaction_file_conflicts(
    config: &XpmConfig,
    local_db_dir: &Path,
    targets: &[conflict::TargetManifest],
    removed: &[String],
    overwrite: &conflict::OverwritePatterns,
) -> Result<()> {
    if targets.is_empty() {
        return Ok(());
    }
    let removed: std::collections::HashSet<String> = removed.iter().cloned().collect();
    let installed = load_installed_manifests(local_db_dir)?;
    let conflicts = conflict::check_file_conflicts_with_removals(
        &config.options.root_dir,
        &installed,
        targets,
        &removed,
        overwrite,
    )
    .context("failed to check file conflicts")?;

    if conflicts.is_empty() {
        return Ok(());
    }

    eprintln!("error: failed to commit transaction (conflicting files)");
    for file_conflict in &conflicts {
        match file_conflict.kind {
            conflict::ConflictKind::TargetTarget => {
                eprintln!(
                    "{}: {} exists in both '{}' and '{}'",
                    file_conflict.target,
                    file_conflict.path,
                    file_conflict.target,
                    file_conflict.other_target.as_deref().unwrap_or("?")
                );
            }
            conflict::ConflictKind::FilesystemOwned => {
                eprintln!(
                    "{}: {} exists in filesystem (owned by {})",
                    file_conflict.target,
                    file_conflict.path,
                    file_conflict.owner.as_deref().unwrap_or("?")
                );
            }
            conflict::ConflictKind::FilesystemUnowned => {
                eprintln!(
                    "{}: {} exists in filesystem",
                    file_conflict.target, file_conflict.path
                );
            }
        }
    }
    anyhow::bail!("failed to commit transaction (conflicting files)");
}
