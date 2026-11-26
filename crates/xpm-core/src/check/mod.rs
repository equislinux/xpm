//! Installed-file verification for `xpm check` (pacman `-Qk` / `-Qkk`).
//!
//! The fast pass verifies that every path in the package's `files` manifest
//! still exists with the recorded file/directory type. The full pass also
//! parses the stored `.MTREE` copy and verifies regular-file SHA-256 digests,
//! symlink targets, and basic mode/type information. Configuration files
//! recorded in the local-database `backup` list are allowed to differ.

use std::collections::HashSet;
use std::fs::{self, File};
use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::error::{XpmError, XpmResult};
use crate::local_db;
use crate::package::mtree::parse_mtree;
use crate::package::types::{MtreeEntry, MtreeFileType};

/// Package metadata entries that must never be checked as package payload.
const METADATA_PATHS: &[&str] = &[".PKGINFO", ".BUILDINFO", ".MTREE", ".INSTALL"];

mod manifest;
mod mtree;
mod package;

pub use manifest::{repair_package_manifest, ManifestRepair};
pub use package::{check_package, PackageCheck};

use manifest::check_manifest;
use mtree::check_mtree;

fn normalize_entry(raw: &str) -> Option<String> {
    let mut parts = Vec::new();
    for component in Path::new(raw).components() {
        match component {
            std::path::Component::Normal(part) => parts.push(part.to_string_lossy().into_owned()),
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => return None,
            std::path::Component::RootDir | std::path::Component::Prefix(_) => {}
        }
    }

    if parts.is_empty() {
        None
    } else {
        Some(parts.join("/"))
    }
}

#[cfg(test)]
mod tests;
