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

#[test]
fn query_exact_search_quiet_owner_and_foreign() {
    let fixture = Fixture::new();
    fixture.add_package("hello", "1.0-1");
    fixture.add_package("hello-world", "2.0-1");
    fixture.add_package("local-only", "3.0-1");

    fixture.write_files("hello", &["usr/", "usr/bin/", "usr/bin/hello"]);
    fixture.write_files("hello-world", &["usr/bin/hello-world"]);
    fixture.write_files("local-only", &["usr/bin/local-only"]);

    let exact = fixture.run(&["query", "hello"]);
    assert!(exact.status.success(), "{}", stderr(&exact));
    assert_eq!(stdout(&exact), "hello 1.0-1\n");

    let missing = fixture.run(&["query", "hell"]);
    assert_eq!(missing.status.code(), Some(1));
    assert!(
        stderr(&missing).contains("package 'hell' was not found"),
        "{}",
        stderr(&missing)
    );

    let search = fixture.run(&["query", "--search", "hello"]);
    assert!(search.status.success(), "{}", stderr(&search));
    let search_stdout = stdout(&search);
    assert!(search_stdout.contains("hello 1.0-1"));
    assert!(search_stdout.contains("hello-world 2.0-1"));

    let quiet = fixture.run(&["-Qq"]);
    assert!(quiet.status.success(), "{}", stderr(&quiet));
    let quiet_stdout = stdout(&quiet);
    for name in ["hello", "hello-world", "local-only"] {
        assert!(
            quiet_stdout.lines().any(|line| line == name),
            "missing quiet name {name}: {quiet_stdout}"
        );
    }
    assert!(!quiet_stdout.contains("1.0-1"), "{quiet_stdout}");

    let owner = fixture.run(&["-Qo", "/usr/bin/hello"]);
    assert!(owner.status.success(), "{}", stderr(&owner));
    assert_eq!(stdout(&owner), "/usr/bin/hello is owned by hello 1.0-1\n");

    let owner_dir = fixture.run(&["query", "--owner", "usr/bin/"]);
    assert!(owner_dir.status.success(), "{}", stderr(&owner_dir));
    assert!(stdout(&owner_dir).contains("hello"));

    let unowned = fixture.run(&["-Qo", "/usr/bin/nope"]);
    assert_eq!(unowned.status.code(), Some(1));
    assert!(
        stderr(&unowned).contains("no package owns /usr/bin/nope"),
        "{}",
        stderr(&unowned)
    );

    fixture.write_sync_db("core", &[("hello", "1.0-1"), ("hello-world", "2.0-1")]);
    let foreign = fixture.run(&["-Qm"]);
    assert!(foreign.status.success(), "{}", stderr(&foreign));
    assert_eq!(stdout(&foreign), "local-only 3.0-1\n");

    let foreign_quiet = fixture.run(&["-Qmq"]);
    assert!(foreign_quiet.status.success(), "{}", stderr(&foreign_quiet));
    assert_eq!(stdout(&foreign_quiet), "local-only\n");
}

#[test]
fn query_unrequired_and_deps_modes_differ_from_orphans() {
    let fixture = Fixture::new();
    for name in ["app", "orphan-a", "orphan-b", "explicit-extra"] {
        fixture.add_package(name, "1.0-1");
    }
    fixture.write_reason("app", "explicit");
    fixture.write_reason("orphan-a", "dep");
    fixture.write_reason("orphan-b", "dep");
    fixture.write_reason("explicit-extra", "explicit");
    fixture.write_list("orphan-a", "depends", &["orphan-b"]);
    fixture.write_list("orphan-b", "depends", &[]);

    let unrequired = fixture.run(&["-Qt"]);
    assert!(unrequired.status.success(), "{}", stderr(&unrequired));
    let unrequired_stdout = stdout(&unrequired);
    assert!(
        unrequired_stdout.contains("app 1.0-1"),
        "{unrequired_stdout}"
    );
    assert!(
        unrequired_stdout.contains("explicit-extra 1.0-1"),
        "{unrequired_stdout}"
    );
    assert!(
        unrequired_stdout.contains("orphan-a 1.0-1"),
        "orphan-a is not required by anything else: {unrequired_stdout}"
    );
    assert!(
        !unrequired_stdout.contains("orphan-b 1.0-1"),
        "orphan-b is directly required by orphan-a: {unrequired_stdout}"
    );

    let deps = fixture.run(&["-Qdt"]);
    assert!(deps.status.success(), "{}", stderr(&deps));
    assert_eq!(stdout(&deps), "orphan-a 1.0-1\n");

    let orphans = fixture.run(&["query", "--orphans"]);
    assert!(orphans.status.success(), "{}", stderr(&orphans));
    let orphans_stdout = stdout(&orphans);
    assert!(orphans_stdout.contains("orphan-a"));
    assert!(
        orphans_stdout.contains("orphan-b"),
        "the transitive orphan mode is preserved: {orphans_stdout}"
    );

    let groups_quiet = fixture.run(&["-Qqg"]);
    assert!(groups_quiet.status.success(), "{}", stderr(&groups_quiet));
}

#[test]
fn check_fast_and_full_report_statuses() {
    let fixture = Fixture::new();
    fixture.add_package("hello", "1.0-1");
    fixture.add_package("broken", "1.0-1");

    let data = b"#!/bin/sh\necho hello\n";
    fixture.write_file("usr/bin/hello", data, 0o755);
    fixture.set_mode("usr", 0o755);
    fixture.set_mode("usr/bin", 0o755);
    fixture.write_files("hello", &["usr/", "usr/bin/", "usr/bin/hello"]);
    let hash = sha256_hex(data);
    fixture.write_mtree(
        "hello",
        &format!(
            "#mtree\n\
             ./usr type=dir mode=0755 uid=0 gid=0\n\
             ./usr/bin type=dir mode=0755 uid=0 gid=0\n\
             ./usr/bin/hello type=file mode=0755 size={} sha256digest={hash} uid=0 gid=0\n",
            data.len()
        ),
    );

    fixture.write_files("broken", &["usr/bin/missing"]);

    let fast = fixture.run(&["-Qk"]);
    assert_eq!(fast.status.code(), Some(1));
    let fast_stdout = stdout(&fast);
    assert!(
        fast_stdout.contains("hello: 3 total files, 0 missing files"),
        "{fast_stdout}"
    );
    assert!(
        fast_stdout.contains("broken: 1 total files, 1 missing files"),
        "{fast_stdout}"
    );

    let full_ok = fixture.run(&["-Qkk", "hello"]);
    assert!(full_ok.status.success(), "{}", stderr(&full_ok));
    assert!(
        stdout(&full_ok).contains("hello: 3 total files, 0 altered files"),
        "{}",
        stdout(&full_ok)
    );

    fixture.write_file("usr/bin/hello", b"tampered", 0o755);
    let full_altered = fixture.run(&["-Qkk", "hello"]);
    assert_eq!(full_altered.status.code(), Some(1));
    let full_stdout = stdout(&full_altered);
    assert!(
        full_stdout.contains("usr/bin/hello (altered)"),
        "{full_stdout}"
    );
    assert!(
        full_stdout.contains("hello: 3 total files, 1 altered files"),
        "{full_stdout}"
    );

    let unknown = fixture.run(&["check", "does-not-exist"]);
    assert!(!unknown.status.success());
    assert!(
        stderr(&unknown).contains("does-not-exist"),
        "{}",
        stderr(&unknown)
    );
}

#[test]
fn mark_and_deptest_pacman_mappings() {
    let fixture = Fixture::new();
    fixture.add_package("glibc", "2.39-1");
    fixture.add_package("hello", "1.0-1");
    fixture.write_reason("hello", "explicit");

    fixture.add_package("firefox", "128.0-1");
    fixture.write_list("firefox", "provides", &["browser=128.0"]);

    let satisfied = fixture.run(&["-T", "glibc>=2.38", "browser>=120", "hello"]);
    assert!(satisfied.status.success(), "{}", stderr(&satisfied));
    assert!(stdout(&satisfied).is_empty());

    let unmet = fixture.run(&["-T", "glibc>=3", "browser>=130", "missing"]);
    assert_eq!(unmet.status.code(), Some(127));
    assert_eq!(stdout(&unmet), "glibc>=3\nbrowser>=130\nmissing\n");

    let as_deps = fixture.run(&["-D", "--asdeps", "hello"]);
    assert!(as_deps.status.success(), "{}", stderr(&as_deps));
    assert!(stdout(&as_deps).contains("Marked 1 package(s)"));
    assert_eq!(fixture.read_reason("hello"), "dep");

    let as_explicit = fixture.run(&["-D", "--asexplicit", "hello"]);
    assert!(as_explicit.status.success(), "{}", stderr(&as_explicit));
    assert_eq!(fixture.read_reason("hello"), "explicit");

    let unknown = fixture.run(&["-D", "--asdeps", "does-not-exist"]);
    assert!(!unknown.status.success());
    assert_eq!(fixture.read_reason("hello"), "explicit");
}
