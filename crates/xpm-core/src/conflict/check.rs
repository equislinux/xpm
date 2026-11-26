//! Conflict checks against targets and the live filesystem.

use super::*;

struct PreparedTarget<'a> {
    manifest: &'a TargetManifest,
    files: Vec<String>,
    file_set: HashSet<String>,
    old_files: HashSet<String>,
    backups: HashSet<String>,
}

/// Finds all file conflicts for the given targets.
///
/// This is the compatibility wrapper for callers that do not replace any
/// installed package (plain installs). Upgrades and replacements must use
/// [`check_file_conflicts_with_removals`] so files moving from an old package
/// to another package in the same transaction are not reported as conflicts.
pub fn check_file_conflicts(
    root: &Path,
    installed: &[InstalledManifest],
    targets: &[TargetManifest],
    overwrite: &OverwritePatterns,
) -> XpmResult<Vec<FileConflict>> {
    check_file_conflicts_with_removals(root, installed, targets, &HashSet::new(), overwrite)
}

/// Finds all file conflicts, treating the files of packages named in
/// `removed` as transferable to other targets in the same transaction.
///
/// `removed` contains every installed package whose old version is going away
/// (upgrades, replacements and removals). A path owned by one of those packages
/// is free for another target to claim, unless a package outside the
/// transaction still owns it. This mirrors libalpm's `removed_files` handling.
pub fn check_file_conflicts_with_removals(
    root: &Path,
    installed: &[InstalledManifest],
    targets: &[TargetManifest],
    removed: &HashSet<String>,
    overwrite: &OverwritePatterns,
) -> XpmResult<Vec<FileConflict>> {
    let installed = prepare_installed(installed)?;
    let owners = OwnerIndex::new(&installed);
    let targets = prepare_targets(targets)?;

    let mut conflicts = Vec::new();

    for (index, first) in targets.iter().enumerate() {
        for second in targets.iter().skip(index + 1) {
            find_target_conflicts(root, first, second, overwrite, &mut conflicts);
        }
    }

    for target in &targets {
        find_filesystem_conflicts(root, &owners, target, removed, overwrite, &mut conflicts);
    }

    conflicts.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then_with(|| left.target.cmp(&right.target))
            .then_with(|| left.other_target.cmp(&right.other_target))
            .then_with(|| left.kind.cmp(&right.kind))
            .then_with(|| left.owner.cmp(&right.owner))
    });

    Ok(conflicts)
}

fn prepare_installed(installed: &[InstalledManifest]) -> XpmResult<Vec<PreparedInstalled>> {
    installed
        .iter()
        .map(|manifest| {
            let files = normalize_list(&manifest.name, &manifest.files)?;
            Ok(PreparedInstalled {
                name: manifest.name.clone(),
                files: files.into_iter().collect(),
            })
        })
        .collect()
}

fn prepare_targets(targets: &[TargetManifest]) -> XpmResult<Vec<PreparedTarget<'_>>> {
    targets
        .iter()
        .map(|manifest| {
            let mut files = normalize_list(&manifest.name, &manifest.files)?;
            let old_files_list = normalize_list(&manifest.name, &manifest.old_files)?;
            let backups_list = normalize_list(&manifest.name, &manifest.backups)?;

            files.sort();
            let file_set = files.iter().cloned().collect();

            Ok(PreparedTarget {
                manifest,
                files,
                file_set,
                old_files: old_files_list.into_iter().collect(),
                backups: backups_list.into_iter().collect(),
            })
        })
        .collect()
}

fn find_target_conflicts(
    root: &Path,
    first: &PreparedTarget<'_>,
    second: &PreparedTarget<'_>,
    overwrite: &OverwritePatterns,
    conflicts: &mut Vec<FileConflict>,
) {
    let mut left = 0;
    let mut right = 0;

    while left < first.files.len() && right < second.files.len() {
        match path_cmp(&first.files[left], &second.files[right]) {
            std::cmp::Ordering::Less => left += 1,
            std::cmp::Ordering::Greater => right += 1,
            std::cmp::Ordering::Equal => {
                let first_path = &first.files[left];
                let second_path = &second.files[right];
                let both_directories = first_path.ends_with('/') && second_path.ends_with('/');

                if !both_directories {
                    let path = rooted(root, first_path);
                    let forced = overwrite.matches(first_path, &path)
                        && second.file_set.contains(first_path);

                    if !forced {
                        conflicts.push(FileConflict {
                            target: first.manifest.name.clone(),
                            path,
                            kind: ConflictKind::TargetTarget,
                            owner: None,
                            other_target: Some(second.manifest.name.clone()),
                        });
                    }
                }

                left += 1;
                right += 1;
            }
        }
    }
}

fn find_filesystem_conflicts(
    root: &Path,
    owners: &OwnerIndex,
    target: &PreparedTarget<'_>,
    removed: &HashSet<String>,
    overwrite: &OverwritePatterns,
    conflicts: &mut Vec<FileConflict>,
) {
    for relative in &target.files {
        if target.old_files.contains(relative) {
            continue;
        }

        let stat_relative = relative.trim_end_matches('/');
        let rooted_path = root.join(stat_relative);
        let metadata = match fs::symlink_metadata(&rooted_path) {
            Ok(metadata) => metadata,
            Err(_) => continue,
        };

        let incoming_directory = relative.ends_with('/');
        let filesystem_directory = metadata.file_type().is_dir();

        if incoming_directory && filesystem_directory {
            continue;
        }

        let lookup = if incoming_directory {
            &relative[..relative.len() - 1]
        } else {
            relative.as_str()
        };
        let rooted_lookup = root.join(lookup);
        let rooted_lookup_str = rooted_lookup.to_string_lossy();
        let owners = owners.find(lookup, removed);

        if incoming_directory && !filesystem_directory {
            if target.old_files.contains(lookup) {
                continue;
            }
            if target.backups.contains(lookup) && !owners.any {
                continue;
            }
            if overwrite.matches(lookup, &rooted_lookup_str) {
                continue;
            }
        } else if !incoming_directory && filesystem_directory {
            let directory = format!("{lookup}/");
            if target.old_files.contains(&directory)
                && dir_belongsto_pkgs(root, &directory, &target.old_files)
            {
                continue;
            }
            if target.backups.contains(lookup) && !owners.any {
                continue;
            }
        } else {
            if target.backups.contains(lookup) && !owners.any {
                continue;
            }
            if overwrite.matches(lookup, &rooted_lookup_str) {
                continue;
            }
        }

        // The file is leaving with a package that is upgraded, replaced or
        // removed in this same transaction: another target may take it over.
        // If a package outside the transaction still owns it, it stays a
        // conflict (reported below with that package's name).
        if owners.all_removed {
            continue;
        }

        let kind = if owners.remaining.is_some() {
            ConflictKind::FilesystemOwned
        } else {
            ConflictKind::FilesystemUnowned
        };

        conflicts.push(FileConflict {
            target: target.manifest.name.clone(),
            path: rooted_lookup_str.into_owned(),
            kind,
            owner: owners.remaining,
            other_target: None,
        });
    }
}
