//! Filesystem/archive helpers shared by hooks.

use super::*;

// ── Extraction helpers ──────────────────────────────────────────────────────

/// Opens a package archive, detecting zstd/gzip/xz/plain-tar by magic bytes.
pub(super) fn open_package_archive(pkg_file: &Path) -> XpmResult<tar::Archive<Box<dyn Read>>> {
    // Sniff the magic bytes on a throwaway handle so the decoder sees the
    // stream from the beginning.
    let mut magic = [0u8; 6];
    let n = {
        let mut probe = std::io::BufReader::new(fs::File::open(pkg_file)?);
        probe.read(&mut magic)?
    };

    let reader = std::io::BufReader::new(fs::File::open(pkg_file)?);
    let reader: Box<dyn Read> = if n >= 4 && magic[..4] == [0x28, 0xB5, 0x2F, 0xFD] {
        Box::new(
            Decoder::new(reader)
                .map_err(|e| XpmError::Package(format!("failed to decode zstd: {e}")))?,
        )
    } else if n >= 2 && magic[..2] == [0x1F, 0x8B] {
        Box::new(GzDecoder::new(reader))
    } else if n >= 6 && magic[..6] == [0xFD, 0x37, 0x7A, 0x58, 0x5A, 0x00] {
        Box::new(XzDecoder::new(reader))
    } else {
        Box::new(reader)
    };

    Ok(tar::Archive::new(reader))
}

pub(super) fn ensure_no_symlink_ancestors(root: &Path, target: &Path) -> XpmResult<()> {
    let relative = target.strip_prefix(root).map_err(|_| {
        XpmError::Package(format!(
            "target {} is outside the installation root {}",
            target.display(),
            root.display()
        ))
    })?;
    let components: Vec<_> = relative.components().collect();
    let mut current = root.to_path_buf();

    for component in components.iter().take(components.len().saturating_sub(1)) {
        current.push(component.as_os_str());
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(XpmError::Package(format!(
                    "refusing to extract through symlink {}",
                    current.display()
                )));
            }
            Ok(metadata) if !metadata.is_dir() => {
                return Err(XpmError::Package(format!(
                    "path component is not a directory: {}",
                    current.display()
                )));
            }
            Ok(_) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => break,
            Err(err) => return Err(err.into()),
        }
    }

    Ok(())
}

pub(super) struct EntryAttrs {
    pub(super) mode: u32,
    pub(super) uid: u32,
    pub(super) gid: u32,
    pub(super) mtime: i64,
    pub(super) xattrs: Vec<(Vec<u8>, Vec<u8>)>,
}

pub(super) fn read_entry_attrs<R: Read>(entry: &mut tar::Entry<R>) -> XpmResult<EntryAttrs> {
    let header = entry.header();
    let mode = header.mode().unwrap_or(0o644) & 0o7777;
    let uid = header.uid().unwrap_or(0) as u32;
    let gid = header.gid().unwrap_or(0) as u32;
    let mtime = header.mtime().unwrap_or(0) as i64;
    let mut xattrs = Vec::new();

    if let Some(extensions) = entry.pax_extensions()? {
        for extension in extensions {
            let extension = extension?;
            let key = extension.key_bytes();
            if let Some(name) = key.strip_prefix(b"SCHILY.xattr.") {
                if !name.is_empty() {
                    xattrs.push((name.to_vec(), extension.value_bytes().to_vec()));
                }
            }
        }
    }

    Ok(EntryAttrs {
        mode,
        uid,
        gid,
        mtime,
        xattrs,
    })
}

pub(super) fn apply_entry_attrs(path: &Path, attrs: &EntryAttrs, symlink: bool) -> XpmResult<()> {
    use std::os::unix::fs::PermissionsExt;

    let path_c = std::ffi::CString::new(path.as_os_str().as_bytes())
        .map_err(|_| XpmError::Package(format!("path contains NUL: {}", path.display())))?;

    // chown clears setuid/setgid bits, so ownership must be applied before the
    // final mode. This is why sudo and other setuid binaries lost their bits.
    if unsafe { libc::geteuid() } == 0 {
        let rc = unsafe { libc::lchown(path_c.as_ptr(), attrs.uid, attrs.gid) };
        if rc != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
    }

    if !symlink {
        fs::set_permissions(path, fs::Permissions::from_mode(attrs.mode))?;
    }

    let times = [
        libc::timespec {
            tv_sec: attrs.mtime as libc::time_t,
            tv_nsec: 0,
        },
        libc::timespec {
            tv_sec: attrs.mtime as libc::time_t,
            tv_nsec: 0,
        },
    ];
    let flags = if symlink {
        libc::AT_SYMLINK_NOFOLLOW
    } else {
        0
    };
    let rc = unsafe { libc::utimensat(libc::AT_FDCWD, path_c.as_ptr(), times.as_ptr(), flags) };
    if rc != 0 {
        return Err(std::io::Error::last_os_error().into());
    }

    for (name, value) in &attrs.xattrs {
        let name_c = std::ffi::CString::new(name.as_slice()).map_err(|_| {
            XpmError::Package(format!("xattr name contains NUL on {}", path.display()))
        })?;
        let rc = unsafe {
            libc::lsetxattr(
                path_c.as_ptr(),
                name_c.as_ptr(),
                value.as_ptr().cast(),
                value.len(),
                0,
            )
        };
        if rc != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
    }

    Ok(())
}

/// Normalizes an archive path to the manifest form (`etc/foo.conf`).
/// A trailing `/` (directory entries) is preserved on purpose.
pub(super) fn normalize_archive_path(path: &Path) -> String {
    let raw = path.to_string_lossy();
    let mut normalized = raw.as_ref();
    while let Some(stripped) = normalized.strip_prefix("./") {
        normalized = stripped;
    }
    normalized.to_string()
}

/// Whether the entry is package metadata rather than payload.
///
/// Only top-level metadata files are special; `usr/share/foo.PKGINFO` is a
/// normal package file.
pub(super) fn is_metadata_path(normalized: &str) -> bool {
    matches!(
        normalized,
        ".PKGINFO" | ".BUILDINFO" | ".MTREE" | ".INSTALL"
    )
}

/// SHA-256 of a regular file; `None` when it does not exist.
pub(super) fn sha256_file(path: &Path) -> XpmResult<Option<String>> {
    let mut file = match fs::File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher)?;
    let digest = hasher.finalize();
    Ok(Some(
        digest
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>(),
    ))
}
