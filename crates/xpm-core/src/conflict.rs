//! Pacman-compatible file conflict detection.
//!
//! This module performs the same preflight checks as
//! `_alpm_db_find_fileconflicts()` in libalpm: every target is checked
//! against every other target, then each target is checked against the
//! filesystem using the installed package manifests for ownership.

use std::collections::HashSet;
use std::fs;
use std::path::Path;

use glob::{MatchOptions, Pattern};

use crate::error::{XpmError, XpmResult};

/// Files of an installed package, used for ownership lookups.
#[derive(Debug, Clone, Default)]
pub struct InstalledManifest {
    pub name: String,
    pub files: Vec<String>,
    pub backups: Vec<String>,
}

/// Files that a transaction target wants to install.
#[derive(Debug, Clone, Default)]
pub struct TargetManifest {
    pub name: String,
    /// Incoming package files; directories end with `/`.
    pub files: Vec<String>,
    pub backups: Vec<String>,
    /// Files of the installed version (upgrade), empty for new installs.
    pub old_files: Vec<String>,
}

/// The origin of a file conflict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ConflictKind {
    /// Two transaction targets claim the same path.
    TargetTarget,
    /// The path exists and is owned by an installed package.
    FilesystemOwned,
    /// The path exists and no installed package owns it.
    FilesystemUnowned,
}

/// A single file conflict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileConflict {
    /// The target package that triggered the conflict.
    pub target: String,
    /// Rooted path for output.
    pub path: String,
    pub kind: ConflictKind,
    /// Installed package that owns the path, when known.
    pub owner: Option<String>,
    /// The other target, for target-vs-target conflicts.
    pub other_target: Option<String>,
}

#[derive(Debug, Clone)]
struct OverwritePattern {
    negated: bool,
    pattern: Pattern,
}

/// Parsed `--overwrite` patterns.
///
/// Patterns are evaluated from last to first; the first pattern matching
/// either the relative or the rooted path decides the result. A leading
/// `!` negates a pattern, and a leading `\` escapes a literal `!` or `\`.
#[derive(Debug, Clone, Default)]
pub struct OverwritePatterns {
    patterns: Vec<OverwritePattern>,
}

impl OverwritePatterns {
    pub fn parse<I: IntoIterator<Item = S>, S: AsRef<str>>(patterns: I) -> Self {
        let mut parsed = Vec::new();
        for raw in patterns {
            for piece in raw.as_ref().split(',') {
                if piece.is_empty() {
                    continue;
                }

                let (negated, pattern) = if let Some(rest) = piece.strip_prefix('!') {
                    (true, rest)
                } else if let Some(rest) = piece.strip_prefix('\\') {
                    if rest.starts_with('!') || rest.starts_with('\\') {
                        (false, rest)
                    } else {
                        (false, piece)
                    }
                } else {
                    (false, piece)
                };

                if pattern.is_empty() {
                    continue;
                }

                if let Ok(pattern) = Pattern::new(pattern) {
                    parsed.push(OverwritePattern { negated, pattern });
                }
            }
        }
        Self { patterns: parsed }
    }

    pub fn matches(&self, relative: &str, rooted: &str) -> bool {
        let options = MatchOptions {
            case_sensitive: true,
            require_literal_separator: false,
            require_literal_leading_dot: false,
        };

        for overwrite in self.patterns.iter().rev() {
            if overwrite.pattern.matches_with(relative, options)
                || overwrite.pattern.matches_with(rooted, options)
            {
                return !overwrite.negated;
            }
        }
        false
    }

    pub fn is_empty(&self) -> bool {
        self.patterns.is_empty()
    }
}

struct PreparedInstalled {
    name: String,
    files: HashSet<String>,
}

struct PreparedTarget<'a> {
    manifest: &'a TargetManifest,
    files: Vec<String>,
    file_set: HashSet<String>,
    old_files: HashSet<String>,
    backups: HashSet<String>,
}

/// Finds all file conflicts for the given targets.
pub fn check_file_conflicts(
    root: &Path,
    installed: &[InstalledManifest],
    targets: &[TargetManifest],
    overwrite: &OverwritePatterns,
) -> XpmResult<Vec<FileConflict>> {
    let installed = prepare_installed(installed)?;
    let targets = prepare_targets(targets)?;

    let mut conflicts = Vec::new();

    for (index, first) in targets.iter().enumerate() {
        for second in targets.iter().skip(index + 1) {
            find_target_conflicts(root, first, second, overwrite, &mut conflicts);
        }
    }

    for target in &targets {
        find_filesystem_conflicts(root, &installed, target, overwrite, &mut conflicts);
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

fn normalize_list(name: &str, paths: &[String]) -> XpmResult<Vec<String>> {
    let mut seen = HashSet::new();
    let mut normalized = Vec::new();

    for raw in paths {
        if let Some(path) = normalize_path(name, raw)? {
            if seen.insert(path.clone()) {
                normalized.push(path);
            }
        }
    }

    Ok(normalized)
}

fn normalize_path(name: &str, raw: &str) -> XpmResult<Option<String>> {
    if raw.is_empty() || raw == "/" {
        return Ok(None);
    }

    if raw.contains('\0') || raw.starts_with('/') {
        return Err(unsafe_path(name, raw));
    }

    let mut path = raw;
    while let Some(rest) = path.strip_prefix("./") {
        path = rest;
    }

    if path.is_empty() || path == "." {
        return Ok(None);
    }

    if path.split('/').any(|component| component == "..") {
        return Err(unsafe_path(name, raw));
    }

    Ok(Some(path.to_string()))
}

fn unsafe_path(name: &str, path: &str) -> XpmError {
    XpmError::Package(format!("unsafe path in package '{name}': {path}"))
}

fn rooted(root: &Path, relative: &str) -> String {
    root.join(relative).to_string_lossy().into_owned()
}

fn path_cmp(left: &str, right: &str) -> std::cmp::Ordering {
    let left = left.as_bytes();
    let right = right.as_bytes();

    let mut index = 0;
    while index < left.len() && index < right.len() && left[index] == right[index] {
        index += 1;
    }

    let mut left_index = index;
    let mut right_index = index;

    if left_index == left.len() && right_index < right.len() && right[right_index] == b'/' {
        right_index += 1;
    } else if right_index == right.len() && left_index < left.len() && left[left_index] == b'/' {
        left_index += 1;
    }

    left[left_index..].cmp(&right[right_index..])
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
    installed: &[PreparedInstalled],
    target: &PreparedTarget<'_>,
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
        let owner = find_owner(installed, lookup);

        if incoming_directory && !filesystem_directory {
            if target.old_files.contains(lookup) {
                continue;
            }
            if target.backups.contains(lookup) && owner.is_none() {
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
            if target.backups.contains(lookup) && owner.is_none() {
                continue;
            }
        } else {
            if target.backups.contains(lookup) && owner.is_none() {
                continue;
            }
            if overwrite.matches(lookup, &rooted_lookup_str) {
                continue;
            }
        }

        let kind = if owner.is_some() {
            ConflictKind::FilesystemOwned
        } else {
            ConflictKind::FilesystemUnowned
        };

        conflicts.push(FileConflict {
            target: target.manifest.name.clone(),
            path: rooted_lookup_str.into_owned(),
            kind,
            owner,
            other_target: None,
        });
    }
}

fn find_owner(installed: &[PreparedInstalled], path: &str) -> Option<String> {
    installed
        .iter()
        .find(|package| package.files.contains(path))
        .map(|package| package.name.clone())
}

fn dir_belongsto_pkgs(root: &Path, directory: &str, owned: &HashSet<String>) -> bool {
    let entries = match fs::read_dir(root.join(directory)) {
        Ok(entries) => entries,
        Err(_) => return false,
    };

    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => return false,
        };

        let name = match entry.file_name().into_string() {
            Ok(name) => name,
            Err(_) => return false,
        };

        let child = format!("{directory}{name}");
        let metadata = match fs::symlink_metadata(root.join(&child)) {
            Ok(metadata) => metadata,
            Err(_) => return false,
        };

        let child = if metadata.file_type().is_dir() {
            format!("{child}/")
        } else {
            child
        };

        if !owned.contains(&child) {
            return false;
        }

        if metadata.file_type().is_dir() && !dir_belongsto_pkgs(root, &child, owned) {
            return false;
        }
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;
    use tempfile::TempDir;

    fn target(name: &str, files: &[&str]) -> TargetManifest {
        TargetManifest {
            name: name.to_string(),
            files: files.iter().map(|file| file.to_string()).collect(),
            backups: Vec::new(),
            old_files: Vec::new(),
        }
    }

    fn installed(name: &str, files: &[&str]) -> InstalledManifest {
        InstalledManifest {
            name: name.to_string(),
            files: files.iter().map(|file| file.to_string()).collect(),
            backups: Vec::new(),
        }
    }

    fn overwrite(patterns: &[&str]) -> OverwritePatterns {
        OverwritePatterns::parse(patterns.iter().copied())
    }

    fn write_file(root: &Path, relative: &str) {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent");
        }
        fs::write(path, "existing").expect("write file");
    }

    #[test]
    fn unowned_file_conflicts() {
        let temp = TempDir::new().expect("tempdir");
        write_file(temp.path(), "usr/bin/probe");

        let conflicts = check_file_conflicts(
            temp.path(),
            &[],
            &[target("new", &["usr/bin/probe"])],
            &overwrite(&[]),
        )
        .expect("check conflicts");

        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].target, "new");
        assert_eq!(conflicts[0].kind, ConflictKind::FilesystemUnowned);
        assert_eq!(conflicts[0].owner, None);
        assert_eq!(conflicts[0].path, rooted(temp.path(), "usr/bin/probe"));
    }

    #[test]
    fn owned_file_conflicts() {
        let temp = TempDir::new().expect("tempdir");
        write_file(temp.path(), "usr/bin/probe");

        let conflicts = check_file_conflicts(
            temp.path(),
            &[installed("owner", &["usr/bin/probe"])],
            &[target("new", &["usr/bin/probe"])],
            &overwrite(&[]),
        )
        .expect("check conflicts");

        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].kind, ConflictKind::FilesystemOwned);
        assert_eq!(conflicts[0].owner.as_deref(), Some("owner"));
    }

    #[test]
    fn target_against_target_conflict() {
        let temp = TempDir::new().expect("tempdir");

        let conflicts = check_file_conflicts(
            temp.path(),
            &[],
            &[
                target("owner", &["usr/bin/probe"]),
                target("new", &["usr/bin/probe"]),
            ],
            &overwrite(&[]),
        )
        .expect("check conflicts");

        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].kind, ConflictKind::TargetTarget);
        assert_eq!(conflicts[0].target, "owner");
        assert_eq!(conflicts[0].other_target.as_deref(), Some("new"));
    }

    #[test]
    fn shared_directories_are_not_conflicts() {
        let temp = TempDir::new().expect("tempdir");

        let conflicts = check_file_conflicts(
            temp.path(),
            &[],
            &[
                target("shared-a", &["usr/share/common/"]),
                target("shared-b", &["usr/share/common/"]),
            ],
            &overwrite(&[]),
        )
        .expect("check conflicts");

        assert!(conflicts.is_empty());
    }

    #[test]
    fn target_directory_against_target_file_is_not_forced() {
        let temp = TempDir::new().expect("tempdir");

        let conflicts = check_file_conflicts(
            temp.path(),
            &[],
            &[target("dirpkg", &["foo/"]), target("filepkg", &["foo"])],
            &overwrite(&["foo", "foo/"]),
        )
        .expect("check conflicts");

        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].kind, ConflictKind::TargetTarget);
    }

    #[test]
    fn dangling_symlink_counts_as_existing_file() {
        let temp = TempDir::new().expect("tempdir");
        let path = temp.path().join("usr/bin/probe");
        fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        symlink("missing", &path).expect("symlink");

        let conflicts = check_file_conflicts(
            temp.path(),
            &[],
            &[target("new", &["usr/bin/probe"])],
            &overwrite(&[]),
        )
        .expect("check conflicts");

        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].kind, ConflictKind::FilesystemUnowned);
        assert!(fs::symlink_metadata(&path)
            .expect("lstat")
            .file_type()
            .is_symlink());
    }

    #[test]
    fn unowned_backup_file_is_not_a_conflict() {
        let temp = TempDir::new().expect("tempdir");
        write_file(temp.path(), "etc/test.conf");

        let mut incoming = target("conf", &["etc/test.conf"]);
        incoming.backups.push("etc/test.conf".to_string());

        let conflicts = check_file_conflicts(temp.path(), &[], &[incoming], &overwrite(&[]))
            .expect("check conflicts");

        assert!(conflicts.is_empty());
    }

    #[test]
    fn overwrite_pattern_suppresses_file_conflict() {
        let temp = TempDir::new().expect("tempdir");
        write_file(temp.path(), "usr/bin/probe");

        let conflicts = check_file_conflicts(
            temp.path(),
            &[],
            &[target("new", &["usr/bin/probe"])],
            &overwrite(&["usr/bin/*"]),
        )
        .expect("check conflicts");

        assert!(conflicts.is_empty());
    }

    #[test]
    fn negated_overwrite_pattern_wins_last() {
        let temp = TempDir::new().expect("tempdir");
        write_file(temp.path(), "usr/bin/probe");

        let patterns = overwrite(&["usr/bin/probe", "!usr/bin/probe"]);
        let conflicts = check_file_conflicts(
            temp.path(),
            &[],
            &[target("new", &["usr/bin/probe"])],
            &patterns,
        )
        .expect("check conflicts");

        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].kind, ConflictKind::FilesystemUnowned);
    }

    #[test]
    fn overwrite_matches_rooted_path() {
        let temp = TempDir::new().expect("tempdir");
        write_file(temp.path(), "usr/bin/probe");
        let pattern = rooted(temp.path(), "usr/bin/probe");

        let conflicts = check_file_conflicts(
            temp.path(),
            &[],
            &[target("new", &["usr/bin/probe"])],
            &overwrite(&[&pattern]),
        )
        .expect("check conflicts");

        assert!(conflicts.is_empty());
    }

    #[test]
    fn overwrite_patterns_split_commas_and_ignore_empty() {
        let patterns = OverwritePatterns::parse(["", "usr/*,!usr/bin/probe", "foo"]);
        assert!(!patterns.is_empty());
        assert!(patterns.matches("usr/share/x", "/root/usr/share/x"));
        assert!(!patterns.matches("usr/bin/probe", "/root/usr/bin/probe"));
        assert!(patterns.matches("foo", "/root/foo"));
        assert!(OverwritePatterns::parse(["", ","]).is_empty());
    }

    #[test]
    fn overwrite_escapes_leading_bang() {
        let patterns = overwrite(&["\\!literal"]);
        assert!(patterns.matches("!literal", "/root/!literal"));
        assert!(!patterns.matches("literal", "/root/literal"));
    }

    #[test]
    fn upgrade_only_checks_new_files() {
        let temp = TempDir::new().expect("tempdir");
        write_file(temp.path(), "usr/bin/probe");
        write_file(temp.path(), "usr/bin/added");

        let mut incoming = target("pkg", &["usr/bin/probe", "usr/bin/added"]);
        incoming.old_files.push("usr/bin/probe".to_string());

        let conflicts = check_file_conflicts(temp.path(), &[], &[incoming], &overwrite(&[]))
            .expect("check conflicts");

        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].path, rooted(temp.path(), "usr/bin/added"));
    }

    #[test]
    fn imported_owner_is_reported_by_name() {
        let temp = TempDir::new().expect("tempdir");
        write_file(temp.path(), "usr/bin/probe");

        let imported = installed("imported", &["usr/bin/probe"]);
        let conflicts = check_file_conflicts(
            temp.path(),
            &[imported],
            &[target("new", &["usr/bin/probe"])],
            &overwrite(&[]),
        )
        .expect("check conflicts");

        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].owner.as_deref(), Some("imported"));
        assert_eq!(conflicts[0].kind, ConflictKind::FilesystemOwned);
    }

    #[test]
    fn empty_file_manifest_does_not_provide_ownership() {
        let temp = TempDir::new().expect("tempdir");
        write_file(temp.path(), "usr/bin/virt");

        let virtual_package = installed("virtual-b", &[]);
        let conflicts = check_file_conflicts(
            temp.path(),
            &[virtual_package],
            &[target("new", &["usr/bin/virt"])],
            &overwrite(&[]),
        )
        .expect("check conflicts");

        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].kind, ConflictKind::FilesystemUnowned);
        assert_eq!(conflicts[0].owner, None);
    }

    #[test]
    fn file_cannot_replace_unowned_directory_even_with_overwrite() {
        let temp = TempDir::new().expect("tempdir");
        write_file(temp.path(), "foo/child");

        let conflicts = check_file_conflicts(
            temp.path(),
            &[],
            &[target("new", &["foo"])],
            &overwrite(&["foo"]),
        )
        .expect("check conflicts");

        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].kind, ConflictKind::FilesystemUnowned);
        assert_eq!(conflicts[0].path, rooted(temp.path(), "foo"));
    }

    #[test]
    fn directory_can_replace_file_with_overwrite() {
        let temp = TempDir::new().expect("tempdir");
        write_file(temp.path(), "foo");

        let conflicts = check_file_conflicts(
            temp.path(),
            &[],
            &[target("new", &["foo/"])],
            &overwrite(&[]),
        )
        .expect("check conflicts");
        assert_eq!(conflicts.len(), 1);

        let conflicts = check_file_conflicts(
            temp.path(),
            &[],
            &[target("new", &["foo/"])],
            &overwrite(&["foo"]),
        )
        .expect("check conflicts");
        assert!(conflicts.is_empty());
    }

    #[test]
    fn directory_tree_owned_by_old_package_can_be_replaced() {
        let temp = TempDir::new().expect("tempdir");
        write_file(temp.path(), "foo/x");

        let mut incoming = target("pkg", &["foo"]);
        incoming.old_files.push("foo/".to_string());
        incoming.old_files.push("foo/x".to_string());

        let conflicts = check_file_conflicts(temp.path(), &[], &[incoming], &overwrite(&[]))
            .expect("check conflicts");

        assert!(conflicts.is_empty());
    }

    #[test]
    fn directory_tree_with_unowned_child_conflicts() {
        let temp = TempDir::new().expect("tempdir");
        write_file(temp.path(), "foo/x");
        write_file(temp.path(), "foo/y");

        let mut incoming = target("pkg", &["foo"]);
        incoming.old_files.push("foo/".to_string());
        incoming.old_files.push("foo/x".to_string());

        let conflicts = check_file_conflicts(temp.path(), &[], &[incoming], &overwrite(&[]))
            .expect("check conflicts");

        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].path, rooted(temp.path(), "foo"));
    }

    #[test]
    fn unsafe_target_paths_are_rejected() {
        let temp = TempDir::new().expect("tempdir");

        for path in ["/etc/passwd", "../escape", "usr/../../escape"] {
            let result =
                check_file_conflicts(temp.path(), &[], &[target("bad", &[path])], &overwrite(&[]));
            assert!(matches!(result, Err(XpmError::Package(_))));
        }
    }

    #[test]
    fn leading_dot_slash_is_stripped_and_duplicates_removed() {
        let temp = TempDir::new().expect("tempdir");

        let conflicts = check_file_conflicts(
            temp.path(),
            &[],
            &[target("pkg", &["./foo", "./foo", "", ".", "./", "/"])],
            &overwrite(&[]),
        )
        .expect("check conflicts");

        assert!(conflicts.is_empty());
    }

    #[test]
    fn conflicts_are_sorted_by_path_then_target() {
        let temp = TempDir::new().expect("tempdir");
        write_file(temp.path(), "z");
        write_file(temp.path(), "a");

        let conflicts = check_file_conflicts(
            temp.path(),
            &[],
            &[target("new", &["z", "a"])],
            &overwrite(&[]),
        )
        .expect("check conflicts");

        let paths: Vec<&str> = conflicts
            .iter()
            .map(|conflict| conflict.path.as_str())
            .collect();
        assert_eq!(
            paths,
            vec![rooted(temp.path(), "a"), rooted(temp.path(), "z"),]
        );
    }
}
