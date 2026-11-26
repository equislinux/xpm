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
