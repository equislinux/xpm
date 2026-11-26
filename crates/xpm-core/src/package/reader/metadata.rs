//! Package metadata parsing.

use super::*;

// ── Metadata reader ───────────────────────────────────────────

/// Parsed metadata from a package archive.
///
/// A single pass over the archive also captures the file list, the raw
/// `.MTREE` payload and the `.INSTALL` scriptlet so later phases do not need
/// to decompress the package again just to inspect metadata.
#[derive(Debug, Clone)]
pub struct PackageMetadata {
    pub meta: PackageMeta,
    pub buildinfo: Option<BuildInfo>,
    pub mtree: Vec<MtreeEntry>,
    /// Raw `.MTREE` bytes (stored byte-for-byte in the local database).
    pub mtree_raw: Option<Vec<u8>>,
    /// Raw `.INSTALL` scriptlet, when present.
    pub install_script: Option<Vec<u8>>,
    /// Relative file paths shipped by the package (directories end with `/`).
    pub files: Vec<String>,
    /// MD5 of packaged configuration files (the value pacman stores in its
    /// `backup` file). Computed during the same metadata pass.
    pub backup_md5: HashMap<String, String>,
}

/// Read and parse all metadata from a `.xp` or `.pkg.tar.zst` package.
pub fn read_metadata(path: &Path) -> Result<PackageMetadata, XpmError> {
    let mut archive = open_archive(path)?;

    let mut pkginfo_data: Option<Vec<u8>> = None;
    let mut buildinfo_data: Option<Vec<u8>> = None;
    let mut mtree_data: Option<Vec<u8>> = None;
    let mut install_script: Option<Vec<u8>> = None;
    let mut files: Vec<String> = Vec::new();
    let mut parsed_meta: Option<PackageMeta> = None;
    let mut backup_md5: HashMap<String, String> = HashMap::new();

    for entry in archive.entries()? {
        let mut entry = entry?;
        let entry_path = entry.path()?.to_path_buf();
        let name = entry_path.to_string_lossy().to_string();
        let normalized = name.strip_prefix("./").unwrap_or(&name).to_string();

        match normalized.as_str() {
            ".PKGINFO" => {
                let mut buf = Vec::new();
                entry.read_to_end(&mut buf)?;
                let meta = pkginfo::parse_pkginfo(&buf)?;
                parsed_meta = Some(meta);
                pkginfo_data = Some(buf);
            }
            ".BUILDINFO" => {
                let mut buf = Vec::new();
                entry.read_to_end(&mut buf)?;
                buildinfo_data = Some(buf);
            }
            ".MTREE" => {
                let mut buf = Vec::new();
                entry.read_to_end(&mut buf)?;
                mtree_data = Some(buf);
            }
            ".INSTALL" => {
                let mut buf = Vec::new();
                entry.read_to_end(&mut buf)?;
                install_script = Some(buf);
            }
            _ => {
                if normalized.starts_with('.') || normalized.is_empty() {
                    continue;
                }
                if normalized.starts_with('/')
                    || normalized.split('/').any(|component| component == "..")
                {
                    return Err(XpmError::Package(format!(
                        "package contains an unsafe path: {normalized}"
                    )));
                }
                let mut path = normalized;
                let is_file = entry.header().entry_type().is_file();
                if entry.header().entry_type().is_dir() && !path.ends_with('/') {
                    path.push('/');
                }
                if is_file
                    && parsed_meta
                        .as_ref()
                        .is_some_and(|meta| meta.backup.iter().any(|item| item == &path))
                {
                    let mut bytes = Vec::new();
                    entry.read_to_end(&mut bytes)?;
                    backup_md5.insert(path.clone(), format!("{:x}", Md5::digest(&bytes)));
                }
                files.push(path);
            }
        }
    }
    files.sort();
    files.dedup();

    let pkginfo_bytes =
        pkginfo_data.ok_or_else(|| XpmError::Package("package missing .PKGINFO".into()))?;

    let meta = match parsed_meta {
        Some(meta) => meta,
        None => pkginfo::parse_pkginfo(&pkginfo_bytes)?,
    };

    let buildinfo = match buildinfo_data {
        Some(data) => Some(buildinfo::parse_buildinfo(&data)?),
        None => None,
    };

    let mtree_entries = match mtree_data {
        Some(ref data) => mtree::parse_mtree(data)?,
        None => Vec::new(),
    };

    Ok(PackageMetadata {
        meta,
        buildinfo,
        mtree: mtree_entries,
        mtree_raw: mtree_data,
        install_script,
        files,
        backup_md5,
    })
}
