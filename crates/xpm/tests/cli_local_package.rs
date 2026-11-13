//! Local `.pkg.tar.zst` installation without pacman.
//!
//! The installer contract is `xpm install <local-package>`; this test runs the
//! real binary with an empty PATH so any accidental `pacman`/`pacman-key` or
//! helper lookup would fail immediately.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};

struct Fixture {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    db_path: PathBuf,
    cache_dir: PathBuf,
    home: PathBuf,
    empty_path: PathBuf,
    hooks_dir: PathBuf,
    config: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let tmp = tempfile::TempDir::new().expect("tmp");
        let root = tmp.path().join("root");
        let db_path = root.join("var/lib/xpm");
        let cache_dir = root.join("var/cache/xpm/pkg");
        let home = tmp.path().join("home");
        let empty_path = tmp.path().join("empty-path");
        let hooks_dir = tmp.path().join("alpm-hooks");
        for dir in [&db_path, &cache_dir, &home, &empty_path, &hooks_dir] {
            fs::create_dir_all(dir).expect("dir");
        }

        let config = tmp.path().join("xpm.conf");
        fs::write(
            &config,
            format!(
                "[options]\nroot_dir = \"{}\"\ndb_path = \"{}\"\ncache_dir = \"{}\"\n\
                 sig_level = \"never\"\nmirror_pacman = false\nalpm_hooks = false\n",
                root.display(),
                db_path.display(),
                cache_dir.display()
            ),
        )
        .expect("config");

        Self {
            _tmp: tmp,
            root,
            db_path,
            cache_dir,
            home,
            empty_path,
            hooks_dir,
            config,
        }
    }

    fn package(&self, name: &str, version: &str) -> PathBuf {
        let path = self
            .cache_dir
            .join(format!("{name}-{version}-x86_64.pkg.tar.zst"));
        let pkginfo = format!(
            "pkgname = {name}\npkgver = {version}\npkgdesc = local test\narch = x86_64\nsize = 16\n"
        );
        let mtree = "#mtree\n./usr type=dir mode=0755 uid=0 gid=0\n./usr/bin type=dir mode=0755 uid=0 gid=0\n";

        let mut raw_tar = Vec::new();
        {
            let mut builder = tar::Builder::new(&mut raw_tar);
            for (entry, data) in [(".PKGINFO", pkginfo.as_str()), (".MTREE", mtree)] {
                let bytes = data.as_bytes();
                let mut header = tar::Header::new_gnu();
                header.set_path(entry).unwrap();
                header.set_size(bytes.len() as u64);
                header.set_mode(0o644);
                header.set_cksum();
                builder.append(&header, bytes).unwrap();
            }
            let script = format!("#!/bin/sh\necho {name}\n");
            let mut header = tar::Header::new_gnu();
            header.set_path(format!("usr/bin/{name}")).unwrap();
            header.set_size(script.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            builder.append(&header, script.as_bytes()).unwrap();
            builder.finish().unwrap();
        }

        let compressed = zstd::encode_all(&raw_tar[..], 1).expect("compress");
        fs::write(&path, compressed).expect("write package");
        path
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_xpm"))
            .arg("--no-confirm")
            .arg("-c")
            .arg(&self.config)
            .args(args)
            .env("HOME", &self.home)
            .env("PATH", &self.empty_path)
            .env("XPM_ALPM_HOOKS_DIRS", &self.hooks_dir)
            .output()
            .expect("run xpm")
    }

    fn local_db_version(&self, name: &str) -> String {
        fs::read_to_string(self.db_path.join("local").join(name).join("version"))
            .expect("local db version")
            .trim()
            .to_string()
    }
}

#[test]
fn installs_local_pkg_tar_zst_without_pacman() {
    let fixture = Fixture::new();
    let package = fixture.package("hello", "1.0-1");

    let output = fixture.run(&["install", package.to_str().unwrap()]);
    assert!(
        output.status.success(),
        "local install failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    assert_eq!(
        fs::read_to_string(fixture.root.join("usr/bin/hello")).expect("installed file"),
        "#!/bin/sh\necho hello\n"
    );
    assert_eq!(fixture.local_db_version("hello"), "1.0-1");
    assert!(
        !fixture.root.join("var/lib/pacman").exists(),
        "local install must not create a pacman database"
    );
}

#[test]
fn pacman_u_alias_installs_local_pkg_tar_zst() {
    let fixture = Fixture::new();
    let package = fixture.package("hello-compat", "2.0-1");

    let output = fixture.run(&["-U", package.to_str().unwrap()]);
    assert!(
        output.status.success(),
        "local -U install failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    assert!(fixture.root.join("usr/bin/hello-compat").is_file());
    assert_eq!(fixture.local_db_version("hello-compat"), "2.0-1");
}
