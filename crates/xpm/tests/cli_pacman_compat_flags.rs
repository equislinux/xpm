//! End-to-end coverage for the added pacman compatibility flags:
//! `-Sw`/`-Suw`, `-Qn`, `-Sg`, `-Sl`, `-Sc`/`-Scc`, `-Qp`, `-Qc`,
//! `-F`/`-Fl`/`-Fq`/`-Fy`, `--ignoregroup`, `--assume-installed` and
//! `--sysroot`.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use sha2::{Digest, Sha256};

struct Fixture {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    db_path: PathBuf,
    cache_dir: PathBuf,
    home: PathBuf,
    config: PathBuf,
    mirror: PathBuf,
    arch: String,
}

#[derive(Clone)]
struct Entry {
    name: String,
    version: String,
    depends: Vec<String>,
    groups: Vec<String>,
    files: Vec<String>,
}

impl Entry {
    fn new(name: &str, version: &str) -> Self {
        Self {
            name: name.to_string(),
            version: version.to_string(),
            depends: Vec::new(),
            groups: Vec::new(),
            files: vec![format!("usr/bin/{name}")],
        }
    }

    fn with_groups(mut self, groups: &[&str]) -> Self {
        self.groups = groups.iter().map(|group| group.to_string()).collect();
        self
    }

    fn with_depends(mut self, depends: &[&str]) -> Self {
        self.depends = depends.iter().map(|dep| dep.to_string()).collect();
        self
    }
}

impl Fixture {
    fn new() -> Self {
        let tmp = tempfile::TempDir::new().expect("tmp");
        let root = tmp.path().join("root");
        let db_path = root.join("var/lib/xpm");
        let cache_dir = root.join("var/cache/xpm/pkg");
        let home = tmp.path().join("home");
        let arch = std::env::consts::ARCH.to_string();
        let mirror = tmp.path().join("mirror");
        for dir in [&db_path, &cache_dir, &home, &mirror.join(&arch)] {
            fs::create_dir_all(dir).expect("dir");
        }

        let config = tmp.path().join("xpm.conf");
        fs::write(
            &config,
            format!(
                "[options]\nroot_dir = \"{}\"\ndb_path = \"{}\"\ncache_dir = \"{}\"\nsig_level = \"never\"\n\n\
                 [[repo]]\nname = \"core\"\nserver = [\"file://{}/$arch\"]\n\n\
                 [[repo]]\nname = \"extra\"\nserver = [\"file://{}/$arch\"]\n",
                root.display(),
                db_path.display(),
                cache_dir.display(),
                mirror.display(),
                mirror.display()
            ),
        )
        .expect("config");
        fs::write(
            mirror.join(&arch).join("extra.db"),
            build_repo_db(&[], false),
        )
        .expect("empty extra db");
        fs::write(
            mirror.join(&arch).join("extra.files"),
            build_repo_db(&[], true),
        )
        .expect("empty extra files");

        Self {
            _tmp: tmp,
            root,
            db_path,
            cache_dir,
            home,
            config,
            mirror,
            arch,
        }
    }

    fn local_db(&self) -> PathBuf {
        self.db_path.join("local")
    }

    fn sync_dir(&self) -> PathBuf {
        self.db_path.join("sync")
    }

    fn add_installed(&self, name: &str, version: &str) {
        let dir = self.local_db().join(name);
        fs::create_dir_all(&dir).expect("pkg dir");
        fs::write(dir.join("version"), version).expect("version");
    }

    /// Publishes packages and their sync/files databases into the file mirror.
    fn publish(&self, repo: &str, entries: &[Entry]) {
        fs::create_dir_all(self.mirror.join(&self.arch)).expect("mirror arch");
        fs::create_dir_all(self.sync_dir()).expect("sync dir");

        for entry in entries {
            let bytes = build_package(&self.arch, entry);
            let filename = format!("{}-{}-{}.pkg.tar.zst", entry.name, entry.version, self.arch);
            fs::write(self.mirror.join(&self.arch).join(filename), bytes).expect("package");
        }

        let db = build_repo_db(entries, false);
        let files = build_repo_db(entries, true);
        for directory in [&self.mirror.join(&self.arch), &self.sync_dir()] {
            fs::write(directory.join(format!("{repo}.db")), &db).expect("sync db");
            fs::write(directory.join(format!("{repo}.files")), &files).expect("files db");
        }
    }

    /// Creates only the local sync databases (no remote sync needed).
    fn write_sync_db(&self, repo: &str, entries: &[Entry]) {
        fs::create_dir_all(self.sync_dir()).expect("sync dir");
        fs::write(
            self.sync_dir().join(format!("{repo}.db")),
            build_repo_db(entries, false),
        )
        .expect("sync db");
        fs::write(
            self.sync_dir().join(format!("{repo}.files")),
            build_repo_db(entries, true),
        )
        .expect("files db");
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

fn build_package(arch: &str, entry: &Entry) -> Vec<u8> {
    let mut pkginfo = format!(
        "pkgname = {}\npkgver = {}\npkgdesc = test package\narch = {arch}\n",
        entry.name, entry.version
    );
    for dep in &entry.depends {
        pkginfo.push_str(&format!("depend = {dep}\n"));
    }
    for group in &entry.groups {
        pkginfo.push_str(&format!("group = {group}\n"));
    }

    let mut mtree = String::from(
        "#mtree\n./usr type=dir mode=0755 uid=0 gid=0\n./usr/bin type=dir mode=0755 uid=0 gid=0\n",
    );
    for file in &entry.files {
        let content = format!("{}\n", entry.name);
        mtree.push_str(&format!(
            "./{file} type=file mode=0755 size={} sha256digest={} uid=0 gid=0\n",
            content.len(),
            sha256_hex(content.as_bytes())
        ));
    }

    let mut raw = Vec::new();
    {
        let mut builder = tar::Builder::new(&mut raw);
        for (path, data) in [
            (".PKGINFO", pkginfo.as_bytes()),
            (".MTREE", mtree.as_bytes()),
        ] {
            let mut header = tar::Header::new_gnu();
            header.set_path(path).unwrap();
            header.set_size(data.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder.append(&header, data).unwrap();
        }
        for file in &entry.files {
            let data = format!("{}\n", entry.name);
            let mut header = tar::Header::new_gnu();
            header.set_path(file).unwrap();
            header.set_size(data.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            builder.append(&header, data.as_bytes()).unwrap();
        }
        builder.finish().unwrap();
    }

    zstd::encode_all(&raw[..], 1).expect("compress package")
}

fn build_repo_db(entries: &[Entry], files_only: bool) -> Vec<u8> {
    let mut raw = Vec::new();
    {
        let mut builder = tar::Builder::new(&mut raw);
        for entry in entries {
            let id = format!("{}-{}", entry.name, entry.version);
            let filename = format!(
                "{}-{}-{}.pkg.tar.zst",
                entry.name,
                entry.version,
                std::env::consts::ARCH
            );
            let mut desc = format!(
                "%NAME%\n{}\n\n%VERSION%\n{}\n\n%FILENAME%\n{filename}\n",
                entry.name, entry.version
            );
            for dep in &entry.depends {
                desc.push_str(&format!("\n%DEPENDS%\n{dep}\n"));
            }
            for group in &entry.groups {
                desc.push_str(&format!("\n%GROUPS%\n{group}\n"));
            }
            let desc_bytes = desc.as_bytes();
            let mut header = tar::Header::new_gnu();
            header.set_path(format!("{id}/desc")).unwrap();
            header.set_size(desc_bytes.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder.append(&header, desc_bytes).unwrap();

            if !files_only {
                continue;
            }
            let mut files = String::from("%FILES%\n");
            for file in &entry.files {
                files.push_str(file);
                files.push('\n');
            }
            let files_bytes = files.as_bytes();
            let mut header = tar::Header::new_gnu();
            header.set_path(format!("{id}/files")).unwrap();
            header.set_size(files_bytes.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder.append(&header, files_bytes).unwrap();
        }
        builder.finish().unwrap();
    }

    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(&raw).expect("gzip db");
    encoder.finish().expect("finish gzip")
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "command failed\nstdout:\n{}\nstderr:\n{}",
        stdout(output),
        stderr(output)
    );
}

#[test]
fn sync_listing_groups_native_file_changelog_and_package_queries() {
    let fixture = Fixture::new();
    fixture.add_installed("hello", "1.0-1");
    fixture.add_installed("local-only", "3.0-1");
    fixture.write_sync_db(
        "core",
        &[Entry::new("hello", "1.0-1").with_groups(&["base"])],
    );
    fixture.write_sync_db(
        "extra",
        &[Entry::new("tool", "2.0-1").with_groups(&["desktop"])],
    );

    let list = fixture.run(&["-Sl"]);
    assert_success(&list);
    let list_stdout = stdout(&list);
    assert!(list_stdout.contains("core hello 1.0-1 [installed]"));
    assert!(list_stdout.contains("extra tool 2.0-1"));
    assert!(!list_stdout.contains("extra tool 2.0-1 [installed]"));

    let list_quiet = fixture.run(&["-Slq", "core"]);
    assert_success(&list_quiet);
    assert_eq!(stdout(&list_quiet), "hello\n");

    let missing_repo = fixture.run(&["-Sl", "missing"]);
    assert_eq!(missing_repo.status.code(), Some(1));
    assert!(stderr(&missing_repo).contains("repository 'missing' was not found"));

    let groups = fixture.run(&["-Sg"]);
    assert_success(&groups);
    assert_eq!(stdout(&groups), "base\ndesktop\n");

    let groups_all = fixture.run(&["-Sgg"]);
    assert_success(&groups_all);
    assert_eq!(stdout(&groups_all), "base hello\ndesktop tool\n");

    let group_members = fixture.run(&["-Sgq", "desktop"]);
    assert_success(&group_members);
    assert_eq!(stdout(&group_members), "tool\n");

    let native = fixture.run(&["-Qn"]);
    assert_success(&native);
    assert_eq!(stdout(&native), "hello 1.0-1\n");

    let native_quiet = fixture.run(&["-Qnq"]);
    assert_success(&native_quiet);
    assert_eq!(stdout(&native_quiet), "hello\n");

    let changed = fixture.local_db().join("hello").join("changelog");
    fs::write(&changed, "1.0-1: initial release\n").expect("changelog");
    let changelog = fixture.run(&["-Qc", "hello"]);
    assert_success(&changelog);
    assert_eq!(stdout(&changelog), "1.0-1: initial release\n");

    let no_changelog = fixture.run(&["-Qc", "local-only"]);
    assert_eq!(no_changelog.status.code(), Some(1));
    assert!(stderr(&no_changelog).contains("no changelog available for 'local-only'"));

    let entry = Entry::new("hello", "1.0-1");
    let package = fixture._tmp.path().join("hello.pkg.tar.zst");
    fs::write(&package, build_package(&fixture.arch, &entry)).expect("package file");
    let package_query = fixture.run(&["-Qp", package.to_str().unwrap()]);
    assert_success(&package_query);
    assert_eq!(stdout(&package_query), "hello 1.0-1\n");

    let package_quiet = fixture.run(&["-Qpq", package.to_str().unwrap()]);
    assert_success(&package_quiet);
    assert_eq!(stdout(&package_quiet), "hello\n");
}

#[test]
fn files_database_queries_and_refresh() {
    let fixture = Fixture::new();
    let hello = Entry::new("hello", "1.0-1");
    fixture.publish("core", std::slice::from_ref(&hello));

    let refresh = fixture.run(&["-Fy"]);
    assert_success(&refresh);

    let exact = fixture.run(&["-F", "usr/bin/hello"]);
    assert_success(&exact);
    assert_eq!(
        stdout(&exact),
        "usr/bin/hello is owned by core/hello 1.0-1\n"
    );

    let leading_slash = fixture.run(&["-F", "/usr/bin/hello"]);
    assert_success(&leading_slash);
    assert_eq!(
        stdout(&leading_slash),
        "/usr/bin/hello is owned by core/hello 1.0-1\n"
    );

    let quiet = fixture.run(&["-Fq", "usr/bin/hello"]);
    assert_success(&quiet);
    assert_eq!(stdout(&quiet), "core/hello\n");

    let list = fixture.run(&["-Fl", "hello"]);
    assert_success(&list);
    assert_eq!(stdout(&list), "hello usr/bin/hello\n");

    let missing = fixture.run(&["-F", "usr/bin/missing"]);
    assert_eq!(missing.status.code(), Some(1));
    assert!(stderr(&missing).contains("no package owns"));
}

#[test]
fn cache_clean_safe_and_all_modes() {
    let fixture = Fixture::new();
    fixture.add_installed("hello", "1.0-1");
    let entry = Entry::new("hello", "1.0-1");

    let current = fixture
        .cache_dir
        .join(format!("hello-1.0-1-{}.pkg.tar.zst", fixture.arch));
    let old = fixture
        .cache_dir
        .join(format!("hello-0.9-1-{}.pkg.tar.zst", fixture.arch));
    let orphan = fixture
        .cache_dir
        .join(format!("orphan-2.0-1-{}.pkg.tar.zst", fixture.arch));
    fs::write(&current, build_package(&fixture.arch, &entry)).expect("current");
    fs::write(
        &old,
        build_package(&fixture.arch, &Entry::new("hello", "0.9-1")),
    )
    .expect("old");
    fs::write(
        &orphan,
        build_package(&fixture.arch, &Entry::new("orphan", "2.0-1")),
    )
    .expect("orphan");
    fs::write(
        fixture
            .cache_dir
            .join(format!("broken-1.0-1-{}.pkg.tar.zst", fixture.arch)),
        b"garbage",
    )
    .expect("corrupt");
    fs::write(fixture.cache_dir.join("notes.txt"), b"keep").expect("notes");

    let safe = fixture.run(&["-Sc"]);
    assert_success(&safe);
    assert!(current.is_file(), "the installed version must stay cached");
    assert!(!old.exists());
    assert!(!orphan.exists());
    assert!(fixture
        .cache_dir
        .join(format!("broken-1.0-1-{}.pkg.tar.zst", fixture.arch))
        .is_file());
    assert!(fixture.cache_dir.join("notes.txt").is_file());
    assert!(stdout(&safe).contains("Removed 2 cached package file(s)"));

    let all = fixture.run(&["-Scc"]);
    assert_success(&all);
    assert!(!current.exists());
    assert!(fixture.cache_dir.join("notes.txt").is_file());
    assert!(fs::read_dir(&fixture.cache_dir)
        .expect("cache dir")
        .filter_map(|entry| entry.ok())
        .all(|entry| entry.file_name() == "notes.txt"));
}

#[test]
fn download_only_ignoregroup_assume_installed_and_sysroot() {
    let fixture = Fixture::new();
    let app = Entry::new("app", "1.0-1").with_depends(&["virtual>=2"]);
    let kernel = Entry::new("kernel", "1.0-1").with_groups(&["sys"]);
    fixture.publish("core", &[app, kernel]);

    let sync = fixture.run(&["sync"]);
    assert_success(&sync);

    // -Sw downloads the package but does not install it.
    let download = fixture.run(&["-Sw", "kernel"]);
    assert_success(&download);
    let cached = fixture
        .cache_dir
        .join(format!("kernel-1.0-1-{}.pkg.tar.zst", fixture.arch));
    assert!(cached.is_file(), "download-only must populate the cache");
    assert!(!fixture.root.join("usr/bin/kernel").exists());
    assert!(!fixture.local_db().join("kernel").exists());

    // --assume-installed satisfies a dependency with no provider.
    let print = fixture.run(&["-Sp", "--assume-installed", "virtual=2.0-1", "app"]);
    assert_success(&print);
    assert!(stdout(&print).contains("app 1.0-1"));

    let missing = fixture.run(&["-Sp", "app"]);
    assert_eq!(missing.status.code(), Some(1));

    let install = fixture.run(&["-S", "--assume-installed", "virtual=2.0-1", "app"]);
    assert_success(&install);
    assert!(fixture.root.join("usr/bin/app").exists());
    assert!(fixture.local_db().join("app").is_dir());

    // A sysroot redirects the local database location.
    let sysroot = fixture._tmp.path().join("sysroot");
    let sysroot_local = sysroot.join("var/lib/xpm/local/hello");
    fs::create_dir_all(&sysroot_local).expect("sysroot local db");
    fs::write(sysroot_local.join("version"), "9.0-1").expect("sysroot version");

    let query = fixture.run(&["--sysroot", sysroot.to_str().unwrap(), "query"]);
    assert_success(&query);
    assert_eq!(stdout(&query), "hello 9.0-1\n");
}

#[test]
fn sysupgrade_download_only_leaves_the_installed_version_alone() {
    let fixture = Fixture::new();
    fixture.add_installed("kernel", "1.0-1");
    fixture.publish(
        "core",
        &[Entry::new("kernel", "2.0-1").with_groups(&["sys"])],
    );

    let upgrade = fixture.run(&["-Suw"]);
    assert_success(&upgrade);
    let cached = fixture
        .cache_dir
        .join(format!("kernel-2.0-1-{}.pkg.tar.zst", fixture.arch));
    assert!(cached.is_file(), "-Suw must download the new package");

    let version = fs::read_to_string(fixture.local_db().join("kernel/version")).expect("version");
    assert_eq!(version.trim(), "1.0-1", "download-only must not upgrade");
    assert!(!fixture.root.join("usr/bin/kernel").exists());
}

#[test]
fn ignoregroup_skips_sysupgrade_packages() {
    let fixture = Fixture::new();
    fixture.add_installed("kernel", "1.0-1");
    fs::create_dir_all(fixture.local_db().join("kernel")).expect("local db");
    fs::write(fixture.local_db().join("kernel/groups"), "sys\n").expect("groups");

    let kernel_v1 = Entry::new("kernel", "1.0-1").with_groups(&["sys"]);
    fixture.publish("core", std::slice::from_ref(&kernel_v1));
    assert_success(&fixture.run(&["sync"]));

    let kernel_v2 = Entry::new("kernel", "2.0-1").with_groups(&["sys"]);
    fixture.publish("core", &[kernel_v2]);
    let upgrade = fixture.run(&["-Syu", "--ignoregroup", "sys"]);
    assert_success(&upgrade);
    assert!(stdout(&upgrade).contains("Nothing to do"));

    let version = fs::read_to_string(fixture.local_db().join("kernel/version")).expect("version");
    assert_eq!(version.trim(), "1.0-1");
}

#[allow(dead_code)]
fn _path_exists(path: &Path) -> bool {
    path.exists()
}
