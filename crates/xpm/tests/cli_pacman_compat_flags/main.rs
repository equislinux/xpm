//! End-to-end coverage for the added pacman compatibility flags:
//! `-Sw`/`-Suw`, `-Qn`, `-Sg`, `-Sl`, `-Sc`/`-Scc`, `-Qp`, `-Qc`,
//! `-F`/`-Fl`/`-Fq`/`-Fy`, `--ignoregroup`, `--assume-installed` and
//! `--sysroot`.

use std::fs;
use std::io::Write;
use std::path::PathBuf;
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

    fn with_files(mut self, files: &[&str]) -> Self {
        self.files = files.iter().map(|file| file.to_string()).collect();
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

mod files_cache;
mod sync_query;
mod upgrade;
