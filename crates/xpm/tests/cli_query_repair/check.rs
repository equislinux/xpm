use super::*;

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
fn check_repair_rebuilds_corrupted_manifest() {
    let fixture = Fixture::new();
    fixture.add_package("systemd", "262.1-1");
    let empty_hash = sha256_hex(b"");
    fixture.write_mtree(
        "systemd",
        &format!(
            "#mtree\n\
             ./usr type=dir mode=0755 uid=0 gid=0\n\
             ./usr/lib type=dir mode=0755 uid=0 gid=0\n\
             ./usr/lib/system-systemd\\134x2dcryptsetup.slice type=file mode=0644 size=0 sha256digest={empty_hash} uid=0 gid=0\n"
        ),
    );
    fixture.write_files(
        "systemd",
        &[
            "usr/",
            "usr/lib/",
            "usr/lib/system-systemd/x2dcryptsetup.slice",
        ],
    );

    let repaired = fixture.run(&["check", "--repair", "systemd"]);
    assert!(repaired.status.success(), "{}", stderr(&repaired));
    let out = stdout(&repaired);
    assert!(out.contains("repaired systemd"), "{out}");

    let files =
        fs::read_to_string(fixture.local_db().join("systemd").join("files")).expect("read files");
    assert!(
        files.contains("usr/lib/system-systemd\\x2dcryptsetup.slice"),
        "{files}"
    );
    assert!(
        !files.contains("usr/lib/system-systemd/x2dcryptsetup.slice"),
        "{files}"
    );
}
