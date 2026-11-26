//! ALPM archive/section parsing helpers.

use super::*;

#[derive(Debug, Default)]
pub(super) struct ArchivePackage {
    pub(super) desc: Option<String>,
    pub(super) depends: Option<String>,
    pub(super) files: Option<String>,
}

pub(super) fn read_repo_archive(bytes: &[u8]) -> XpmResult<HashMap<String, ArchivePackage>> {
    let mut archive = open_archive(bytes)?;
    let mut packages = HashMap::<String, ArchivePackage>::new();

    for entry_result in archive.entries()? {
        let mut entry = entry_result.map_err(|e| XpmError::Database(e.to_string()))?;
        if !entry.header().entry_type().is_file() {
            continue;
        }

        let path = entry
            .path()
            .map_err(|e| XpmError::Database(e.to_string()))?
            .into_owned();
        let path_str = path.to_string_lossy();
        let mut parts = path_str.split('/').filter(|p| !p.is_empty());
        let Some(pkg_id) = parts.next() else {
            continue;
        };
        let Some(file_name) = parts.next() else {
            continue;
        };

        let mut content = String::new();
        entry.read_to_string(&mut content)?;

        let pkg = packages.entry(pkg_id.to_string()).or_default();
        match file_name {
            "desc" => pkg.desc = Some(content),
            "depends" => pkg.depends = Some(content),
            "files" => pkg.files = Some(content),
            _ => {}
        }
    }

    Ok(packages)
}

pub(super) fn open_archive(bytes: &[u8]) -> XpmResult<tar::Archive<Box<dyn Read>>> {
    let reader: Box<dyn Read> = if is_gzip(bytes) {
        Box::new(GzDecoder::new(Cursor::new(bytes.to_vec())))
    } else if is_zstd(bytes) {
        // xpkg repo-add writes zstd-compressed databases by default; ALPM-style
        // `.db` files can also arrive gzip or xz compressed.
        Box::new(
            zstd::Decoder::new(Cursor::new(bytes.to_vec()))
                .map_err(|e| XpmError::Database(format!("failed to decode zstd database: {e}")))?,
        )
    } else if is_xz(bytes) {
        Box::new(XzDecoder::new(Cursor::new(bytes.to_vec())))
    } else {
        Box::new(Cursor::new(bytes.to_vec()))
    };

    Ok(tar::Archive::new(reader))
}

pub(super) fn is_gzip(bytes: &[u8]) -> bool {
    bytes.len() >= 2 && bytes[0] == 0x1f && bytes[1] == 0x8b
}

pub(super) fn is_zstd(bytes: &[u8]) -> bool {
    bytes.len() >= 4 && bytes[..4] == [0x28, 0xb5, 0x2f, 0xfd]
}

pub(super) fn is_xz(bytes: &[u8]) -> bool {
    bytes.len() >= 6 && bytes[..6] == [0xfd, 0x37, 0x7a, 0x58, 0x5a, 0x00]
}

pub(super) fn merge_sections(
    into: &mut HashMap<String, Vec<String>>,
    from: HashMap<String, Vec<String>>,
) {
    for (k, mut v) in from {
        into.entry(k).or_default().append(&mut v);
    }
}

pub(super) fn repo_entry_from_sections(
    pkg_id: &str,
    sections: &HashMap<String, Vec<String>>,
) -> XpmResult<RepoEntry> {
    let name = first_value(sections, "NAME").ok_or_else(|| {
        XpmError::Database(format!("missing %NAME% in repository entry '{pkg_id}'"))
    })?;
    let version = first_value(sections, "VERSION").ok_or_else(|| {
        XpmError::Database(format!("missing %VERSION% in repository entry '{pkg_id}'"))
    })?;

    Ok(RepoEntry {
        name,
        version,
        filename: first_value(sections, "FILENAME"),
        sha256sum: first_value(sections, "SHA256SUM"),
        url: first_value(sections, "URL"),
        description: first_value(sections, "DESC"),
        arch: first_value(sections, "ARCH"),
        depends: values(sections, "DEPENDS"),
        opt_depends: values(sections, "OPTDEPENDS"),
        provides: values(sections, "PROVIDES"),
        conflicts: values(sections, "CONFLICTS"),
        replaces: values(sections, "REPLACES"),
        groups: values(sections, "GROUPS"),
        files: Vec::new(),
    })
}

pub(crate) fn first_value(sections: &HashMap<String, Vec<String>>, key: &str) -> Option<String> {
    sections.get(key).and_then(|values| values.first()).cloned()
}

pub(super) fn values(sections: &HashMap<String, Vec<String>>, key: &str) -> Vec<String> {
    sections.get(key).cloned().unwrap_or_default()
}

pub(crate) fn parse_alpm_sections(input: &str) -> HashMap<String, Vec<String>> {
    let mut map = HashMap::<String, Vec<String>>::new();
    let mut current: Option<String> = None;

    for raw in input.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }

        if line.starts_with('%') && line.ends_with('%') && line.len() > 2 {
            let key = line.trim_matches('%').to_ascii_uppercase();
            current = Some(key);
            continue;
        }

        if let Some(section) = &current {
            map.entry(section.clone())
                .or_default()
                .push(line.to_string());
        }
    }

    map
}
