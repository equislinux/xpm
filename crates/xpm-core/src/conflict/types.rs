//! Public conflict types.

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
