//! Full CLI journey against a local HTTP repository:
//! `sync` → `install` → user edits a config file → `upgrade` (with the
//! `.pacnew` rule) → generation linking in `history`.
//!
//! The tiny HTTP server and all package/database artifacts are built in
//! temporary directories: the test is hermetic, needs no root and no network.

use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::{Arc, Mutex};
use std::thread;

use sha2::{Digest, Sha256};

// ── Minimal HTTP server ─────────────────────────────────────────────────────

/// Serves a fixed path → bytes map. Path keys include the leading `/`.
struct TestServer {
    base_url: String,
    routes: Arc<Mutex<HashMap<String, Vec<u8>>>>,
}

impl TestServer {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let routes: Arc<Mutex<HashMap<String, Vec<u8>>>> = Arc::new(Mutex::new(HashMap::new()));
        let served = Arc::clone(&routes);

        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut buffer = [0u8; 8192];
                let read = stream.read(&mut buffer).unwrap_or(0);
                let request = String::from_utf8_lossy(&buffer[..read]);
                let path = request
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .unwrap_or("/")
                    .to_string();

                let body = served.lock().expect("routes").get(&path).cloned();
                let response = match body {
                    Some(body) => {
                        let mut response = format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        )
                        .into_bytes();
                        response.extend_from_slice(&body);
                        response
                    }
                    None => {
                        b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                            .to_vec()
                    }
                };
                let _ = stream.write_all(&response);
            }
        });

        Self {
            base_url: format!("http://127.0.0.1:{port}"),
            routes,
        }
    }

    fn set(&self, path: &str, bytes: Vec<u8>) {
        self.routes
            .lock()
            .expect("routes")
            .insert(path.to_string(), bytes);
    }
}

// ── Fixture ─────────────────────────────────────────────────────────────────

struct Fixture {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    db_path: PathBuf,
    home: PathBuf,
    config: PathBuf,
    arch: String,
}

impl Fixture {
    fn new(server_base: &str) -> Self {
        let tmp = tempfile::TempDir::new().expect("tmp");
        let root = tmp.path().join("root");
        let db_path = root.join("var/lib/xpm");
        let cache_dir = root.join("var/cache/xpm/pkg");
        let home = tmp.path().join("home");
        for dir in [&db_path, &cache_dir, &home] {
            fs::create_dir_all(dir).expect("dir");
        }

        let arch = std::env::consts::ARCH.to_string();
        let config = tmp.path().join("xpm.conf");
        fs::write(
            &config,
            format!(
                "[options]\nroot_dir = \"{}\"\ndb_path = \"{}\"\ncache_dir = \"{}\"\nsig_level = \"never\"\n\n\
                 [[repo]]\nname = \"test\"\nserver = [\"{server_base}/x/$arch\"]\n",
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
            arch,
        }
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

    fn read(&self, relative: &str) -> String {
        fs::read_to_string(self.root.join(relative))
            .unwrap_or_else(|e| panic!("read {relative}: {e}"))
    }

    /// Writes the current/default generation so journals link to it.
    fn set_generation(&self, id: &str) {
        let state = self.root.join("var/lib/x");
        fs::create_dir_all(state.join("generations").join(id)).expect("state");
        fs::write(state.join("current"), format!("{id}\n")).expect("current");
    }
}

// ── Artifacts ───────────────────────────────────────────────────────────────

fn sha256_hex(data: &[u8]) -> String {
    Sha256::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Builds a `.xp` package: a binary, a configuration file (declared as
/// `backup`) and the matching `.PKGINFO`/`.MTREE`.
fn build_package(name: &str, version: &str, binary: &str, config: &str) -> Vec<u8> {
    let files: [(&str, &[u8]); 2] = [
        ("usr/bin/hello", binary.as_bytes()),
        ("etc/hello.conf", config.as_bytes()),
    ];

    let pkginfo = format!(
        "pkgname = {name}\npkgver = {version}\npkgdesc = hello test package\narch = {}\nbackup = etc/hello.conf\n",
        std::env::consts::ARCH
    );

    let mut mtree = String::from("#mtree\n");
    for (path, content) in &files {
        mtree.push_str(&format!(
            "./{path} type=file mode=0755 size={} sha256digest={} uid=0 gid=0\n",
            content.len(),
            sha256_hex(content)
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
        for (path, content) in &files {
            let mut header = tar::Header::new_gnu();
            header.set_path(path).unwrap();
            header.set_size(content.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            builder.append(&header, *content).unwrap();
        }
        builder.finish().unwrap();
    }

    zstd::encode_all(&raw[..], 1).expect("compress package")
}

/// Builds an xpkg-extended repo database (FILENAME included) as a `.tar.gz`.
fn build_db(name: &str, version: &str) -> Vec<u8> {
    let filename = format!("{name}-{version}-{}.xp", std::env::consts::ARCH);
    let desc = format!(
        "%NAME%\n{name}\n\n%VERSION%\n{version}\n\n%DESC%\nhello test package\n\n%ARCH%\n{}\n\n%FILENAME%\n{filename}\n",
        std::env::consts::ARCH
    );

    let gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    let mut builder = tar::Builder::new(gz);
    let mut header = tar::Header::new_gnu();
    header.set_size(desc.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    builder
        .append_data(
            &mut header,
            format!("{name}-{version}/desc"),
            desc.as_bytes(),
        )
        .expect("append db entry");
    let gz = builder.into_inner().expect("finish tar");
    gz.finish().expect("finish gzip")
}

// ── Test ────────────────────────────────────────────────────────────────────

#[test]
fn sync_install_upgrade_with_pacnew_and_generations() {
    let server = TestServer::start();
    let fixture = Fixture::new(&server.base_url);

    let pkg_v1 = format!("hello-1.0-1-{}.xp", fixture.arch);
    let pkg_v2 = format!("hello-2.0-1-{}.xp", fixture.arch);
    let db_url = format!("/x/{}/test.db", fixture.arch);
    let pkg1_url = format!("/x/{}/{pkg_v1}", fixture.arch);
    let pkg2_url = format!("/x/{}/{pkg_v2}", fixture.arch);

    // ── Install v1 ──────────────────────────────────────────────────────
    fixture.set_generation("0001");
    server.set(&db_url, build_db("hello", "1.0-1"));
    server.set(
        &pkg1_url,
        build_package("hello", "1.0-1", "#!/bin/sh\necho v1\n", "config v1"),
    );

    let sync = fixture.run(&["sync"]);
    assert!(
        sync.status.success(),
        "sync failed: {}",
        String::from_utf8_lossy(&sync.stderr)
    );

    let install = fixture.run(&["install", "hello"]);
    assert!(
        install.status.success(),
        "install failed: {}\n--- sync stdout ---\n{}\n--- sync stderr ---\n{}",
        String::from_utf8_lossy(&install.stderr),
        String::from_utf8_lossy(&sync.stdout),
        String::from_utf8_lossy(&sync.stderr)
    );
    assert_eq!(fixture.read("usr/bin/hello"), "#!/bin/sh\necho v1\n");
    assert_eq!(fixture.read("etc/hello.conf"), "config v1");

    // The install journal is linked to the current generation.
    let history = fixture.run(&["history", "--json"]);
    let history_out = String::from_utf8_lossy(&history.stdout);
    assert!(
        history_out.contains("\"action\":\"install\"")
            && history_out.contains("\"generation\":\"0001\""),
        "install journal must link generation 0001: {history_out}"
    );

    // ── User modifies the configuration file ────────────────────────────
    fs::write(fixture.root.join("etc/hello.conf"), "user tuned").expect("edit config");

    // ── Upgrade to v2 ───────────────────────────────────────────────────
    fixture.set_generation("0002");
    server.set(&db_url, build_db("hello", "2.0-1"));
    server.set(
        &pkg2_url,
        build_package("hello", "2.0-1", "#!/bin/sh\necho v2\n", "config v2"),
    );

    let upgrade = fixture.run(&["upgrade"]);
    assert!(
        upgrade.status.success(),
        "upgrade failed: {}",
        String::from_utf8_lossy(&upgrade.stderr)
    );

    // Binary replaced, modified config preserved with a .pacnew next to it.
    assert_eq!(
        fixture.read("usr/bin/hello"),
        "#!/bin/sh\necho v2\n",
        "binary must be upgraded"
    );
    assert_eq!(
        fixture.read("etc/hello.conf"),
        "user tuned",
        "modified config must be preserved"
    );
    assert_eq!(
        fixture.read("etc/hello.conf.pacnew"),
        "config v2",
        "new config version must land as .pacnew"
    );

    // Newest journal is the upgrade, linked to generation 0002.
    let history = fixture.run(&["history", "--json"]);
    let newest = String::from_utf8_lossy(&history.stdout)
        .lines()
        .next()
        .unwrap_or_default()
        .to_string();
    assert!(
        newest.contains("\"action\":\"upgrade\"")
            && newest.contains("\"from\":\"1.0-1\"")
            && newest.contains("\"to\":\"2.0-1\"")
            && newest.contains("\"generation\":\"0002\""),
        "upgrade journal mismatch: {newest}"
    );

    // ── Remove keeps the modified config as .pacsave ────────────────────
    let remove = fixture.run(&["remove", "hello"]);
    assert!(
        remove.status.success(),
        "remove failed: {}",
        String::from_utf8_lossy(&remove.stderr)
    );
    assert!(!fixture.root.join("usr/bin/hello").exists());
    assert_eq!(
        fixture.read("etc/hello.conf.pacsave"),
        "user tuned",
        "modified config must survive removal as .pacsave"
    );

    // The local database no longer tracks the package.
    assert!(!fixture.db_path.join("local/hello").exists());
}
