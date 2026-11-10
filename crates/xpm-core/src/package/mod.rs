//! Package format support — readers and parsers for `.xp` and `.pkg.tar.zst` archives.
//!
//! This module handles reading package archives, parsing embedded metadata
//! files (`.PKGINFO`, `.BUILDINFO`, `.MTREE`), and validating file integrity
//! after extraction.

pub mod buildinfo;
pub mod mtree;
pub mod pkginfo;
pub mod reader;
pub mod types;
pub mod validate;

pub use reader::{list_files, read_metadata, read_raw_entry, PackageMetadata};
pub use types::{BuildInfo, MtreeEntry, MtreeFileType, PackageMeta};
pub use validate::validate_integrity;
