//! ALPM-compatible version parsing and comparison.
//!
//! Implements the `vercmp` algorithm used by pacman/libalpm to compare package
//! versions. The version format is `[epoch:]pkgver[-pkgrel]`.
//!
//! Reference: <https://man.archlinux.org/man/vercmp.8>

use std::cmp::Ordering;
use std::fmt;

/// A parsed ALPM package version.
///
/// Format: `[epoch:]pkgver[-pkgrel]`
///
/// - `epoch` — override for version ordering (default: 0)
/// - `pkgver` — upstream version string
/// - `pkgrel` — Arch package release number
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Version {
    /// Epoch override (0 if not specified).
    pub epoch: u64,
    /// Upstream version string (e.g. "1.2.3").
    pub pkgver: String,
    /// Package release (e.g. "1"). Empty if not specified.
    pub pkgrel: String,
}

impl Version {
    /// Parse a version string in ALPM format.
    ///
    /// Accepted formats:
    /// - `"1.2.3"` → epoch=0, pkgver="1.2.3", pkgrel=""
    /// - `"1.2.3-1"` → epoch=0, pkgver="1.2.3", pkgrel="1"
    /// - `"2:1.2.3-1"` → epoch=2, pkgver="1.2.3", pkgrel="1"
    pub fn parse(s: &str) -> Self {
        let (epoch, rest) = match s.find(':') {
            Some(pos) => {
                let epoch = s[..pos].parse::<u64>().unwrap_or(0);
                (epoch, &s[pos + 1..])
            }
            None => (0, s),
        };

        let (pkgver, pkgrel) = match rest.rfind('-') {
            Some(pos) => (rest[..pos].to_string(), rest[pos + 1..].to_string()),
            None => (rest.to_string(), String::new()),
        };

        Self {
            epoch,
            pkgver,
            pkgrel,
        }
    }

    /// Compare two version strings using the ALPM vercmp algorithm.
    pub fn cmp_versions(a: &str, b: &str) -> Ordering {
        let va = Self::parse(a);
        let vb = Self::parse(b);
        va.cmp(&vb)
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.epoch > 0 {
            write!(f, "{}:", self.epoch)?;
        }
        write!(f, "{}", self.pkgver)?;
        if !self.pkgrel.is_empty() {
            write!(f, "-{}", self.pkgrel)?;
        }
        Ok(())
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        // 1. Compare epoch first
        match self.epoch.cmp(&other.epoch) {
            Ordering::Equal => {}
            ord => return ord,
        }

        // 2. Compare pkgver
        match alpm_vercmp(&self.pkgver, &other.pkgver) {
            Ordering::Equal => {}
            ord => return ord,
        }

        // 3. Compare pkgrel. libalpm ignores pkgrel when either side omits
        // it: `vercmp 262.1-1 262.1` returns 0, so dependencies written as
        // `pkg=262.1` match `262.1-1`.
        if self.pkgrel.is_empty() || other.pkgrel.is_empty() {
            return Ordering::Equal;
        }

        alpm_vercmp(&self.pkgrel, &other.pkgrel)
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// ALPM version comparison algorithm (vercmp).
///
/// Compares two version segments character-by-character, splitting into
/// runs of digits and non-digits. Digit runs are compared numerically;
/// non-digit runs are compared lexicographically.
///
/// This mirrors the behavior of `alpm_pkg_vercmp` from libalpm.
fn alpm_vercmp(a: &str, b: &str) -> Ordering {
    let a_bytes = a.as_bytes();
    let b_bytes = b.as_bytes();
    let mut ai = 0;
    let mut bi = 0;

    // Skip leading identical bytes for performance
    while ai < a_bytes.len() && bi < b_bytes.len() && a_bytes[ai] == b_bytes[bi] {
        ai += 1;
        bi += 1;
    }

    // If both exhausted, they're equal
    if ai == a_bytes.len() && bi == b_bytes.len() {
        return Ordering::Equal;
    }

    // Walk back to the start of the current segment so we split on a boundary
    while ai > 0 && ai < a_bytes.len() && is_alnum(a_bytes[ai]) && is_alnum(a_bytes[ai - 1]) {
        ai -= 1;
        bi -= 1;
    }

    // Now compare segment by segment
    loop {
        // Skip separators (non-alphanumeric)
        while ai < a_bytes.len() && !is_alnum(a_bytes[ai]) {
            ai += 1;
        }
        while bi < b_bytes.len() && !is_alnum(b_bytes[bi]) {
            bi += 1;
        }

        // If either is exhausted
        if ai >= a_bytes.len() || bi >= b_bytes.len() {
            break;
        }

        let a_is_digit = a_bytes[ai].is_ascii_digit();
        let b_is_digit = b_bytes[bi].is_ascii_digit();

        // Extract segments
        let seg_a_start = ai;
        if a_is_digit {
            while ai < a_bytes.len() && a_bytes[ai].is_ascii_digit() {
                ai += 1;
            }
        } else {
            while ai < a_bytes.len() && a_bytes[ai].is_ascii_alphabetic() {
                ai += 1;
            }
        }

        let seg_b_start = bi;
        if b_is_digit {
            while bi < b_bytes.len() && b_bytes[bi].is_ascii_digit() {
                bi += 1;
            }
        } else {
            while bi < b_bytes.len() && b_bytes[bi].is_ascii_alphabetic() {
                bi += 1;
            }
        }

        let seg_a = &a_bytes[seg_a_start..ai];
        let seg_b = &b_bytes[seg_b_start..bi];

        // If segment types differ, numeric segments are always newer
        if a_is_digit && !b_is_digit {
            return Ordering::Greater;
        }
        if !a_is_digit && b_is_digit {
            return Ordering::Less;
        }

        if a_is_digit {
            // Numeric comparison: strip leading zeros, then compare by length, then lex
            let sa = strip_leading_zeros(seg_a);
            let sb = strip_leading_zeros(seg_b);

            match sa.len().cmp(&sb.len()) {
                Ordering::Equal => match sa.cmp(sb) {
                    Ordering::Equal => continue,
                    ord => return ord,
                },
                ord => return ord,
            }
        } else {
            // Lexicographic comparison for alpha segments
            match seg_a.cmp(seg_b) {
                Ordering::Equal => continue,
                ord => return ord,
            }
        }
    }

    // Whichever has content remaining is newer
    if ai < a_bytes.len() {
        // If remaining is alpha, it's a pre-release-like suffix — older
        if a_bytes[ai].is_ascii_alphabetic() {
            return Ordering::Less;
        }
        return Ordering::Greater;
    }
    if bi < b_bytes.len() {
        if b_bytes[bi].is_ascii_alphabetic() {
            return Ordering::Greater;
        }
        return Ordering::Less;
    }

    Ordering::Equal
}

#[inline]
fn is_alnum(b: u8) -> bool {
    b.is_ascii_alphanumeric()
}

#[inline]
fn strip_leading_zeros(bytes: &[u8]) -> &[u8] {
    let start = bytes.iter().position(|&b| b != b'0').unwrap_or(bytes.len());
    &bytes[start..]
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Version parsing ──────────────────────────────────────────────

    #[test]
    fn parse_simple() {
        let v = Version::parse("1.2.3");
        assert_eq!(v.epoch, 0);
        assert_eq!(v.pkgver, "1.2.3");
        assert_eq!(v.pkgrel, "");
    }

    #[test]
    fn parse_with_pkgrel() {
        let v = Version::parse("1.2.3-1");
        assert_eq!(v.epoch, 0);
        assert_eq!(v.pkgver, "1.2.3");
        assert_eq!(v.pkgrel, "1");
    }

    #[test]
    fn parse_with_epoch() {
        let v = Version::parse("2:1.2.3-1");
        assert_eq!(v.epoch, 2);
        assert_eq!(v.pkgver, "1.2.3");
        assert_eq!(v.pkgrel, "1");
    }

    #[test]
    fn parse_epoch_no_rel() {
        let v = Version::parse("1:5.0");
        assert_eq!(v.epoch, 1);
        assert_eq!(v.pkgver, "5.0");
        assert_eq!(v.pkgrel, "");
    }

    #[test]
    fn display_roundtrip() {
        let cases = ["1.2.3", "1.2.3-1", "2:1.2.3-1", "1:5.0"];
        for s in cases {
            assert_eq!(Version::parse(s).to_string(), s);
        }
    }

    // ── vercmp — same version ────────────────────────────────────────

    #[test]
    fn vercmp_equal() {
        assert_eq!(Version::cmp_versions("1.0", "1.0"), Ordering::Equal);
        assert_eq!(Version::cmp_versions("1.0-1", "1.0-1"), Ordering::Equal);
        assert_eq!(Version::cmp_versions("2:1.0-1", "2:1.0-1"), Ordering::Equal);
    }

    // ── vercmp — epoch dominance ─────────────────────────────────────

    #[test]
    fn vercmp_epoch() {
        assert_eq!(Version::cmp_versions("1:1.0", "2:1.0"), Ordering::Less);
        assert_eq!(Version::cmp_versions("2:1.0", "1:1.0"), Ordering::Greater);
        assert_eq!(Version::cmp_versions("1:1.0", "0:2.0"), Ordering::Greater);
    }

    // ── vercmp — numeric segments ────────────────────────────────────

    #[test]
    fn vercmp_numeric() {
        assert_eq!(Version::cmp_versions("1.0", "1.1"), Ordering::Less);
        assert_eq!(Version::cmp_versions("1.1", "1.0"), Ordering::Greater);
        assert_eq!(Version::cmp_versions("1.9", "1.10"), Ordering::Less);
        assert_eq!(Version::cmp_versions("1.10", "1.9"), Ordering::Greater);
    }

    // ── vercmp — alpha segments ──────────────────────────────────────

    #[test]
    fn vercmp_alpha() {
        assert_eq!(Version::cmp_versions("1.0a", "1.0b"), Ordering::Less);
        assert_eq!(Version::cmp_versions("1.0b", "1.0a"), Ordering::Greater);
    }

    // ── vercmp — numeric vs alpha ────────────────────────────────────

    #[test]
    fn vercmp_numeric_beats_alpha() {
        // Numeric segments are always "newer" than alpha segments
        assert_eq!(Version::cmp_versions("1.0.1", "1.0.a"), Ordering::Greater);
        assert_eq!(Version::cmp_versions("1.0.a", "1.0.1"), Ordering::Less);
    }

    // ── vercmp — pkgrel comparison ───────────────────────────────────

    #[test]
    fn vercmp_pkgrel() {
        assert_eq!(Version::cmp_versions("1.0-1", "1.0-2"), Ordering::Less);
        assert_eq!(Version::cmp_versions("1.0-2", "1.0-1"), Ordering::Greater);
    }

    #[test]
    fn vercmp_ignores_pkgrel_when_one_side_omits_it() {
        // pacman's `vercmp 262.1-1 262.1` prints 0.
        assert_eq!(Version::cmp_versions("262.1-1", "262.1"), Ordering::Equal);
        assert_eq!(Version::cmp_versions("262.1", "262.1-1"), Ordering::Equal);
        assert_eq!(Version::cmp_versions("5.3.20-1", "5.3.20"), Ordering::Equal);
    }

    // ── vercmp — real-world examples ─────────────────────────────────

    #[test]
    fn vercmp_real_world() {
        assert_eq!(Version::cmp_versions("6.2.9-1", "6.2.10-1"), Ordering::Less);
        assert_eq!(
            Version::cmp_versions("2:4.14.1-1", "2:4.14.2-1"),
            Ordering::Less
        );
        assert_eq!(
            Version::cmp_versions("1:2.3.4-1", "2.3.4-1"),
            Ordering::Greater
        );
    }

    // ── vercmp — leading zeros ───────────────────────────────────────

    #[test]
    fn vercmp_leading_zeros() {
        assert_eq!(Version::cmp_versions("1.01", "1.1"), Ordering::Equal);
        assert_eq!(Version::cmp_versions("1.001", "1.1"), Ordering::Equal);
    }

    // ── Ordering trait ───────────────────────────────────────────────

    #[test]
    fn version_ordering() {
        let mut versions: Vec<Version> = vec![
            Version::parse("1.0-1"),
            Version::parse("2:0.1-1"),
            Version::parse("1.1-1"),
            Version::parse("0.9-1"),
        ];
        versions.sort();
        let sorted: Vec<String> = versions.iter().map(|v| v.to_string()).collect();
        assert_eq!(sorted, vec!["0.9-1", "1.0-1", "1.1-1", "2:0.1-1"]);
    }
}
