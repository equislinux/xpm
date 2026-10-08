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
            .output()
            .expect("run xpm")
    }
}

#[test]
fn rollback_last_dry_run_and_execute_downgrade() {
    let fixture = Fixture::new();
    fixture.cache_package("hello", "0.9-1");
    fixture.install_into_db("hello", "1.0-1");
    fixture.record_journal(vec![JournalPackage::upgrade("hello", "0.9-1", "1.0-1")]);

    // Dry-run: plan only, database untouched.
    let dry = fixture.run(&["rollback", "--last", "--dry-run"]);
    assert!(
        dry.status.success(),
        "dry-run failed: {}",
        String::from_utf8_lossy(&dry.stderr)
    );
    let stdout = String::from_utf8_lossy(&dry.stdout);
    assert!(
        stdout.contains("reinstall hello 0.9-1"),
        "unexpected dry-run output: {stdout}"
    );
    assert_eq!(fixture.read_db_version("hello"), "1.0-1");

    // Real rollback: the cached 0.9-1 must replace the installed 1.0-1.
    let run = fixture.run(&["rollback", "--last"]);
    assert!(
        run.status.success(),
        "rollback failed: {}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(fixture.read_db_version("hello"), "0.9-1");

    // A new journal entry documents the rollback.
    let history = fixture.run(&["history", "--json"]);
    let history_out = String::from_utf8_lossy(&history.stdout);
    assert!(
        history_out.contains("\"action\":\"rollback\""),
        "history must include the rollback: {history_out}"
    );
}

#[test]
fn rollback_reports_missing_cache_file() {
    let fixture = Fixture::new();
    fixture.install_into_db("hello", "1.0-1");
    fixture.record_journal(vec![JournalPackage::upgrade("hello", "0.9-1", "1.0-1")]);

    let run = fixture.run(&["rollback", "--last"]);
    assert!(!run.status.success(), "rollback must fail on a cache miss");
    let stderr = String::from_utf8_lossy(&run.stderr);
    assert!(
        stderr.contains("not in") && stderr.contains("hello 0.9-1"),
        "unexpected error: {stderr}"
    );
    // Nothing changed.
    assert_eq!(fixture.read_db_version("hello"), "1.0-1");
}

#[test]
fn diff_compares_against_generation_capture() {
    let fixture = Fixture::new();
    fixture.install_into_db("hello", "0.9-1");

    let generation_dir = fixture.root.join("var/lib/x/generations/0007");
    fs::create_dir_all(&generation_dir).expect("generation dir");
    fs::write(
        generation_dir.join("packages.tsv"),
        "hello 1.0-1\nother 2.0-1\n",
    )
    .expect("packages.tsv");

    let run = fixture.run(&["diff", "0007", "--json"]);
    assert!(
        run.status.success(),
        "diff failed: {}",
        String::from_utf8_lossy(&run.stderr)
    );
    let stdout = String::from_utf8_lossy(&run.stdout);
    assert!(stdout.contains("\"generation\":\"0007\""), "json: {stdout}");
    assert!(
        stdout.contains("\"name\":\"other\"") && stdout.contains("\"removed\""),
        "expected removed package in diff: {stdout}"
    );
    assert!(
        stdout.contains("\"name\":\"hello\"") && stdout.contains("\"from\":\"1.0-1\""),
        "expected changed package in diff: {stdout}"
    );
}

#[test]
fn rollback_runs_alpm_hooks_in_order() {
    let fixture = Fixture::new();
    fixture.cache_package("hello", "0.9-1");
    fixture.install_into_db("hello", "1.0-1");
    fixture.record_journal(vec![JournalPackage::upgrade("hello", "0.9-1", "1.0-1")]);

    let marker = fixture._tmp.path().join("hook-order.txt");
    fixture.add_alpm_hook("10-pre", "PreTransaction", &marker, "pre");
    fixture.add_alpm_hook("20-post", "PostTransaction", &marker, "post");

    let run = fixture.run(&["rollback", "--last"]);
    assert!(
        run.status.success(),
        "rollback with hooks failed: {}",
        String::from_utf8_lossy(&run.stderr)
    );

    let order = fs::read_to_string(&marker).expect("hook marker");
    assert_eq!(order, "pre\npost\n");
    assert_eq!(fixture.read_db_version("hello"), "0.9-1");
}

#[test]
fn history_links_generation_and_diff_current_resolves_it() {
    let fixture = Fixture::new();
    fixture.install_into_db("hello", "0.9-1");

    // `current` points at generation 0009 which captured the same package.
    let state = fixture.root.join("var/lib/x");
    let generation_dir = state.join("generations/0009");
    fs::create_dir_all(&generation_dir).expect("generation dir");
    fs::write(state.join("current"), "0009\n").expect("current");
    fs::write(generation_dir.join("packages.tsv"), "hello 0.9-1\n").expect("packages.tsv");

    let diff = fixture.run(&["diff", "current"]);
    assert!(
        diff.status.success(),
        "diff current failed: {}",
        String::from_utf8_lossy(&diff.stderr)
    );
    assert!(
        String::from_utf8_lossy(&diff.stdout).contains("No differences"),
        "expected a clean diff: {}",
        String::from_utf8_lossy(&diff.stdout)
    );

    // Journal with a linked generation shows the marker in history.
    let mut journal = fixture.record_journal(vec![JournalPackage::install("hello", "0.9-1")]);
    journal.generation = Some("0009".to_string());
    journal.persist().expect("persist with generation");

    let history = fixture.run(&["history"]);
    assert!(
        String::from_utf8_lossy(&history.stdout).contains("gen:0009"),
        "history must show the generation: {}",
        String::from_utf8_lossy(&history.stdout)
    );
}
