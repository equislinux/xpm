use std::fs;
use std::path::{Path, PathBuf};

use tempfile::TempDir;

use super::*;

fn write(dir: &Path, name: &str, contents: &str) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, contents).unwrap();
    path
}

fn parse(path: &Path) -> PacmanConfig {
    parse_pacman_conf(path).unwrap()
}

fn repo<'a>(config: &'a PacmanConfig, name: &str) -> &'a PacmanRepo {
    config
        .repositories
        .iter()
        .find(|repo| repo.name == name)
        .unwrap()
}

#[test]
fn parses_options_repositories_and_includes() {
    let dir = TempDir::new().unwrap();
    let mirror = write(
        dir.path(),
        "mirrorlist",
        "Server = https://mirror.example/$repo/$arch\n\
             Server = https://backup.example/$repo\n",
    );
    let conf = write(
        dir.path(),
        "pacman.conf",
        &format!(
            "[options]\n\
                 Architecture = x86_64\n\
                 CacheDir = /var/cache/pacman/pkg/\n\
                 Color\n\
                 SigLevel = Required DatabaseOptional\n\
                 ParallelDownloads = 5\n\
                 \n\
                 [x]\n\
                 Server = file:///srv/x/$repo\n\
                 SigLevel = PackageRequired\n\
                 \n\
                 [core]\n\
                 Include = {}\n\
                 Usage = Sync Search\n\
                 \n\
                 [extra]\n\
                 Server = https://extra.example/$repo\n",
            mirror.display()
        ),
    );

    let config = parse(&conf);

    assert_eq!(
        config
            .repositories
            .iter()
            .map(|repo| repo.name.as_str())
            .collect::<Vec<_>>(),
        vec!["x", "core", "extra"]
    );
    assert_eq!(
        config.options.cache_dirs,
        vec![PathBuf::from("/var/cache/pacman/pkg/")]
    );
    assert_eq!(config.options.parallel_downloads, 5);
    assert_eq!(config.options.color, ColorMode::Auto);
    assert_eq!(config.options.architectures, vec!["x86_64"]);
    assert_eq!(config.options.clean_method, CleanMethod::KeepInstalled);
    assert_eq!(config.sources, vec![conf.clone(), mirror.clone()]);
    assert!(config.warnings.is_empty());

    let x = repo(&config, "x");
    assert_eq!(x.servers, vec!["file:///srv/x/$repo"]);
    assert_eq!(x.usage, Usage::ALL);
    assert_eq!(x.source.file, conf);
    assert_eq!(x.source.line, 8);
    assert_eq!(
        x.effective_sig_level(&config.options.sig_level)
            .package_check(),
        SigCheck::Required
    );
    assert_eq!(
        x.effective_sig_level(&config.options.sig_level)
            .database_check(),
        SigCheck::Optional
    );

    let core = repo(&config, "core");
    assert_eq!(
        core.servers,
        vec![
            "https://mirror.example/$repo/$arch",
            "https://backup.example/$repo"
        ]
    );
    assert_eq!(core.usage, Usage::SYNC | Usage::SEARCH);
    assert_eq!(core.sig_level, None);
    assert_eq!(
        core.effective_sig_level(&config.options.sig_level),
        config.options.sig_level
    );

    let extra = repo(&config, "extra");
    assert_eq!(extra.servers, vec!["https://extra.example/$repo"]);
    assert!(config.validate().is_ok());
}

#[test]
fn hashtags_are_not_inline_comments() {
    let dir = TempDir::new().unwrap();
    let conf = write(
        dir.path(),
        "pacman.conf",
        "[options]\n\
             RootDir = /root#not-a-comment\n\
             # a real comment\n\
             [core]\n\
             Server = https://example.invalid/A#B\n",
    );

    let config = parse(&conf);

    assert_eq!(
        config.options.root_dir,
        PathBuf::from("/root#not-a-comment")
    );
    assert_eq!(
        repo(&config, "core").servers,
        vec!["https://example.invalid/A#B"]
    );
}

#[test]
fn includes_share_section_state_and_recurse() {
    let dir = TempDir::new().unwrap();
    let mirror = write(
        dir.path(),
        "mirrorlist",
        "Server = https://one.example/$repo\n",
    );
    let middle = write(
        dir.path(),
        "middle",
        &format!(
            "Include = {}\nCacheServer = https://cache.example/$repo\n",
            mirror.display()
        ),
    );
    let conf = write(
        dir.path(),
        "pacman.conf",
        &format!("[core]\nInclude = {}\n", middle.display()),
    );

    let config = parse(&conf);
    let core = repo(&config, "core");

    assert_eq!(core.servers, vec!["https://one.example/$repo"]);
    assert_eq!(core.cache_servers, vec!["https://cache.example/$repo"]);
    assert_eq!(config.sources, vec![conf, middle, mirror]);
}

#[test]
fn included_section_changes_parent_state() {
    let dir = TempDir::new().unwrap();
    let extra = write(
        dir.path(),
        "extra.conf",
        "[extra]\nServer = https://inner.example/$repo\n",
    );
    let conf = write(
        dir.path(),
        "pacman.conf",
        &format!(
            "[core]\nInclude = {}\nServer = https://outer.example/$repo\n",
            extra.display()
        ),
    );

    let config = parse(&conf);

    assert!(repo(&config, "core").servers.is_empty());
    assert_eq!(
        repo(&config, "extra").servers,
        vec!["https://inner.example/$repo", "https://outer.example/$repo"]
    );
}

#[test]
fn siglevel_matrix_and_mask_merge() {
    let dir = TempDir::new().unwrap();
    let conf = write(
        dir.path(),
        "pacman.conf",
        "[options]\n\
             SigLevel = Required DatabaseOptional\n\
             [plain]\n\
             Server = https://plain.example/$repo\n\
             [trustall]\n\
             Server = https://trust.example/$repo\n\
             SigLevel = PackageTrustAll\n\
             [dbnever]\n\
             Server = https://db.example/$repo\n\
             SigLevel = DatabaseNever\n\
             [pkgoptional]\n\
             Server = https://opt.example/$repo\n\
             SigLevel = PackageOptional\n",
    );

    let config = parse(&conf);
    let global = config.options.sig_level;

    assert_eq!(global.package_check(), SigCheck::Required);
    assert_eq!(global.database_check(), SigCheck::Optional);
    assert_eq!(global.package_trust(), SigTrust::TrustedOnly);
    assert_eq!(global.database_trust(), SigTrust::TrustedOnly);

    let plain = repo(&config, "plain").effective_sig_level(&global);
    assert_eq!(plain, global);

    let trustall = repo(&config, "trustall").effective_sig_level(&global);
    assert_eq!(trustall.package_check(), SigCheck::Required);
    assert_eq!(trustall.database_check(), SigCheck::Optional);
    assert_eq!(trustall.package_trust(), SigTrust::TrustAll);
    assert_eq!(trustall.database_trust(), SigTrust::TrustedOnly);
    assert!(trustall.package_allows_marginal());
    assert!(trustall.package_allows_unknown());
    assert!(!trustall.database_allows_marginal());
    assert!(!trustall.database_allows_unknown());

    let dbnever = repo(&config, "dbnever").effective_sig_level(&global);
    assert_eq!(dbnever.package_check(), SigCheck::Required);
    assert_eq!(dbnever.database_check(), SigCheck::Never);
    assert_eq!(dbnever.database_trust(), SigTrust::TrustedOnly);

    let pkgoptional = repo(&config, "pkgoptional").effective_sig_level(&global);
    assert_eq!(pkgoptional.package_check(), SigCheck::Optional);
    assert_eq!(pkgoptional.database_check(), SigCheck::Optional);
}

#[test]
fn siglevel_tokens_apply_left_to_right() {
    let dir = TempDir::new().unwrap();
    let conf = write(
        dir.path(),
        "pacman.conf",
        "[options]\n\
             SigLevel = Required Never\n\
             [a]\n\
             SigLevel = Never Required DatabaseOptional\n\
             [b]\n\
             SigLevel = PackageTrustAll DatabaseNever\n\
             [c]\n\
             SigLevel = DatabaseOptional PackageRequired\n",
    );

    let config = parse(&conf);
    assert_eq!(config.options.sig_level.package_check(), SigCheck::Never);
    assert_eq!(config.options.sig_level.database_check(), SigCheck::Never);

    let a = repo(&config, "a").effective_sig_level(&config.options.sig_level);
    assert_eq!(a.package_check(), SigCheck::Required);
    assert_eq!(a.database_check(), SigCheck::Optional);

    let b = repo(&config, "b").effective_sig_level(&config.options.sig_level);
    assert_eq!(b.package_check(), SigCheck::Never);
    assert_eq!(b.package_trust(), SigTrust::TrustAll);
    assert_eq!(b.database_check(), SigCheck::Never);

    let c = repo(&config, "c").effective_sig_level(&config.options.sig_level);
    assert_eq!(c.package_check(), SigCheck::Required);
    assert_eq!(c.database_check(), SigCheck::Optional);
}

#[test]
fn architecture_lists_and_auto_are_literal() {
    let dir = TempDir::new().unwrap();
    let conf = write(
        dir.path(),
        "pacman.conf",
        "[options]\n\
             Architecture = x86_64\n\
             Architecture = auto aarch64\n",
    );

    let config = parse(&conf);

    assert_eq!(
        config.options.architectures,
        vec!["x86_64", "auto", "aarch64"]
    );
}

#[test]
fn pattern_lists_append_and_preserve_inversion() {
    let dir = TempDir::new().unwrap();
    let conf = write(
        dir.path(),
        "pacman.conf",
        "[options]\n\
             IgnorePkg = linux linux-lts\n\
             IgnorePkg = foo\n\
             NoUpgrade = etc/* !etc/foo.conf\n\
             NoUpgrade = boot/*\n\
             NoExtract = !usr/share/*\n\
             HoldPkg = pacman xpm\n\
             IgnoreGroup = xfce\n",
    );

    let config = parse(&conf);

    assert_eq!(config.options.ignore_pkg, vec!["linux", "linux-lts", "foo"]);
    assert_eq!(
        config.options.no_upgrade,
        vec!["etc/*", "!etc/foo.conf", "boot/*"]
    );
    assert_eq!(config.options.no_extract, vec!["!usr/share/*"]);
    assert_eq!(config.options.hold_pkg, vec!["pacman", "xpm"]);
    assert_eq!(config.options.ignore_group, vec!["xfce"]);
}

#[test]
fn duplicate_repositories_are_first_wins_with_warning() {
    let dir = TempDir::new().unwrap();
    let conf = write(
        dir.path(),
        "pacman.conf",
        "[core]\n\
             Server = https://one.example/$repo\n\
             [extra]\n\
             Server = https://extra.example/$repo\n\
             [core]\n\
             Server = https://two.example/$repo\n",
    );

    let config = parse(&conf);

    assert_eq!(
        config
            .repositories
            .iter()
            .map(|repo| repo.name.as_str())
            .collect::<Vec<_>>(),
        vec!["core", "extra"]
    );
    assert_eq!(
        repo(&config, "core").servers,
        vec!["https://one.example/$repo"]
    );
    assert_eq!(config.warnings.len(), 1);
    assert!(config.warnings[0].contains("already defined"));
    assert!(config.warnings[0].contains(&format!("{}, line 5", conf.display())));
}

#[test]
fn unknown_directives_warn_and_invalid_values_fail_with_location() {
    let dir = TempDir::new().unwrap();
    let unknown = write(
        dir.path(),
        "unknown.conf",
        "[options]\n\
             Frobnicate = yes\n\
             [core]\n\
             Mystery = value\n",
    );
    let config = parse(&unknown);
    assert_eq!(config.warnings.len(), 2);
    assert!(config.warnings[0].contains("Frobnicate"));
    assert!(config.warnings[0].contains(&format!("{}, line 2", unknown.display())));
    assert!(config.warnings[1].contains("Mystery"));
    assert!(config.warnings[1].contains("core"));

    let bad_sig = write(
        dir.path(),
        "bad-sig.conf",
        "[options]\n\
             SigLevel = PackageNope\n",
    );
    let error = parse_pacman_conf(&bad_sig).unwrap_err().to_string();
    assert!(error.contains(&format!("{}, line 2", bad_sig.display())));
    assert!(error.contains("PackageNope"));

    let bad_parallel = write(
        dir.path(),
        "bad-parallel.conf",
        "[options]\n\
             ParallelDownloads = 0\n",
    );
    let error = parse_pacman_conf(&bad_parallel).unwrap_err().to_string();
    assert!(error.contains(&format!("{}, line 2", bad_parallel.display())));
    assert!(error.contains("ParallelDownloads"));

    let bad_usage = write(
        dir.path(),
        "bad-usage.conf",
        "[options]\n\
             SigLevel = Required\n\
             [core]\n\
             Usage = Search Nope\n",
    );
    let error = parse_pacman_conf(&bad_usage).unwrap_err().to_string();
    assert!(error.contains(&format!("{}, line 4", bad_usage.display())));
    assert!(error.contains("Nope"));
}

#[test]
fn missing_include_is_fatal_and_glob_order_is_sorted() {
    let dir = TempDir::new().unwrap();
    let missing = dir.path().join("does-not-exist");
    let conf = write(
        dir.path(),
        "missing.conf",
        &format!("[core]\nInclude = {}\n", missing.display()),
    );

    let error = parse_pacman_conf(&conf).unwrap_err().to_string();
    assert!(error.contains(&format!("{}, line 2", conf.display())));
    assert!(error.contains("could not read include"));
    assert!(error.contains("does-not-exist"));

    let includes = dir.path().join("conf.d");
    fs::create_dir(&includes).unwrap();
    write(
        &includes,
        "20-second.conf",
        "Server = https://second.example/$repo\n",
    );
    write(
        &includes,
        "10-first.conf",
        "Server = https://first.example/$repo\n",
    );
    let glob_conf = write(
        dir.path(),
        "glob.conf",
        &format!("[core]\nInclude = {}\n", includes.join("*.conf").display()),
    );

    let config = parse(&glob_conf);
    assert_eq!(
        repo(&config, "core").servers,
        vec![
            "https://first.example/$repo",
            "https://second.example/$repo"
        ]
    );
}

#[test]
fn include_depth_is_limited() {
    let dir = TempDir::new().unwrap();
    let mut previous = write(
        dir.path(),
        "leaf.conf",
        "[core]\nServer = https://leaf/$repo\n",
    );
    for index in 0..11 {
        let name = format!("level{index}.conf");
        previous = write(
            dir.path(),
            &name,
            &format!("Include = {}\n", previous.display()),
        );
    }

    let error = parse_pacman_conf(&previous).unwrap_err().to_string();
    assert!(error.contains("max recursion depth"));
}

#[test]
fn server_variables_are_preserved_and_arch_is_validated() {
    let dir = TempDir::new().unwrap();
    let with_arch = write(
        dir.path(),
        "with-arch.conf",
        "[options]\n\
             Architecture = x86_64\n\
             [core]\n\
             Server = https://mirror.example/$repo/$arch\n\
             CacheServer = https://cache.example/$repo/$arch\n",
    );
    let config = parse(&with_arch);
    assert_eq!(
        repo(&config, "core").servers,
        vec!["https://mirror.example/$repo/$arch"]
    );
    assert_eq!(
        repo(&config, "core").cache_servers,
        vec!["https://cache.example/$repo/$arch"]
    );
    assert!(config.validate().is_ok());

    let without_arch = write(
        dir.path(),
        "without-arch.conf",
        "[core]\n\
             Server = https://mirror.example/$repo\n\
             CacheServer = https://cache.example/$arch\n",
    );
    let config = parse(&without_arch);
    assert_eq!(
        repo(&config, "core").servers,
        vec!["https://mirror.example/$repo"]
    );
    let error = config.validate().unwrap_err().to_string();
    assert!(error.contains("$arch"));
    assert!(error.contains("Architecture"));
    assert!(error.contains(&format!("{}, line 1", without_arch.display())));
}

#[test]
fn defaults_root_db_and_options_follow_pacman() {
    let dir = TempDir::new().unwrap();
    let conf = write(
        dir.path(),
        "pacman.conf",
        "[options]\n\
             RootDir = /mnt/root\n\
             CacheDir = /cache/one\n\
             CacheDir = /cache/two\n\
             HookDir = /hooks/one\n",
    );

    let config = parse(&conf);
    let options = &config.options;

    assert_eq!(options.root_dir, PathBuf::from("/mnt/root"));
    assert_eq!(options.db_path, PathBuf::from("/mnt/root/var/lib/pacman/"));
    assert_eq!(
        options.log_file,
        PathBuf::from("/mnt/root/var/log/pacman.log")
    );
    assert_eq!(
        options.cache_dirs,
        vec![PathBuf::from("/cache/one"), PathBuf::from("/cache/two")]
    );
    assert_eq!(options.hook_dirs, vec![PathBuf::from("/hooks/one")]);
    assert_eq!(options.parallel_downloads, 1);
    assert_eq!(options.color, ColorMode::Auto);
    assert_eq!(options.clean_method, CleanMethod::KeepInstalled);
    assert_eq!(options.effective_local_file_sig_level(), options.sig_level);
    assert_eq!(options.effective_remote_file_sig_level(), options.sig_level);
}

#[test]
fn directive_before_section_is_fatal_and_flags_parse() {
    let dir = TempDir::new().unwrap();
    let bad = write(dir.path(), "bad.conf", "RootDir = /\n");
    let error = parse_pacman_conf(&bad).unwrap_err().to_string();
    assert!(error.contains(&format!("{}, line 1", bad.display())));
    assert!(error.contains("must belong to a section"));

    let flags = write(
        dir.path(),
        "flags.conf",
        "[options]\n\
             UseSyslog\n\
             CheckSpace\n\
             VerbosePkgLists\n\
             NoProgressBar\n\
             DisableDownloadTimeout\n\
             ILoveCandy\n\
             DisableSandbox\n\
             XferCommand = /usr/bin/curl %u -o %o\n\
             DownloadUser = alpm\n\
             CleanMethod = KeepCurrent\n",
    );
    let options = parse(&flags).options;
    assert!(options.use_syslog);
    assert!(options.check_space);
    assert!(options.verbose_pkg_lists);
    assert!(options.no_progress_bar);
    assert!(options.disable_download_timeout);
    assert!(options.ilovecandy);
    assert!(options.disable_sandbox_filesystem);
    assert!(options.disable_sandbox_syscalls);
    assert_eq!(
        options.xfer_command.as_deref(),
        Some("/usr/bin/curl %u -o %o")
    );
    assert_eq!(options.download_user.as_deref(), Some("alpm"));
    assert_eq!(options.clean_method, CleanMethod::KeepCurrent);
}
