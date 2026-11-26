use super::*;

fn options() -> BuildOptions {
    BuildOptions::new("/tmp/xpm-test-packages")
}

#[test]
fn makepkg_line_adds_nodeps_and_quotes_extra_args() {
    let mut options = options();
    options.chroot_nodeps = true;
    options.run_checks = false;
    options.extra_args = vec!["--config".into(), "/tmp/a b.conf".into()];
    let line = makepkg_line(&options);
    assert!(line.contains("--nodeps"));
    assert!(line.contains("--nocheck"));
    assert!(line.contains("'/tmp/a b.conf'"));
}

#[test]
fn makepkg_environment_forces_pkgext_and_forwards_caches() {
    let mut options = options();
    let env = makepkg_environment(&options);
    assert!(env.contains("PKGEXT=.pkg.tar.zst"));
    assert!(!env.contains("SRCDEST=/srcdest"));
    assert!(!env.contains("BUILDDIR=/builddir"));

    options.srcdest = Some("/tmp/sources".into());
    options.builddir = Some("/tmp/build".into());
    let env = makepkg_environment(&options);
    assert!(env.contains("SRCDEST=/srcdest"));
    assert!(env.contains("BUILDDIR=/builddir"));
}

#[test]
fn copy_kind_round_trips() {
    for kind in [
        CopyKind::Snapshot,
        CopyKind::Reflink,
        CopyKind::Overlay,
        CopyKind::Copy,
    ] {
        assert_eq!(CopyKind::parse(kind.as_str()), Some(kind));
    }
    assert_eq!(CopyKind::parse("other"), None);
}

#[test]
fn template_freshness_honors_ttl_refresh_and_arch() {
    let dir = std::env::temp_dir().join(format!(
        "xpm-template-test-{}-{}",
        std::process::id(),
        now_epoch_nanos()
    ));
    let root = dir.join("root");
    std::fs::create_dir_all(root.join("usr/bin")).unwrap();
    std::fs::create_dir_all(root.join("var/lib/xpm/local")).unwrap();
    std::fs::create_dir_all(root.join("etc")).unwrap();
    std::fs::write(root.join("usr/bin/bash"), b"").unwrap();
    std::fs::write(root.join("etc/xpm.conf"), b"").unwrap();

    let stamp_path = dir.join("template.json");
    let stamp = TemplateStamp::new(&current_arch(), now_epoch());
    write_stamp(&stamp_path, &stamp).unwrap();

    let mut options = options();
    assert_eq!(
        template_action(&root, &stamp_path, &options),
        TemplateAction::Fresh
    );

    options.chroot_refresh = true;
    assert_ne!(
        template_action(&root, &stamp_path, &options),
        TemplateAction::Fresh
    );

    options.chroot_refresh = false;
    options.chroot_ttl_days = 0;
    assert_ne!(
        template_action(&root, &stamp_path, &options),
        TemplateAction::Fresh
    );

    options.chroot_ttl_days = 7;
    let stale = TemplateStamp {
        refreshed_at: now_epoch().saturating_sub(8 * SECONDS_PER_DAY),
        ..stamp
    };
    write_stamp(&stamp_path, &stale).unwrap();
    assert_ne!(
        template_action(&root, &stamp_path, &options),
        TemplateAction::Fresh
    );

    let wrong_arch = TemplateStamp::new("not-this-arch", now_epoch());
    write_stamp(&stamp_path, &wrong_arch).unwrap();
    options.chroot_ttl_days = 7;
    assert_eq!(
        template_action(&root, &stamp_path, &options),
        TemplateAction::Recreate
    );

    let changed_config = TemplateStamp {
        config_hash: "different".into(),
        ..TemplateStamp::new(&current_arch(), now_epoch())
    };
    write_stamp(&stamp_path, &changed_config).unwrap();
    assert_eq!(
        template_action(&root, &stamp_path, &options),
        TemplateAction::Recreate
    );

    let _ = std::fs::remove_dir_all(dir);
}
