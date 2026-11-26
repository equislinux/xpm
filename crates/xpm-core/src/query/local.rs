//! Local package loading.

use super::*;

/// An installed package plus the metadata needed by the query commands.
#[derive(Debug, Clone)]
pub struct LocalPackage {
    pub name: String,
    pub version: String,
    pub reason: InstallReason,
    pub depends: Vec<String>,
    pub provides: Vec<String>,
    pub files: Vec<String>,
}

impl LocalPackage {
    /// Dependency-tester view of this package.
    pub fn dependency_view(&self) -> InstalledDependency {
        InstalledDependency::new(self.name.clone(), &self.version, &self.provides)
    }
}

/// Loads every installed package from the xpm local database.
pub fn load_local_packages(local_db_dir: &Path) -> XpmResult<Vec<LocalPackage>> {
    let mut packages = Vec::new();
    if !local_db_dir.exists() {
        return Ok(packages);
    }

    for entry in fs::read_dir(local_db_dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }

        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let Some(version) = local_db::read_version(local_db_dir, &name) else {
            continue;
        };

        packages.push(LocalPackage {
            reason: InstallReason::read(local_db_dir, &name),
            depends: local_db::read_depends(local_db_dir, &name)?.unwrap_or_default(),
            provides: local_db::read_provides(local_db_dir, &name)?,
            files: local_db::read_files(local_db_dir, &name)?,
            name,
            version,
        });
    }

    packages.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(packages)
}
