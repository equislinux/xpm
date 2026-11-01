//! Install-reason metadata for the local database.
//!
//! `xpm install` records `<db_path>/local/<pkg>/reason` with `explicit`
//! (user-requested) or `dep` (pulled in as a dependency). `xpm upgrade`
//! preserves the previous value. A missing file counts as `explicit`, so
//! packages installed before this metadata existed keep working.
//! See `docs/GENERATIONS.md` section 4.

use std::fs;
use std::path::Path;

use crate::error::XpmResult;

/// File name inside each package's local database entry.
pub const REASON_FILE: &str = "reason";

/// Value recorded for user-requested packages.
pub const REASON_EXPLICIT: &str = "explicit";

/// Value recorded for packages installed as dependencies.
pub const REASON_DEP: &str = "dep";

/// Why a package was installed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallReason {
    Explicit,
    Dep,
}

impl InstallReason {
    /// On-disk representation of this reason.
    pub fn as_str(self) -> &'static str {
        match self {
            InstallReason::Explicit => REASON_EXPLICIT,
            InstallReason::Dep => REASON_DEP,
        }
    }

    /// Parses the contents of a `reason` file. Unknown or empty contents and
    /// a missing file default to [`InstallReason::Explicit`].
    pub fn parse(raw: Option<&str>) -> Self {
        match raw.map(str::trim) {
            Some(value) if value.eq_ignore_ascii_case(REASON_DEP) => InstallReason::Dep,
            _ => InstallReason::Explicit,
        }
    }

    /// Reads the reason recorded for an installed package. A missing file
    /// counts as explicit.
    pub fn read(local_db_dir: &Path, pkg: &str) -> Self {
        Self::read_optional(local_db_dir, pkg).unwrap_or(InstallReason::Explicit)
    }

    /// Like [`InstallReason::read`], but distinguishes a missing file
    /// (`None`) from an explicitly recorded `explicit` reason.
    pub fn read_optional(local_db_dir: &Path, pkg: &str) -> Option<Self> {
        let path = local_db_dir.join(pkg).join(REASON_FILE);
        fs::read_to_string(path)
            .ok()
            .map(|raw| Self::parse(Some(&raw)))
    }

    /// Records the reason for an installed package, creating its local
    /// database directory if needed.
    pub fn write(self, local_db_dir: &Path, pkg: &str) -> XpmResult<()> {
        let pkg_dir = local_db_dir.join(pkg);
        fs::create_dir_all(&pkg_dir)?;
        fs::write(pkg_dir.join(REASON_FILE), self.as_str())?;
        Ok(())
    }
}

/// Marks every package in `packages` with `reason` (`pacman -D --asdeps` /
/// `--asexplicit`). Every package is validated before any file is written, so
/// an unknown name leaves the local database untouched.
pub fn mark_packages(
    local_db_dir: &Path,
    packages: &[String],
    reason: InstallReason,
) -> XpmResult<usize> {
    for package in packages {
        if crate::local_db::read_version(local_db_dir, package).is_none() {
            return Err(crate::XpmError::PackageNotFound {
                name: package.clone(),
            });
        }
    }

    for package in packages {
        reason.write(local_db_dir, package)?;
    }

    Ok(packages.len())
}

/// Keeps only the packages whose recorded reason matches `wanted`.
pub fn retain_by_reason(
    local_db_dir: &Path,
    packages: &mut Vec<(String, String)>,
    wanted: InstallReason,
) {
    packages.retain(|(name, _)| InstallReason::read(local_db_dir, name) == wanted);
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn parse_defaults_to_explicit() {
        assert_eq!(InstallReason::parse(None), InstallReason::Explicit);
        assert_eq!(InstallReason::parse(Some("")), InstallReason::Explicit);
        assert_eq!(
            InstallReason::parse(Some("explicit")),
            InstallReason::Explicit
        );
        assert_eq!(InstallReason::parse(Some("dep\n")), InstallReason::Dep);
        assert_eq!(
            InstallReason::parse(Some("unexpected")),
            InstallReason::Explicit
        );
    }

    #[test]
    fn write_and_read_roundtrip() {
        let tmp = TempDir::new().expect("tmp");
        let local_db = tmp.path().join("local");

        InstallReason::Dep
            .write(&local_db, "libfoo")
            .expect("write dep");
        assert_eq!(InstallReason::read(&local_db, "libfoo"), InstallReason::Dep);

        InstallReason::Explicit
            .write(&local_db, "libbar")
            .expect("write explicit");
        assert_eq!(
            InstallReason::read(&local_db, "libbar"),
            InstallReason::Explicit
        );

        let raw = fs::read_to_string(local_db.join("libfoo").join(REASON_FILE)).expect("read file");
        assert_eq!(raw, REASON_DEP);
    }

    #[test]
    fn missing_file_defaults_to_explicit() {
        let tmp = TempDir::new().expect("tmp");
        let local_db = tmp.path();

        assert_eq!(InstallReason::read_optional(local_db, "legacy"), None);
        assert_eq!(
            InstallReason::read(local_db, "legacy"),
            InstallReason::Explicit
        );
    }

    #[test]
    fn mark_packages_changes_real_entries_and_rejects_unknown_names() {
        let tmp = TempDir::new().expect("tmp");
        let local_db = tmp.path().join("local");

        for name in ["alpha", "beta"] {
            let dir = local_db.join(name);
            fs::create_dir_all(&dir).expect("pkg dir");
            fs::write(dir.join(crate::local_db::VERSION_FILE), "1.0-1").expect("version");
        }

        let count = mark_packages(
            &local_db,
            &["alpha".to_string(), "beta".to_string()],
            InstallReason::Dep,
        )
        .expect("mark deps");
        assert_eq!(count, 2);
        assert_eq!(InstallReason::read(&local_db, "alpha"), InstallReason::Dep);
        assert_eq!(InstallReason::read(&local_db, "beta"), InstallReason::Dep);

        mark_packages(&local_db, &["beta".to_string()], InstallReason::Explicit)
            .expect("mark explicit");
        assert_eq!(
            InstallReason::read(&local_db, "beta"),
            InstallReason::Explicit
        );

        let err = mark_packages(
            &local_db,
            &["alpha".to_string(), "missing".to_string()],
            InstallReason::Explicit,
        )
        .expect_err("unknown package must fail");
        assert!(err.to_string().contains("missing"));
        assert_eq!(
            InstallReason::read(&local_db, "alpha"),
            InstallReason::Dep,
            "validation must happen before writing"
        );
    }

    #[test]
    fn retain_by_reason_filters_and_treats_missing_as_explicit() {
        let tmp = TempDir::new().expect("tmp");
        let local_db = tmp.path();

        InstallReason::Explicit
            .write(local_db, "alpha")
            .expect("write alpha");
        InstallReason::Dep
            .write(local_db, "beta")
            .expect("write beta");

        let packages = || {
            vec![
                ("alpha".to_string(), "1.0-1".to_string()),
                ("beta".to_string(), "1.0-1".to_string()),
                ("legacy".to_string(), "1.0-1".to_string()),
            ]
        };

        let mut explicit = packages();
        retain_by_reason(local_db, &mut explicit, InstallReason::Explicit);
        let names: Vec<&str> = explicit.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(names, vec!["alpha", "legacy"]);

        let mut deps = packages();
        retain_by_reason(local_db, &mut deps, InstallReason::Dep);
        let names: Vec<&str> = deps.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(names, vec!["beta"]);
    }
}
