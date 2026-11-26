//! Archive reader for `.xp` and `.pkg.tar.zst` packages.
//!
//! Supports zstd (default), gzip, and xz compression. Auto-detects
//! compression from magic bytes.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};

use md5::{Digest, Md5};

use crate::error::XpmError;
use crate::package::buildinfo;
use crate::package::mtree;
use crate::package::pkginfo;
use crate::package::types::{BuildInfo, MtreeEntry, PackageFile, PackageMeta};

mod compression;
mod extract;
mod files;
mod metadata;
mod raw;

pub use extract::extract_to;
pub use files::list_files;
pub use metadata::{read_metadata, PackageMetadata};
pub use raw::read_raw_entry;

use compression::open_archive;
#[cfg(test)]
use compression::{detect_compression, Compression};

#[cfg(test)]
mod tests;
