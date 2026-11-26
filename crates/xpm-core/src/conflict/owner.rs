//! Installed-file ownership index.

use super::*;

pub(super) struct PreparedInstalled {
    pub(super) name: String,
    pub(super) files: HashSet<String>,
}

pub(super) struct OwnerIndex {
    by_path: HashMap<String, Vec<String>>,
}

impl OwnerIndex {
    pub(super) fn new(installed: &[PreparedInstalled]) -> Self {
        let mut by_path: HashMap<String, Vec<String>> = HashMap::new();
        for package in installed {
            for path in &package.files {
                by_path
                    .entry(path.clone())
                    .or_default()
                    .push(package.name.clone());
            }
        }
        Self { by_path }
    }

    pub(super) fn find(&self, path: &str, removed: &HashSet<String>) -> Owners {
        let directory = format!("{path}/");
        let mut owners = Owners::default();
        for key in [path, directory.as_str()] {
            let Some(names) = self.by_path.get(key) else {
                continue;
            };
            for name in names {
                owners.any = true;
                if !removed.contains(name) {
                    owners.remaining = Some(name.clone());
                    return owners;
                }
            }
        }
        owners.all_removed = owners.any;
        owners
    }
}

#[derive(Debug, Default)]
pub(super) struct Owners {
    /// First owner that is not leaving in this transaction.
    pub(super) remaining: Option<String>,
    /// At least one installed package lists the path.
    pub(super) any: bool,
    /// There is an owner and every owner is leaving in this transaction.
    pub(super) all_removed: bool,
}

pub(super) fn dir_belongsto_pkgs(root: &Path, directory: &str, owned: &HashSet<String>) -> bool {
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
