//! Scale/stress coverage for the repository database (roadmap #38).
//!
//! Builds a synthetic repository of thousands of packages in memory, parses it
//! through the public API and asserts that both correctness and a generous
//! time budget hold. The parser must stay linear: a full Arch-scale repository
//! (~15k entries) is parsed in well under a second on current hardware.

use std::path::Path;
use std::time::{Duration, Instant};

const PACKAGE_COUNT: usize = 3000;

fn build_large_db() -> Vec<u8> {
    let gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    let mut builder = tar::Builder::new(gz);

    for index in 0..PACKAGE_COUNT {
        let name = format!("pkg{index:05}");
        let version = format!("{}.0-1", index % 50 + 1);
        let deps = if index == 0 {
            String::new()
        } else {
            format!("%DEPENDS%\npkg{:05}\n\n", index - 1)
        };
        let desc = format!(
            "%NAME%\n{name}\n\n%VERSION%\n{version}\n\n%DESC%\nsynthetic package {index}\n\n%ARCH%\nx86_64\n\n%FILENAME%\n{name}-{version}-x86_64.pkg.tar.zst\n"
        );
        // desc entry
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
            .expect("append desc");

        if !deps.is_empty() {
            let mut header = tar::Header::new_gnu();
            header.set_size(deps.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder
                .append_data(
                    &mut header,
                    format!("{name}-{version}/depends"),
                    deps.as_bytes(),
                )
                .expect("append depends");
        }
    }

    let gz = builder.into_inner().expect("finish tar");
    gz.finish().expect("finish gzip")
}

#[test]
fn parses_thousands_of_packages_quickly() {
    let bytes = build_large_db();
    let tmp = tempfile::TempDir::new().expect("tmp");
    let db_path = tmp.path().join("stress.db");
    std::fs::write(&db_path, &bytes).expect("write db");

    let started = Instant::now();
    let db = xpm_core::repo_db::parse_sync_db(&db_path, "stress").expect("parse large db");
    let elapsed = started.elapsed();

    assert_eq!(db.entries.len(), PACKAGE_COUNT);
    assert_eq!(db.entries[0].name, "pkg00000");
    assert_eq!(
        db.entries[PACKAGE_COUNT - 1].name,
        format!("pkg{:05}", PACKAGE_COUNT - 1)
    );
    assert_eq!(
        db.entries[1].depends,
        vec!["pkg00000".to_string()],
        "dependency metadata must survive at scale"
    );

    eprintln!(
        "parsed {PACKAGE_COUNT} packages in {elapsed:?} ({} bytes db)",
        bytes.len()
    );
    assert!(
        elapsed < Duration::from_secs(10),
        "large database parse took too long: {elapsed:?}"
    );
}

#[test]
fn merge_files_db_stays_linear() {
    // Build a .files archive matching a subset of the same synthetic repo.
    let gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    let mut builder = tar::Builder::new(gz);
    for index in 0..PACKAGE_COUNT {
        let name = format!("pkg{index:05}");
        let version = format!("{}.0-1", index % 50 + 1);
        let files = format!("%FILES%\nusr/\nusr/bin/{name}\n");
        let mut header = tar::Header::new_gnu();
        header.set_size(files.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder
            .append_data(
                &mut header,
                format!("{name}-{version}/files"),
                files.as_bytes(),
            )
            .expect("append files");
    }
    let gz = builder.into_inner().expect("finish tar");
    let files_bytes = gz.finish().expect("finish gzip");

    let db_bytes = build_large_db();
    let tmp = tempfile::TempDir::new().expect("tmp");
    let db_path = tmp.path().join("stress.db");
    let files_path = tmp.path().join("stress.files");
    std::fs::write(&db_path, &db_bytes).expect("write db");
    std::fs::write(&files_path, &files_bytes).expect("write files");

    let mut db = xpm_core::repo_db::parse_sync_db(&db_path, "stress").expect("parse");
    let started = Instant::now();
    xpm_core::repo_db::merge_files_db(Path::new(&files_path), &mut db).expect("merge files");
    let elapsed = started.elapsed();

    let with_files = db.entries.iter().filter(|e| !e.files.is_empty()).count();
    assert_eq!(with_files, PACKAGE_COUNT);
    eprintln!("merged {PACKAGE_COUNT} file lists in {elapsed:?}");
    assert!(
        elapsed < Duration::from_secs(10),
        "merge took too long: {elapsed:?}"
    );
}
