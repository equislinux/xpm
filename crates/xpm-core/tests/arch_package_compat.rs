//! Compatibility with a **real** Arch Linux package (roadmap #21): the fixture
//! is built by the system's own `makepkg`, not by xpm's test helpers, and then
//! parsed with xpm's readers.
//!
//! The test is skipped when `makepkg`/`fakeroot` are unavailable, so the suite
//! stays portable (CI containers without `base-devel`).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use xpm_core::package::read_metadata;

fn tool_available(tool: &str) -> bool {
    Command::new(tool)
        .arg("--version")
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false)
}

/// Runs `makepkg` in `dir` and returns the produced package path.
fn run_makepkg(dir: &Path) -> PathBuf {
    let status = Command::new("makepkg")
        .args(["--nodeps", "--noconfirm", "--force"])
        .current_dir(dir)
        .status()
        .expect("run makepkg");
    assert!(status.success(), "makepkg failed");

    let package = fs::read_dir(dir)
        .expect("read makepkg output")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .find(|path| {
            path.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|name| name.ends_with(".pkg.tar.zst"))
        });
    package.expect("makepkg produced a .pkg.tar.zst")
}

#[test]
fn parses_package_built_by_makepkg() {
    if !tool_available("makepkg") || !tool_available("fakeroot") {
        eprintln!("skipping: makepkg/fakeroot not available");
        return;
    }

    let tmp = tempfile::TempDir::new().expect("tmp");
    let pkgbuild = r#"
pkgname=hello-xpm-compat
pkgver=1.2.3
pkgrel=2
pkgdesc="Compatibility fixture built by makepkg"
arch=('any')
license=('MIT')
backup=('etc/hello-xpm-compat.conf')

package() {
  install -Dm755 /dev/stdin "$pkgdir/usr/bin/hello-xpm-compat" <<'EOF'
#!/bin/sh
echo hello from xpm compatibility fixture
EOF
  install -Dm644 /dev/stdin "$pkgdir/etc/hello-xpm-compat.conf" <<'EOF'
# fixture configuration
key = value
EOF
}
"#;
    fs::write(tmp.path().join("PKGBUILD"), pkgbuild).expect("write PKGBUILD");

    let package = run_makepkg(tmp.path());
    let metadata = read_metadata(&package).expect("xpm must read a real makepkg package");

    assert_eq!(metadata.meta.name, "hello-xpm-compat");
    assert_eq!(metadata.meta.version, "1.2.3");
    assert_eq!(metadata.meta.release, "2");
    assert_eq!(metadata.meta.full_version(), "1.2.3-2");
    assert_eq!(metadata.meta.arch, vec!["any".to_string()]);
    assert_eq!(metadata.meta.license, vec!["MIT".to_string()]);
    // The `backup` declaration drives .pacnew/.pacsave handling.
    assert_eq!(
        metadata.meta.backup,
        vec!["etc/hello-xpm-compat.conf".to_string()]
    );
    // .MTREE must parse and list both payload files with hashes.
    let hashes = xpm_core::local_db::mtree_hash_map(&metadata.mtree);
    assert_eq!(hashes.len(), 2, "expected both files in .MTREE: {hashes:?}");
    assert!(hashes.contains_key("usr/bin/hello-xpm-compat"));
    assert!(hashes.contains_key("etc/hello-xpm-compat.conf"));
}

#[test]
fn file_listing_of_real_package_matches_mtree() {
    if !tool_available("makepkg") || !tool_available("fakeroot") {
        eprintln!("skipping: makepkg/fakeroot not available");
        return;
    }

    let tmp = tempfile::TempDir::new().expect("tmp");
    fs::write(
        tmp.path().join("PKGBUILD"),
        "pkgname=listing-compat\npkgver=1.0\npkgrel=1\npkgdesc=\"listing\"\narch=('any')\nlicense=('MIT')\npackage() {\n  mkdir -p \"$pkgdir/usr/share/listing-compat\"\n  echo data > \"$pkgdir/usr/share/listing-compat/data.txt\"\n}\n",
    )
    .expect("write PKGBUILD");

    let package = run_makepkg(tmp.path());
    let metadata = read_metadata(&package).expect("read package");
    let paths = xpm_core::local_db::mtree_paths(&metadata.mtree);

    assert!(paths
        .iter()
        .any(|p| p == "usr/share/listing-compat/data.txt"));
    // Directories carry a trailing slash in the manifest contract.
    assert!(paths.iter().any(|p| p == "usr/share/listing-compat/"));
}
