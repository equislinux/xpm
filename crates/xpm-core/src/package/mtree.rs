//! Parser for `.MTREE` manifest files.
//!
//! Format:
//! ```text
//! #mtree
//! ./path/to/dir type=dir mode=0755 uid=0 gid=0
//! ./path/to/file type=file mode=0644 size=1234 sha256digest=abcdef... uid=0 gid=0
//! ./path/to/link type=link link=target uid=0 gid=0
//! ```
//!
//! Real Arch packages built by `makepkg` ship the manifest gzip-compressed
//! (`pacman` grew this in 5.2) while xpkg writes it as plain text; both are
//! accepted here.

use std::io::Read;
use std::path::PathBuf;

use crate::error::XpmError;
use crate::package::types::{MtreeEntry, MtreeFileType};

// ── Public API ────────────────────────────────────────────────

/// Parse the raw bytes of an `.MTREE` file into a list of [`MtreeEntry`].
///
/// Transparently decompresses the gzip-wrapped variant used by makepkg.
pub fn parse_mtree(data: &[u8]) -> Result<Vec<MtreeEntry>, XpmError> {
    let decoded = decode_mtree(data)?;
    let text = std::str::from_utf8(&decoded)
        .map_err(|e| XpmError::Package(format!("invalid UTF-8 in .MTREE: {e}")))?;

    let mut entries = Vec::new();

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let entry = parse_mtree_line(line)?;
        entries.push(entry);
    }

    Ok(entries)
}

/// Returns the plain-text manifest, decompressing gzip when needed.
fn decode_mtree(data: &[u8]) -> Result<Vec<u8>, XpmError> {
    if !data.starts_with(&[0x1F, 0x8B]) {
        return Ok(data.to_vec());
    }

    let mut decoder = flate2::read::GzDecoder::new(data);
    let mut decoded = Vec::new();
    decoder
        .read_to_end(&mut decoded)
        .map_err(|e| XpmError::Package(format!("failed to decompress .MTREE: {e}")))?;
    Ok(decoded)
}

// ── Line parser ───────────────────────────────────────────────

fn parse_mtree_line(line: &str) -> Result<MtreeEntry, XpmError> {
    let mut parts = line.split_whitespace();

    let path_str = parts
        .next()
        .ok_or_else(|| XpmError::Package("empty .MTREE line".into()))?;
    let path = PathBuf::from(path_str);

    let mut file_type = MtreeFileType::File;
    let mut mode: u32 = 0o644;
    let mut uid: u32 = 0;
    let mut gid: u32 = 0;
    let mut size: u64 = 0;
    let mut sha256: Option<String> = None;
    let mut link_target: Option<String> = None;

    for attr in parts {
        let (key, value) = match attr.split_once('=') {
            Some(kv) => kv,
            None => continue,
        };

        match key {
            "type" => {
                file_type = match value {
                    "dir" => MtreeFileType::Dir,
                    "file" => MtreeFileType::File,
                    "link" => MtreeFileType::Link,
                    other => {
                        return Err(XpmError::Package(format!("unknown .MTREE type: {other}")));
                    }
                };
            }
            "mode" => {
                mode = u32::from_str_radix(value, 8).unwrap_or(0o644);
            }
            "uid" => uid = value.parse().unwrap_or(0),
            "gid" => gid = value.parse().unwrap_or(0),
            "size" => size = value.parse().unwrap_or(0),
            "sha256digest" => sha256 = Some(value.to_string()),
            "link" => link_target = Some(value.to_string()),
            _ => {} // Forward-compatible: ignore unknown attributes.
        }
    }

    Ok(MtreeEntry {
        path,
        file_type,
        mode,
        uid,
        gid,
        size,
        sha256,
        link_target,
    })
}

// ── Tests ─────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_directory_entry() {
        let data = "#mtree\n./usr/bin type=dir mode=0755 uid=0 gid=0\n";
        let entries = parse_mtree(data.as_bytes()).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, PathBuf::from("./usr/bin"));
        assert_eq!(entries[0].file_type, MtreeFileType::Dir);
        assert_eq!(entries[0].mode, 0o755);
    }

    #[test]
    fn parse_file_entry_with_hash() {
        let hash = "e3b0c44298fc1c149afbf4c8996fb924\
                     27ae41e4649b934ca495991b7852b855";
        let line = format!(
            "./usr/bin/hello type=file mode=0755 size=4096 sha256digest={hash} uid=0 gid=0"
        );
        let data = format!("#mtree\n{line}\n");
        let entries = parse_mtree(data.as_bytes()).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].file_type, MtreeFileType::File);
        assert_eq!(entries[0].size, 4096);
        assert_eq!(entries[0].sha256.as_deref(), Some(hash));
        assert_eq!(entries[0].mode, 0o755);
    }

    #[test]
    fn parse_symlink_entry() {
        let data = "#mtree\n./usr/lib/libfoo.so type=link link=libfoo.so.1 uid=0 gid=0\n";
        let entries = parse_mtree(data.as_bytes()).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].file_type, MtreeFileType::Link);
        assert_eq!(entries[0].link_target.as_deref(), Some("libfoo.so.1"));
    }

    #[test]
    fn parse_multiple_entries() {
        let data = "#mtree\n\
            ./usr type=dir mode=0755 uid=0 gid=0\n\
            ./usr/bin type=dir mode=0755 uid=0 gid=0\n\
            ./usr/bin/hello type=file mode=0755 size=100 uid=0 gid=0\n";
        let entries = parse_mtree(data.as_bytes()).unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].file_type, MtreeFileType::Dir);
        assert_eq!(entries[2].file_type, MtreeFileType::File);
    }

    #[test]
    fn parse_skips_comments_and_blank_lines() {
        let data = "# header\n#mtree\n\n./f type=file mode=0644 uid=0 gid=0\n\n";
        let entries = parse_mtree(data.as_bytes()).unwrap();
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn parse_unknown_type_returns_error() {
        let data = "./f type=socket uid=0 gid=0\n";
        let result = parse_mtree(data.as_bytes());
        assert!(result.is_err());
    }

    #[test]
    fn parse_empty_mtree() {
        let entries = parse_mtree(b"#mtree\n").unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn parse_ignores_unknown_attributes() {
        let data = "./f type=file mode=0644 future_attr=42 uid=0 gid=0\n";
        let entries = parse_mtree(data.as_bytes()).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].mode, 0o644);
    }

    #[test]
    fn parses_gzip_compressed_mtree_as_written_by_makepkg() {
        use std::io::Write;

        let plain =
            b"#mtree\n./usr/bin/hello type=file mode=0755 size=1 sha256digest=aa uid=0 gid=0\n";
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(plain).expect("gzip write");
        let gz = encoder.finish().expect("gzip finish");

        let entries = parse_mtree(&gz).expect("gzip mtree must parse");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].sha256.as_deref(), Some("aa"));
    }

    #[test]
    fn corrupt_gzip_mtree_reports_error() {
        let result = parse_mtree(&[0x1F, 0x8B, 0x00, 0x01]);
        assert!(result.is_err());
    }
}
