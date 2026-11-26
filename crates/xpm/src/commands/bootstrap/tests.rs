use super::*;

#[test]
fn copies_public_keyring_material_only() {
    let tmp = tempfile::TempDir::new().expect("tmp");
    let host = tmp.path().join("host-gnupg");
    let target = tmp.path().join("root/etc/xpm/gnupg");
    fs::create_dir_all(&host).expect("host");
    for name in [
        "pubring.gpg",
        "pubring.kbx",
        "trustdb.gpg",
        "trustedkeys.gpg",
        "gpg.conf",
        "common.conf",
    ] {
        fs::write(host.join(name), name.as_bytes()).expect("keyring file");
    }
    fs::create_dir_all(host.join("private-keys-v1.d")).expect("private dir");
    fs::write(host.join("private-keys-v1.d/secret.key"), b"secret").expect("private key");

    copy_public_keyring(&host, &target).expect("copy");

    for name in [
        "pubring.gpg",
        "pubring.kbx",
        "trustdb.gpg",
        "trustedkeys.gpg",
        "gpg.conf",
        "common.conf",
    ] {
        assert!(target.join(name).is_file(), "missing {name}");
    }
    assert!(!target.join("private-keys-v1.d").exists());
    assert!(!target.join("secret.key").exists());
}

#[test]
fn bootstrap_uses_the_host_keyring_and_target_paths() {
    let mut config = XpmConfig::default();
    config.options.gpg_dir = PathBuf::from("/tmp/target/etc/pacman.d/gnupg");
    let host_gpg = Path::new("/etc/pacman.d/gnupg");
    let root = Path::new("/tmp/target-root");
    let boot = bootstrap_config(&config, root, host_gpg);
    assert_eq!(boot.options.gpg_dir, host_gpg);
    assert_eq!(boot.options.root_dir, root);
    assert_eq!(boot.options.db_path, root.join("var/lib/xpm"));
    assert_eq!(boot.options.cache_dir, root.join("var/cache/xpm/pkg"));
    assert!(!boot.options.alpm_hooks);
}

#[test]
fn missing_host_keyring_is_a_noop() {
    let tmp = tempfile::TempDir::new().expect("tmp");
    let target = tmp.path().join("root/etc/xpm/gnupg");
    copy_public_keyring(&tmp.path().join("missing"), &target).expect("noop");
    assert!(!target.exists());
}
