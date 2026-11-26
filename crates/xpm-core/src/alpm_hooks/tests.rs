use super::*;
use std::os::unix::fs::PermissionsExt;
use tempfile::TempDir;

const GEN_PRE_HOOK: &str = r#"
[Trigger]
Operation = Install
Operation = Upgrade
Operation = Remove
Type = Package
Target = *

[Action]
Description = Record a pre-transaction generation
When = PreTransaction
Exec = /usr/share/x/hooks/pacman-gen.sh pre
Depends = x-scripts
"#;

#[test]
fn parses_generation_pre_hook() {
    let hook = parse_hook("10-x-gen-pre.hook", GEN_PRE_HOOK).expect("parse");
    assert_eq!(hook.triggers.len(), 1);
    assert_eq!(
        hook.triggers[0].operations,
        vec![
            HookOperation::Install,
            HookOperation::Upgrade,
            HookOperation::Remove
        ]
    );
    assert_eq!(hook.triggers[0].trigger_type, TriggerType::Package);
    assert_eq!(hook.triggers[0].targets, vec!["*"]);
    assert_eq!(hook.when, vec![HookWhen::PreTransaction]);
    assert_eq!(hook.exec, "/usr/share/x/hooks/pacman-gen.sh pre");
    assert_eq!(hook.depends, vec!["x-scripts"]);
    assert!(!hook.abort_on_fail);
}

#[test]
fn parses_flags_both_phases_and_multiple_triggers() {
    let content = r#"
[Trigger]
Type = Package
Target = linux*
Target = linux-lts

[Trigger]
Operation = Upgrade
Type = Path
Target = usr/lib/modules

[Action]
When = PreTransaction|PostTransaction
Exec = /usr/bin/mkinitcpio -P
AbortOnFail
NeedsTargets
"#;
    let hook = parse_hook("90-modules.hook", content).expect("parse");
    assert_eq!(hook.triggers.len(), 2);
    assert_eq!(hook.triggers[1].trigger_type, TriggerType::Path);
    assert_eq!(
        hook.when,
        vec![HookWhen::PreTransaction, HookWhen::PostTransaction]
    );
    assert!(hook.abort_on_fail);
    assert!(hook.needs_targets);
}

#[test]
fn rejects_missing_exec_and_unknown_keys() {
    let no_exec = "[Trigger]\nTarget = *\n\n[Action]\nWhen = PreTransaction\n";
    assert!(parse_hook("bad.hook", no_exec).is_err());

    let bad_op = "[Trigger]\nOperation = Dance\nTarget = *\n\n[Action]\nExec = /bin/true\n";
    assert!(parse_hook("bad.hook", bad_op).is_err());
}

#[test]
fn glob_match_supports_star_and_question() {
    assert!(glob_match("*", "anything"));
    assert!(glob_match("linux*", "linux-lts"));
    assert!(glob_match("linux*", "linux"));
    assert!(glob_match("linux*", "linux-firmware-extra"));
    assert!(!glob_match("linux-lts", "linux"));
    assert!(glob_match("linux-?", "linux-a"));
    assert!(!glob_match("linux-?", "linux-ab"));
    assert!(glob_match("a*b*c", "axxbyyc"));
}

#[test]
fn matches_operations_and_targets() {
    let hook = parse_hook("10-x-gen-pre.hook", GEN_PRE_HOOK).expect("parse");

    let install = HookTransaction {
        packages: vec![("kitty", HookOperation::Install)],
        paths: Vec::new(),
    };
    assert!(hook.matches(&install, |_| true));

    let upgrade = HookTransaction {
        packages: vec![("linux", HookOperation::Upgrade)],
        paths: Vec::new(),
    };
    assert!(hook.matches(&upgrade, |_| true));

    // Depends not installed: hook must not run.
    assert!(!hook.matches(&install, |name| name != "x-scripts"));
}

#[test]
fn path_trigger_needs_matching_path() {
    let content = "[Trigger]\nOperation = Upgrade\nType = Path\nTarget = usr/lib/modules/*\n\n[Action]\nExec = /bin/true\n";
    let hook = parse_hook("modules.hook", content).expect("parse");

    let without_paths = HookTransaction {
        packages: vec![("linux", HookOperation::Upgrade)],
        paths: Vec::new(),
    };
    assert!(!hook.matches(&without_paths, |_| true));

    let with_paths = HookTransaction {
        packages: vec![("linux", HookOperation::Upgrade)],
        paths: vec![(
            "usr/lib/modules/6.12/vmlinuz".to_string(),
            HookOperation::Upgrade,
        )],
    };
    assert!(hook.matches(&with_paths, |_| true));
}

#[test]
fn path_trigger_respects_operation_and_glob() {
    let content = "[Trigger]\nOperation = Install\nType = Path\nTarget = usr/lib/modules/*/vmlinuz\n\n[Action]\nExec = /bin/true\nNeedsTargets\n";
    let hook = parse_hook("modules-install.hook", content).expect("parse");
    let upgrade_only = HookTransaction {
        packages: vec![("linux", HookOperation::Upgrade)],
        paths: vec![(
            "usr/lib/modules/6.12/vmlinuz".to_string(),
            HookOperation::Upgrade,
        )],
    };
    assert!(!hook.matches(&upgrade_only, |_| true));

    let install = HookTransaction {
        packages: vec![("linux", HookOperation::Install)],
        paths: vec![(
            "usr/lib/modules/6.12/vmlinuz".to_string(),
            HookOperation::Install,
        )],
    };
    assert!(hook.matches(&install, |_| true));
    assert_eq!(
        hook.matched_targets(&install),
        vec!["usr/lib/modules/6.12/vmlinuz".to_string()]
    );
}

#[test]
fn load_hooks_overrides_by_name_and_sorts() {
    let base = TempDir::new().expect("base");
    let etc = TempDir::new().expect("etc");

    fs::write(
        base.path().join("20-b.hook"),
        "[Trigger]\nTarget = *\n\n[Action]\nExec = /bin/true\n",
    )
    .expect("base hook");
    fs::write(
        etc.path().join("10-a.hook"),
        "[Trigger]\nTarget = *\n\n[Action]\nExec = /bin/false\n",
    )
    .expect("etc hook");
    // Same file name in the higher-priority directory overrides the base.
    fs::write(
        base.path().join("30-c.hook"),
        "[Trigger]\nTarget = *\n\n[Action]\nExec = /bin/false\n",
    )
    .expect("base c");
    fs::write(
        etc.path().join("30-c.hook"),
        "[Trigger]\nTarget = *\n\n[Action]\nExec = /bin/true\n",
    )
    .expect("etc c");

    let hooks =
        load_hooks(&[base.path().to_path_buf(), etc.path().to_path_buf()]).expect("load hooks");
    let names: Vec<&str> = hooks.iter().map(|h| h.name.as_str()).collect();
    assert_eq!(names, vec!["10-a.hook", "20-b.hook", "30-c.hook"]);
    // 30-c.hook comes from `etc` (last directory wins).
    assert_eq!(hooks[2].exec, "/bin/true");
}

#[test]
fn runs_hook_with_targets_on_stdin() {
    let tmp = TempDir::new().expect("tmp");
    let script = tmp.path().join("hook.sh");
    let output = tmp.path().join("targets.txt");
    fs::write(
        &script,
        format!("#!/bin/sh\ncat > '{}'\n", output.display()),
    )
    .expect("write script");
    let mut perms = fs::metadata(&script).expect("meta").permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&script, perms).expect("chmod");

    let content = format!(
            "[Trigger]\nOperation = Install\nType = Package\nTarget = kitty\n\n[Action]\nWhen = PreTransaction\nExec = {} --flag\nNeedsTargets\n",
            script.display()
        );
    let hook = parse_hook("50-test.hook", &content).expect("parse");

    let tx = HookTransaction {
        packages: vec![("kitty", HookOperation::Install)],
        paths: Vec::new(),
    };
    let failures = run_hooks(
        &[hook],
        HookWhen::PreTransaction,
        &tx,
        Path::new("/"),
        Path::new("/var/lib/xpm"),
        |_| true,
    )
    .expect("run");
    assert!(failures.is_empty(), "unexpected failures: {failures:?}");
    assert_eq!(fs::read_to_string(&output).expect("targets"), "kitty\n");
}

#[test]
fn split_exec_handles_quotes_and_escapes() {
    assert_eq!(
        split_exec(r#"/bin/sh -c 'echo hello world'"#).unwrap(),
        vec!["/bin/sh", "-c", "echo hello world"]
    );
    assert_eq!(
        split_exec(r#"/bin/echo "a b" c\ d"#).unwrap(),
        vec!["/bin/echo", "a b", "c d"]
    );
    assert!(split_exec("'/bin/sh").is_err());
}

#[test]
fn runs_hook_with_quoted_exec_arguments() {
    let tmp = TempDir::new().expect("tmp");
    let output = tmp.path().join("quoted.txt");
    let content = format!(
            "[Trigger]\nOperation = Install\nType = Package\nTarget = kitty\n\n[Action]\nWhen = PreTransaction\nExec = /bin/sh -c 'printf quoted > {}'\n",
            output.display()
        );
    let hook = parse_hook("50-quoted.hook", &content).expect("parse");
    let tx = HookTransaction {
        packages: vec![("kitty", HookOperation::Install)],
        paths: Vec::new(),
    };
    let failures = run_hooks(
        &[hook],
        HookWhen::PreTransaction,
        &tx,
        Path::new("/"),
        Path::new("/var/lib/xpm"),
        |_| true,
    )
    .expect("run");
    assert!(failures.is_empty(), "unexpected failures: {failures:?}");
    assert_eq!(fs::read_to_string(&output).expect("output"), "quoted");
}

#[test]
fn hook_dirs_for_alternative_root_are_relative() {
    let dirs = hook_dirs_for(Path::new("/mnt/target"));
    assert_eq!(
        dirs[0],
        PathBuf::from("/mnt/target/usr/share/libalpm/hooks")
    );
    assert_eq!(dirs[1], PathBuf::from("/mnt/target/etc/pacman.d/hooks"));
}

#[test]
fn pacman_shim_exposes_a_pacman_query_entrypoint() {
    let shim = PacmanShim::new(Path::new("/var/lib/xpm")).expect("shim");
    let shim_path = shim.dir.join("pacman");
    let script = std::fs::read_to_string(&shim_path).expect("pacman shim");
    assert!(script.contains("--dbpath '/var/lib/xpm'"), "{script}");
    let path = shim.path_env();
    assert!(path
        .to_string_lossy()
        .starts_with(&shim.dir.to_string_lossy().to_string()));
}

#[test]
fn enforce_failures_aborts_only_on_pre_with_flag() {
    let pre = HookFailure {
        name: "10-pre.hook".into(),
        abort_on_fail: true,
        error: "boom".into(),
    };
    assert!(enforce_failures(HookWhen::PreTransaction, std::slice::from_ref(&pre)).is_err());

    let post = HookFailure {
        abort_on_fail: true,
        ..pre
    };
    let warnings = enforce_failures(HookWhen::PostTransaction, &[post]).expect("post never aborts");
    assert_eq!(warnings.len(), 1);
}
