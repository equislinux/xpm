use super::*;
use std::io::Write;

#[test]
fn test_default_config() {
    let config = XpmConfig::default();
    assert_eq!(config.options.root_dir, PathBuf::from("/"));
    assert_eq!(config.options.db_path, PathBuf::from("/var/lib/xpm/"));
    assert_eq!(config.options.parallel_downloads, 10);
    assert_eq!(config.options.gpg_dir, PathBuf::from("/etc/xpm/gnupg/"));
    assert!(config.options.alpm_hooks);
    assert_eq!(config.repositories.len(), 1);
    assert_eq!(config.repositories[0].name, "x");
}

#[test]
fn test_load_from_toml() {
    let toml_content = r#"
[options]
root_dir = "/"
db_path = "/var/lib/xpm/"
cache_dir = "/var/cache/xpm/pkg/"
log_file = "/var/log/xpm.log"
gpg_dir = "/etc/pacman.d/gnupg/"
color = true
parallel_downloads = 3
check_space = true

# alpm_hooks = true          # false skips pacman-style .hook execution
# shell_integration = false
# scriptlet_mode = "auto"   # auto | skip | require

[[repo]]
name = "core"
server = ["https://mirror.example.com/archlinux/core/os/x86_64"]

[[repo]]
name = "extra"
server = ["https://mirror.example.com/archlinux/extra/os/x86_64"]
"#;

    let mut tmpfile = tempfile::NamedTempFile::new().unwrap();
    tmpfile.write_all(toml_content.as_bytes()).unwrap();

    let config = XpmConfig::load(tmpfile.path()).unwrap();
    assert_eq!(config.options.parallel_downloads, 3);
    assert_eq!(config.repositories.len(), 2);
    assert_eq!(config.repositories[0].name, "core");
}

#[test]
fn alpm_hooks_can_be_disabled_in_config() {
    let toml_content = r#"
[options]
alpm_hooks = false

[[repo]]
name = "x"
server = ["https://example/$repo/$arch"]
"#;
    let mut tmpfile = tempfile::NamedTempFile::new().unwrap();
    tmpfile.write_all(toml_content.as_bytes()).unwrap();
    let config = XpmConfig::load(tmpfile.path()).unwrap();
    assert!(!config.options.alpm_hooks);
}

#[test]
fn test_load_missing_file_returns_error() {
    let result = XpmConfig::load(Path::new("/nonexistent/xpm.conf"));
    assert!(result.is_err());
}

#[test]
fn test_load_or_default_missing_file() {
    let config = XpmConfig::load_or_default(Path::new("/nonexistent/xpm.conf")).unwrap();
    assert_eq!(config.options.root_dir, PathBuf::from("/"));
}

#[test]
fn test_apply_overrides() {
    let mut config = XpmConfig::default();
    config.apply_overrides(
        Some(Path::new("/mnt/install")),
        Some(Path::new("/custom/db")),
        None,
    );
    assert_eq!(config.options.root_dir, PathBuf::from("/mnt/install"));
    assert_eq!(config.options.db_path, PathBuf::from("/custom/db"));
    // cache_dir should remain default
    assert_eq!(
        config.options.cache_dir,
        PathBuf::from("/var/cache/xpm/pkg/")
    );
}

#[test]
fn from_pacman_maps_repositories_and_split_siglevels() {
    let tmp = tempfile::TempDir::new().unwrap();
    let path = tmp.path().join("pacman.conf");
    std::fs::write(
            &path,
            "[options]\nArchitecture = x86_64\nSigLevel = Required DatabaseOptional\nParallelDownloads = 3\nIgnorePkg = foo\n\n[x]\nSigLevel = Required\nServer = https://example/$repo/$arch\n\n[core]\nServer = https://mirror/$repo/os/$arch\n",
        )
        .unwrap();

    let pacman = crate::pacman_conf::parse_pacman_conf(&path).expect("parse pacman.conf");
    let config = XpmConfig::from_pacman(pacman);

    assert_eq!(config.options.sig_level, SigLevel::Required);
    assert_eq!(config.options.database_sig_level, Some(SigLevel::Optional));
    assert_eq!(config.options.architecture.as_deref(), Some("x86_64"));
    assert_eq!(config.options.parallel_downloads, 3);
    assert_eq!(config.options.ignore_pkg, vec!["foo".to_string()]);
    assert_eq!(config.repositories.len(), 2);
    assert_eq!(config.repositories[0].name, "x");
    assert_eq!(config.repositories[0].sig_level, Some(SigLevel::Required));
    assert_eq!(
        config.repositories[0].database_sig_level,
        Some(SigLevel::Required)
    );
    assert_eq!(
        config.repositories[0].server,
        vec!["https://example/$repo/$arch"]
    );
    assert_eq!(config.repositories[1].name, "core");
    assert_eq!(config.repositories[1].sig_level, Some(SigLevel::Required));
    assert_eq!(
        config.repositories[1].database_sig_level,
        Some(SigLevel::Optional)
    );
}

#[test]
fn keyring_path_prefers_existing_configured_dir() {
    let tmp = tempfile::TempDir::new().unwrap();
    let options = GeneralOptions {
        gpg_dir: tmp.path().to_path_buf(),
        ..Default::default()
    };

    assert_eq!(options.effective_gpg_dir(), tmp.path());
    assert_eq!(options.keyring_path(), tmp.path().join("trustedkeys.gpg"));
    assert_eq!(DEFAULT_GPG_DIR, "/etc/xpm/gnupg/");
    assert_eq!(XPM_GPG_DIR, "/etc/xpm/gnupg/");
    assert_eq!(PACMAN_GPG_DIR, "/etc/pacman.d/gnupg/");
}

#[test]
fn keyring_path_falls_back_to_pubring_gpg() {
    let tmp = tempfile::TempDir::new().unwrap();
    let options = GeneralOptions {
        gpg_dir: tmp.path().to_path_buf(),
        ..Default::default()
    };

    std::fs::write(tmp.path().join("pubring.gpg"), b"not parsed here").unwrap();
    assert_eq!(options.keyring_path(), tmp.path().join("pubring.gpg"));
}

#[test]
fn test_validation_zero_parallel_downloads() {
    let toml_content = r#"
[options]
parallel_downloads = 0

[[repo]]
name = "core"
server = ["https://mirror.example.com/core"]
"#;
    let mut tmpfile = tempfile::NamedTempFile::new().unwrap();
    tmpfile.write_all(toml_content.as_bytes()).unwrap();

    let result = XpmConfig::load(tmpfile.path());
    assert!(result.is_err());
}

#[test]
fn test_validation_empty_repo_name() {
    let toml_content = r#"
[options]
parallel_downloads = 5

[[repo]]
name = ""
server = ["https://mirror.example.com/core"]
"#;
    let mut tmpfile = tempfile::NamedTempFile::new().unwrap();
    tmpfile.write_all(toml_content.as_bytes()).unwrap();

    let result = XpmConfig::load(tmpfile.path());
    assert!(result.is_err());
}
