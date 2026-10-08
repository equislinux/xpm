//! ALPM hook files (`/usr/share/libalpm/hooks/*.hook` and
//! `/etc/pacman.d/hooks/*.hook`).
//!
//! X Linux ships pacman hooks that snapshot generations around transactions
//! (`10-x-gen-pre.hook` / `20-x-gen-post.hook`). Running the same `.hook`
//! files keeps xpm transactions generation-aware without any xpm→x-scripts
//! dependency.
//!
//! Supported syntax (the subset needed in practice):
//!
//! ```ini
//! [Trigger]
//! Operation = Install        # Install | Upgrade | Remove (repeatable)
//! Type = Package             # Package | Path
//! Target = *                 # glob for Package names; prefix for Path
//!
//! [Action]
//! Description = ...
//! When = PreTransaction      # PreTransaction | PostTransaction (both allowed)
//! Exec = /usr/share/x/hooks/pacman-gen.sh pre
//! Depends = x-scripts        # skip the hook when not installed
//! AbortOnFail
//! NeedsTargets               # feed the affected package names on stdin
//! ```
//!
//! Multiple `[Trigger]` sections are OR-ed; targets and operations inside one
//! trigger are AND-ed. Hook order is the lexical file-name order, matching
//! pacman and making `10-`/`20-` prefixes meaningful. `Path` triggers are
//! parsed but only matched when the caller provides the transaction's file
//! paths (xpm package operations do not always expose them); `Package`
//! triggers cover the generation hooks.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::error::{XpmError, XpmResult};

/// Environment variable overriding the hook directories (colon-separated),
/// used by tests and by non-standard installations.
pub const ENV_HOOK_DIRS: &str = "XPM_ALPM_HOOKS_DIRS";

/// Default hook directories, in priority order (later wins by file name).
pub const DEFAULT_HOOK_DIRS: &[&str] = &["/usr/share/libalpm/hooks", "/etc/pacman.d/hooks"];

/// Hook phase relative to the transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HookWhen {
    PreTransaction,
    PostTransaction,
}

/// Operation a trigger applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HookOperation {
    Install,
    Upgrade,
    Remove,
}

/// What the trigger targets are matched against.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TriggerType {
    #[default]
    Package,
    Path,
}

/// One `[Trigger]` section.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Trigger {
    pub operations: Vec<HookOperation>,
    pub trigger_type: TriggerType,
    pub targets: Vec<String>,
}

/// One parsed `.hook` file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlpmHook {
    /// File name (used for ordering and diagnostics).
    pub name: String,
    pub description: Option<String>,
    /// Phases this hook runs in.
    pub when: Vec<HookWhen>,
    /// Command line executed without a shell.
    pub exec: String,
    /// Packages that must be installed for the hook to run.
    pub depends: Vec<String>,
    /// Abort the transaction when a pre hook fails.
    pub abort_on_fail: bool,
    /// Feed the affected package names to the hook's stdin.
    pub needs_targets: bool,
    pub triggers: Vec<Trigger>,
}

/// Context of the transaction a hook is matched against.
#[derive(Debug, Clone, Default)]
pub struct HookTransaction<'a> {
    /// `(package name, operation)` pairs.
    pub packages: Vec<(&'a str, HookOperation)>,
    /// File paths touched by the transaction (for `Path` triggers).
    pub paths: Vec<String>,
}

impl AlpmHook {
    /// Whether the hook must run for `tx`: any trigger matches and every
    /// `Depends` is satisfied by `installed`.
    pub fn matches(
        &self,
        tx: &HookTransaction<'_>,
        mut installed: impl FnMut(&str) -> bool,
    ) -> bool {
        if !self.depends.iter().all(|dep| installed(dep)) {
            return false;
        }

        self.triggers.iter().any(|trigger| {
            // Operations: empty means "any".
            let op_ok = trigger.operations.is_empty()
                || trigger
                    .operations
                    .iter()
                    .any(|op| tx.packages.iter().any(|(_, tx_op)| tx_op == op));
            if !op_ok {
                return false;
            }

            match trigger.trigger_type {
                TriggerType::Package => trigger.targets.iter().any(|pattern| {
                    tx.packages
                        .iter()
                        .any(|(name, _)| glob_match(pattern, name))
                }),
                TriggerType::Path => trigger.targets.iter().any(|prefix| {
                    tx.paths.iter().any(|path| {
                        path.trim_start_matches('/')
                            .starts_with(prefix.trim_start_matches('/'))
                    })
                }),
            }
        })
    }

    /// Whether the hook runs in `when`.
    pub fn runs_at(&self, when: HookWhen) -> bool {
        self.when.contains(&when)
    }

    /// Package names of the transaction that this hook's triggers matched.
    pub fn matched_targets(&self, tx: &HookTransaction<'_>) -> Vec<String> {
        let mut out = Vec::new();
        for trigger in &self.triggers {
            if trigger.trigger_type != TriggerType::Package {
                continue;
            }
            for (name, _) in &tx.packages {
                if trigger
                    .targets
                    .iter()
                    .any(|pattern| glob_match(pattern, name))
                    && !out.iter().any(|seen| seen == name)
                {
                    out.push((*name).to_string());
                }
            }
        }
        out
    }
}

/// A hook that failed during execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookFailure {
    pub name: String,
    pub abort_on_fail: bool,
    pub error: String,
}

/// Parses one `.hook` file.
pub fn parse_hook(name: &str, content: &str) -> XpmResult<AlpmHook> {
    let mut hook = AlpmHook {
        name: name.to_string(),
        description: None,
        when: Vec::new(),
        exec: String::new(),
        depends: Vec::new(),
        abort_on_fail: false,
        needs_targets: false,
        triggers: Vec::new(),
    };

    #[derive(Clone, Copy, PartialEq)]
    enum Section {
        Trigger,
        Action,
    }

    let mut section: Option<Section> = None;
    let mut trigger: Option<Trigger> = None;

    let flush_trigger = |hook: &mut AlpmHook, trigger: &mut Option<Trigger>| {
        if let Some(trigger) = trigger.take() {
            hook.triggers.push(trigger);
        }
    };

    for raw in content.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        if line.starts_with('[') && line.ends_with(']') {
            flush_trigger(&mut hook, &mut trigger);
            let header = line[1..line.len() - 1].trim().to_ascii_lowercase();
            section = match header.as_str() {
                "trigger" => {
                    trigger = Some(Trigger::default());
                    Some(Section::Trigger)
                }
                "action" => Some(Section::Action),
                other => {
                    return Err(XpmError::Other(format!(
                        "{name}: unknown section [{other}]"
                    )))
                }
            };
            continue;
        }

        let Some(section) = section else {
            continue;
        };

        // Flags have no value.
        match line {
            "AbortOnFail" => {
                hook.abort_on_fail = true;
                continue;
            }
            "NeedsTargets" => {
                hook.needs_targets = true;
                continue;
            }
            _ => {}
        }

        let Some((key, value)) = line.split_once('=') else {
            return Err(XpmError::Other(format!(
                "{name}: expected `Key = value`, got `{line}`"
            )));
        };
        let key = key.trim();
        let value = value.trim();

        match section {
            Section::Trigger => {
                let Some(trigger) = trigger.as_mut() else {
                    continue;
                };
                match key {
                    "Operation" => trigger.operations.push(parse_operation(value, name)?),
                    "Type" => {
                        trigger.trigger_type = match value {
                            "Package" => TriggerType::Package,
                            "Path" => TriggerType::Path,
                            other => {
                                return Err(XpmError::Other(format!(
                                    "{name}: unknown trigger type `{other}`"
                                )))
                            }
                        }
                    }
                    "Target" => trigger.targets.push(value.to_string()),
                    _ => {}
                }
            }
            Section::Action => match key {
                "Description" => hook.description = Some(value.to_string()),
                "When" => {
                    for token in value.split(['|', ' ', ',']) {
                        match token.trim() {
                            "" => {}
                            "PreTransaction" => hook.when.push(HookWhen::PreTransaction),
                            "PostTransaction" => hook.when.push(HookWhen::PostTransaction),
                            other => {
                                return Err(XpmError::Other(format!(
                                    "{name}: unknown When `{other}`"
                                )))
                            }
                        }
                    }
                }
                "Exec" => hook.exec = value.to_string(),
                "Depends" => hook.depends.push(value.to_string()),
                _ => {}
            },
        }
    }

    flush_trigger(&mut hook, &mut trigger);

    if hook.exec.is_empty() {
        return Err(XpmError::Other(format!("{name}: missing Exec")));
    }
    if hook.triggers.is_empty() {
        return Err(XpmError::Other(format!("{name}: no [Trigger] section")));
    }
    if hook.when.is_empty() {
        // Pacman defaults to PostTransaction when When is omitted.
        hook.when.push(HookWhen::PostTransaction);
    }
    for trigger in &mut hook.triggers {
        if trigger.targets.is_empty() {
            trigger.targets.push("*".to_string());
        }
    }

    Ok(hook)
}

fn parse_operation(value: &str, name: &str) -> XpmResult<HookOperation> {
    match value {
        "Install" => Ok(HookOperation::Install),
        "Upgrade" => Ok(HookOperation::Upgrade),
        "Remove" => Ok(HookOperation::Remove),
        other => Err(XpmError::Other(format!(
            "{name}: unknown Operation `{other}`"
        ))),
    }
}

/// Loads and parses every `.hook` file from the given directories.
///
/// Files are merged by file name with later directories overriding earlier
/// ones (so `/etc/pacman.d/hooks` wins over `/usr/share/libalpm/hooks`), then
/// sorted by name — the execution order pacman uses.
pub fn load_hooks(dirs: &[PathBuf]) -> XpmResult<Vec<AlpmHook>> {
    let mut by_name: std::collections::BTreeMap<String, PathBuf> =
        std::collections::BTreeMap::new();

    for dir in dirs {
        let Ok(entries) = fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("hook") {
                continue;
            }
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                by_name.insert(name.to_string(), path);
            }
        }
    }

    let mut hooks = Vec::with_capacity(by_name.len());
    for (name, path) in by_name {
        let content = fs::read_to_string(&path)?;
        let hook = parse_hook(&name, &content)
            .map_err(|e| XpmError::Other(format!("failed to parse {}: {e}", path.display())))?;
        hooks.push(hook);
    }
    Ok(hooks)
}

/// Directories to load hooks from: `XPM_ALPM_HOOKS_DIRS` when set, otherwise
/// the pacman defaults.
pub fn hook_dirs() -> Vec<PathBuf> {
    match std::env::var(ENV_HOOK_DIRS) {
        Ok(value) if !value.is_empty() => value.split(':').map(PathBuf::from).collect(),
        _ => DEFAULT_HOOK_DIRS.iter().map(PathBuf::from).collect(),
    }
}

/// Runs every hook that matches `tx` for `when`, in lexical order.
///
/// `installed` answers whether a `Depends` package is present. Hook output is
/// inherited so users see the scripts' messages. Failures are reported, not
/// fatal: the caller decides based on `abort_on_fail`.
pub fn run_hooks(
    hooks: &[AlpmHook],
    when: HookWhen,
    tx: &HookTransaction<'_>,
    mut installed: impl FnMut(&str) -> bool,
) -> XpmResult<Vec<HookFailure>> {
    let mut failures = Vec::new();

    for hook in hooks {
        if !hook.runs_at(when) || !hook.matches(tx, &mut installed) {
            continue;
        }

        let mut command = Command::new(hook.exec.split_whitespace().next().unwrap_or(""));
        for arg in hook.exec.split_whitespace().skip(1) {
            command.arg(arg);
        }
        command.env("XPM_ALPM_HOOK", &hook.name);

        if hook.needs_targets {
            command.stdin(Stdio::piped());
        }

        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(e) => {
                failures.push(HookFailure {
                    name: hook.name.clone(),
                    abort_on_fail: hook.abort_on_fail,
                    error: format!("failed to start `{}`: {e}", hook.exec),
                });
                continue;
            }
        };

        if hook.needs_targets {
            if let Some(mut stdin) = child.stdin.take() {
                use std::io::Write;
                let targets = hook.matched_targets(tx);
                let _ = stdin.write_all((targets.join("\n") + "\n").as_bytes());
            }
        }

        match child.wait() {
            Ok(status) if status.success() => {
                tracing::debug!(hook = %hook.name, "alpm hook ran");
            }
            Ok(status) => failures.push(HookFailure {
                name: hook.name.clone(),
                abort_on_fail: hook.abort_on_fail,
                error: format!("exited with {status}"),
            }),
            Err(e) => failures.push(HookFailure {
                name: hook.name.clone(),
                abort_on_fail: hook.abort_on_fail,
                error: format!("wait failed: {e}"),
            }),
        }
    }

    Ok(failures)
}

/// Executes a hook list the way pacman does: pre hooks may abort, post hooks
/// only warn. Returns an error when a pre hook with `AbortOnFail` fails.
pub fn enforce_failures(when: HookWhen, failures: &[HookFailure]) -> XpmResult<Vec<String>> {
    let mut warnings = Vec::new();
    let mut aborts = Vec::new();

    for failure in failures {
        if when == HookWhen::PreTransaction && failure.abort_on_fail {
            aborts.push(format!("{}: {}", failure.name, failure.error));
        } else {
            warnings.push(format!("{}: {}", failure.name, failure.error));
        }
    }

    if !aborts.is_empty() {
        return Err(XpmError::Transaction(format!(
            "aborting due to failed pre-transaction hook(s): {}",
            aborts.join(", ")
        )));
    }
    Ok(warnings)
}

/// Wildcard match supporting `*` (any sequence) and `?` (one character),
/// which covers the target globs used by distribution hooks.
pub fn glob_match(pattern: &str, text: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let text: Vec<char> = text.chars().collect();

    let (mut p, mut t) = (0usize, 0usize);
    let mut star: Option<usize> = None;
    let mut mark = 0usize;

    while t < text.len() {
        if p < pattern.len() && (pattern[p] == '?' || pattern[p] == text[t]) {
            p += 1;
            t += 1;
        } else if p < pattern.len() && pattern[p] == '*' {
            star = Some(p);
            mark = t;
            p += 1;
        } else if let Some(star_pos) = star {
            p = star_pos + 1;
            mark += 1;
            t = mark;
        } else {
            return false;
        }
    }

    while p < pattern.len() && pattern[p] == '*' {
        p += 1;
    }
    p == pattern.len()
}

/// Convenience: set of `Depends` names present in a local database.
pub fn installed_names(local_db_dir: &Path) -> HashSet<String> {
    let mut names = HashSet::new();
    if let Ok(entries) = fs::read_dir(local_db_dir) {
        for entry in entries.flatten() {
            if entry.path().join("version").is_file() {
                if let Some(name) = entry.file_name().to_str() {
                    names.insert(name.to_string());
                }
            }
        }
    }
    names
}

#[cfg(test)]
mod tests {
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
        let content = "[Trigger]\nOperation = Upgrade\nType = Path\nTarget = usr/lib/modules\n\n[Action]\nExec = /bin/true\n";
        let hook = parse_hook("modules.hook", content).expect("parse");

        let without_paths = HookTransaction {
            packages: vec![("linux", HookOperation::Upgrade)],
            paths: Vec::new(),
        };
        assert!(!hook.matches(&without_paths, |_| true));

        let with_paths = HookTransaction {
            packages: vec![("linux", HookOperation::Upgrade)],
            paths: vec!["usr/lib/modules/6.12/vmlinuz".to_string()],
        };
        assert!(hook.matches(&with_paths, |_| true));
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
        let failures = run_hooks(&[hook], HookWhen::PreTransaction, &tx, |_| true).expect("run");
        assert!(failures.is_empty(), "unexpected failures: {failures:?}");
        assert_eq!(fs::read_to_string(&output).expect("targets"), "kitty\n");
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
        let warnings =
            enforce_failures(HookWhen::PostTransaction, &[post]).expect("post never aborts");
        assert_eq!(warnings.len(), 1);
    }
}
