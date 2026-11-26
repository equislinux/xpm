//! End-to-end CLI tests for the package-level recovery path:
//! `xpm history` (generation linking), `xpm diff <generation>` and
//! `xpm rollback --last`, driven through the real binary against a temporary
//! root/database/cache (no root privileges, no network, no btrfs).

use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use xpm_core::journal::{Journal, JournalPackage};

/// Temporary installation root plus the paths the CLI needs.
struct Fixture {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    db_path: PathBuf,
    cache_dir: PathBuf,
    home: PathBuf,
    config: PathBuf,
    alpm_hooks_dir: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let tmp = tempfile::TempDir::new().expect("tmp");
        let root = tmp.path().join("root");
        let db_path = root.join("var/lib/xpm");
        let cache_dir = root.join("var/cache/xpm/pkg");
        let home = tmp.path().join("home");
        fs::create_dir_all(&db_path).expect("db dir");
        fs::create_dir_all(&cache_dir).expect("cache dir");
        fs::create_dir_all(&home).expect("home dir");

        let config = tmp.path().join("xpm.conf");
        fs::write(
            &config,
            format!(
                "[options]\nroot_dir = \"{}\"\ndb_path = \"{}\"\ncache_dir = \"{}\"\nsig_level = \"never\"\n",
                root.display(),
                db_path.display(),
                cache_dir.display()
            ),
        )
        .expect("write config");

        let alpm_hooks_dir = tmp.path().join("alpm-hooks");

        Self {
            _tmp: tmp,
            root,
            db_path,
            cache_dir,
            home,
            config,
            alpm_hooks_dir,
        }
    }

    /// Writes an executable shell script and an ALPM `.hook` that runs it.
    fn add_alpm_hook(&self, hook_name: &str, when: &str, marker: &Path, label: &str) {
        fs::create_dir_all(&self.alpm_hooks_dir).expect("hook dir");
        let script = self.alpm_hooks_dir.join(format!("{hook_name}.sh"));
        fs::write(
            &script,
            format!("#!/bin/sh\necho {label} >> '{}'\n", marker.display()),
        )
        .expect("hook script");
        let mut perms = fs::metadata(&script).expect("meta").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script, perms).expect("chmod");

        fs::write(
            self.alpm_hooks_dir.join(format!("{hook_name}.hook")),
            format!(
                "[Trigger]\nOperation = Upgrade\nType = Package\nTarget = *\n\n\
                 [Action]\nWhen = {when}\nExec = {}\n",
                script.display()
            ),
        )
        .expect("hook file");
    }

    fn append_config(&self, extra: &str) {
        let mut file = fs::OpenOptions::new()
            .append(true)
            .open(&self.config)
            .expect("open config");
        file.write_all(extra.as_bytes()).expect("append config");
    }

    fn journal_dir(&self) -> PathBuf {
        self.db_path.join("journal")
    }

    fn local_db(&self) -> PathBuf {
        self.db_path.join("local")
    }

    /// Creates `<local db>/<name>/version`.
    fn install_into_db(&self, name: &str, version: &str) {
        let dir = self.local_db().join(name);
        fs::create_dir_all(&dir).expect("pkg db dir");
        fs::write(dir.join("version"), version).expect("version file");
    }

    fn read_db_version(&self, name: &str) -> String {
        fs::read_to_string(self.local_db().join(name).join("version"))
            .expect("read version")
            .trim()
            .to_string()
    }

    /// Builds a minimal valid `.xp` package in the cache. `version` is the
    /// full version (`0.9-1`, version plus release).
    fn cache_package(&self, name: &str, version: &str) -> PathBuf {
        let (pkgver, release) = version.rsplit_once('-').unwrap_or((version, "1"));
        let path = self.cache_dir.join(format!("{name}-{version}-x86_64.xp"));
        let pkginfo = format!(
            "pkgname = {name}\npkgver = {pkgver}-{release}\npkgdesc = test\narch = x86_64\n"
        );
        let mtree =
            "#mtree\n./usr type=dir mode=0755 uid=0 gid=0\n./usr/share type=dir mode=0755 uid=0 gid=0\n";

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
            let mut header = tar::Header::new_gnu();
            header.set_path("usr/").unwrap();
            header.set_size(0);
            header.set_mode(0o755);
            header.set_entry_type(tar::EntryType::Directory);
            header.set_cksum();
            builder.append(&header, &[][..]).unwrap();
            let mut header = tar::Header::new_gnu();
            header.set_path("usr/share/").unwrap();
            header.set_size(0);
            header.set_mode(0o755);
            header.set_entry_type(tar::EntryType::Directory);
            header.set_cksum();
            builder.append(&header, &[][..]).unwrap();
            builder.finish().unwrap();
        }

        let compressed = zstd::encode_all(&raw_tar[..], 1).expect("compress");
        let mut file = fs::File::create(&path).expect("create package");
        file.write_all(&compressed).expect("write package");
        path
    }

    /// Records a successful journal entry, as `xpm upgrade` would.
    fn record_journal(&self, packages: Vec<JournalPackage>) -> Journal {
        let mut journal = Journal::start(&self.journal_dir(), "upgrade", &self.root, packages)
            .expect("start journal");
        journal.finish("ok", None).expect("finish journal");
        journal
    }

    fn run(&self, args: &[&str]) -> std::process::Output {
        Command::new(env!("CARGO_BIN_EXE_xpm"))
            .arg("--no-confirm")
            .arg("-c")
            .arg(&self.config)
            .args(args)
            .env("HOME", &self.home)
            .env("XPM_HOOKS_DIR", self._tmp.path().join("hooks"))
            .env("XPM_ALPM_HOOKS_DIRS", &self.alpm_hooks_dir)
            .env("XPM_ALPM_HOOKS_NO_CHROOT", "1")
            .output()
            .expect("run xpm")
    }
}

mod diff_history;
mod hooks;
mod rollback;
