//! End-to-end tests for the pacman-compatible query and repair commands:
//! exact queries, quiet output, owner/foreign/unrequired lookups, file checks,
//! install-reason changes and dependency tests. Everything runs against a
//! temporary root/database with the real binary; no root privileges or network
//! are required.

use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Command, Output};

use sha2::{Digest, Sha256};

struct Fixture {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    db_path: PathBuf,
    home: PathBuf,
    config: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let tmp = tempfile::TempDir::new().expect("tmp");
        let root = tmp.path().join("root");
        let db_path = root.join("var/lib/xpm");
        let cache_dir = root.join("var/cache/xpm/pkg");
        let home = tmp.path().join("home");
        for dir in [&db_path, &cache_dir, &home] {
            fs::create_dir_all(dir).expect("dir");
        }

        let config = tmp.path().join("xpm.conf");
        fs::write(
            &config,
            format!(
                "[options]\nroot_dir = \"{}\"\ndb_path = \"{}\"\ncache_dir = \"{}\"\nsig_level = \"never\"\n\n\
                 [[repo]]\nname = \"core\"\nserver = [\"file:///nonexistent/$arch\"]\n",
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
            home,
            config,
        }
    }

    fn local_db(&self) -> PathBuf {
        self.db_path.join("local")
    }

    fn add_package(&self, name: &str, version: &str) {
        let dir = self.local_db().join(name);
        fs::create_dir_all(&dir).expect("pkg dir");
        fs::write(dir.join("version"), version).expect("version");
    }

    fn write_reason(&self, name: &str, reason: &str) {
        fs::write(self.local_db().join(name).join("reason"), reason).expect("reason");
    }

    fn write_list(&self, name: &str, file: &str, entries: &[&str]) {
        let mut out = String::new();
        for entry in entries {
            out.push_str(entry);
            out.push('\n');
        }
        fs::write(self.local_db().join(name).join(file), out).expect("list file");
    }

    fn write_files(&self, name: &str, files: &[&str]) {
        let mut out = String::from("%FILES%\n");
        for file in files {
            out.push_str(file);
            out.push('\n');
        }
        fs::write(self.local_db().join(name).join("files"), out).expect("files");
    }

    fn write_mtree(&self, name: &str, contents: &str) {
        fs::write(self.local_db().join(name).join("mtree"), contents).expect("mtree");
    }

    fn set_mode(&self, relative: &str, mode: u32) {
        let path = self.root.join(relative);
        let mut permissions = fs::metadata(&path).expect("metadata").permissions();
        permissions.set_mode(mode);
        fs::set_permissions(&path, permissions).expect("chmod");
    }

    fn write_file(&self, relative: &str, data: &[u8], mode: u32) {
        let path = self.root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("parent");
        }
        let mut file = fs::File::create(&path).expect("create file");
        file.write_all(data).expect("write file");
        drop(file);
        let mut permissions = fs::metadata(&path).expect("metadata").permissions();
        permissions.set_mode(mode);
        fs::set_permissions(&path, permissions).expect("chmod");
    }

    fn write_sync_db(&self, repo: &str, packages: &[(&str, &str)]) {
        let sync_dir = self.db_path.join("sync");
        fs::create_dir_all(&sync_dir).expect("sync dir");

        let mut raw = Vec::new();
        {
            let mut builder = tar::Builder::new(&mut raw);
            for (name, version) in packages {
                let desc = format!("%NAME%\n{name}\n\n%VERSION%\n{version}\n");
                let bytes = desc.as_bytes();
                let mut header = tar::Header::new_gnu();
                header.set_size(bytes.len() as u64);
                header.set_mode(0o644);
                header.set_cksum();
                builder
                    .append_data(&mut header, format!("{name}-{version}/desc"), bytes)
                    .expect("append sync entry");
            }
            builder.finish().expect("finish sync db");
        }

        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(&raw).expect("gzip sync db");
        let bytes = encoder.finish().expect("finish gzip");
        fs::write(sync_dir.join(format!("{repo}.db")), bytes).expect("write sync db");
    }

    fn read_reason(&self, name: &str) -> String {
        fs::read_to_string(self.local_db().join(name).join("reason"))
            .expect("read reason")
            .trim()
            .to_string()
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_xpm"))
            .arg("-c")
            .arg(&self.config)
            .arg("--no-confirm")
            .args(args)
            .env("HOME", &self.home)
            .env("XPM_HOOKS_DIR", self._tmp.path().join("hooks"))
            .env("XPM_ALPM_HOOKS_DIRS", self._tmp.path().join("alpm-hooks"))
            .output()
            .expect("run xpm")
    }
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn sha256_hex(data: &[u8]) -> String {
    Sha256::digest(data)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

mod check;
mod mark;
mod query;
