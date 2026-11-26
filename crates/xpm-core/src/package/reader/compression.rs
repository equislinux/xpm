//! Compression detection and archive opening.

use super::*;

// ── Compression detection ─────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Compression {
    Zstd,
    Gzip,
    Xz,
    None,
}

pub(super) fn detect_compression(path: &Path) -> Result<Compression, XpmError> {
    let mut file = File::open(path)?;
    let mut magic = [0u8; 6];
    let n = file.read(&mut magic)?;

    if n >= 4 && magic[..4] == [0x28, 0xB5, 0x2F, 0xFD] {
        return Ok(Compression::Zstd);
    }
    if n >= 2 && magic[..2] == [0x1F, 0x8B] {
        return Ok(Compression::Gzip);
    }
    if n >= 6 && magic[..6] == [0xFD, 0x37, 0x7A, 0x58, 0x5A, 0x00] {
        return Ok(Compression::Xz);
    }

    Ok(Compression::None)
}

pub(super) fn open_archive(path: &Path) -> Result<tar::Archive<Box<dyn Read>>, XpmError> {
    let file = File::open(path)?;
    let buf = BufReader::new(file);

    let reader: Box<dyn Read> = match detect_compression(path)? {
        Compression::Zstd => Box::new(zstd::Decoder::new(buf)?),
        Compression::Gzip => Box::new(flate2::read::GzDecoder::new(buf)),
        Compression::Xz => Box::new(xz2::read::XzDecoder::new(buf)),
        Compression::None => Box::new(buf),
    };

    Ok(tar::Archive::new(reader))
}
