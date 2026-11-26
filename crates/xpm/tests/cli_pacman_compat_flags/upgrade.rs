use super::*;

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
#[test]
fn upgrade_allows_a_file_to_move_to_another_package() {
    let fixture = Fixture::new();
    fixture.publish(
        "core",
        &[Entry::new("openssl", "1.0-1").with_files(&["usr/lib/libcrypto.so.3"])],
    );
    assert_success(&fixture.run(&["sync"]));
    assert_success(&fixture.run(&["install", "openssl"]));
    assert!(fixture.root.join("usr/lib/libcrypto.so.3").exists());

    fixture.publish(
        "core",
        &[
            Entry::new("openssl", "2.0-1")
                .with_files(&["usr/lib/libcrypto.so.4"])
                .with_depends(&["openssl-compat"]),
            Entry::new("openssl-compat", "1.0-1").with_files(&["usr/lib/libcrypto.so.3"]),
        ],
    );

    let upgrade = fixture.run(&["upgrade"]);
    assert_success(&upgrade);

    let moved = fixture.root.join("usr/lib/libcrypto.so.3");
    assert!(moved.exists(), "the moved file must survive the upgrade");
    assert_eq!(
        fs::read_to_string(&moved).expect("read moved file").trim(),
        "openssl-compat"
    );

    let owner = fixture.run(&["-Qo", "/usr/lib/libcrypto.so.3"]);
    assert_success(&owner);
    assert!(
        stdout(&owner).contains("owned by openssl-compat 1.0-1"),
        "{}",
        stdout(&owner)
    );
}
