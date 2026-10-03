//! Transaction hooks: executables in `pre-transaction.d` / `post-transaction.d`
//! run around a transaction with the environment describing it. The
//! provisioning payload (`x gen`) ships hooks there to record generations
//! after xpm transactions; xpm itself stays unaware of snapshots.
//!
//! Contract (see `docs/GENERATIONS.md`):
//!
//! | Variable | Meaning |
//! |----------|---------|
//! | `XPM_ROOT_DIR` | Target root |
//! | `XPM_ACTION` | `install`, `remove` or `upgrade` |
//! | `XPM_JOURNAL` | Path of the transaction journal |
//! | `XPM_PKG_NAMES` | Space-separated package names |
//! | `XPM_PKG_VERSIONS` | Space-separated target versions (empty for removals) |
//!
//! A failing **pre** hook must abort the transaction (the caller decides);
//! a failing **post** hook only warns.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::error::XpmResult;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct HookRunOutcome {
    pub ran: Vec<String>,
    pub failed: Vec<String>,
}

/// Runs every executable in `dir` in lexical order. Missing directory is not
/// an error (hooks are optional).
pub fn run_transaction_hooks(dir: &Path, envs: &[(String, String)]) -> XpmResult<HookRunOutcome> {
    let mut outcome = HookRunOutcome::default();
    if !dir.exists() {
        return Ok(outcome);
    }

    let mut hooks: Vec<PathBuf> = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let meta = entry.metadata()?;
        if !meta.is_file() {
            continue;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if meta.permissions().mode() & 0o111 == 0 {
                continue;
            }
        }
        hooks.push(entry.path());
    }
    hooks.sort();

    for hook in hooks {
        let name = hook
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let status = Command::new(&hook)
            .envs(envs.iter().map(|(k, v)| (k.clone(), v.clone())))
            .status();
        match status {
            Ok(s) if s.success() => outcome.ran.push(name),
            _ => outcome.failed.push(name),
        }
    }
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use tempfile::TempDir;

    fn write_hook(dir: &Path, name: &str, body: &str) {
        let path = dir.join(name);
        fs::write(&path, format!("#!/usr/bin/env bash\n{body}\n")).expect("write hook");
        let mut perms = fs::metadata(&path).expect("meta").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&path, perms).expect("chmod");
    }

    #[test]
    fn missing_dir_is_ok() {
        let tmp = TempDir::new().expect("tmp");
        let outcome = run_transaction_hooks(&tmp.path().join("nope"), &[]).expect("run");
        assert_eq!(outcome, HookRunOutcome::default());
    }

    #[test]
    fn runs_in_order_with_env_and_reports_failures() {
        let tmp = TempDir::new().expect("tmp");
        let mark = tmp.path().join("order.txt");
        write_hook(
            tmp.path(),
            "10-first.sh",
            &format!(
                "printf 'first:%s\\n' \"$XPM_ACTION\" >> '{}'",
                mark.display()
            ),
        );
        write_hook(tmp.path(), "20-fail.sh", "exit 3");
        write_hook(
            tmp.path(),
            "30-last.sh",
            &format!("printf 'last\\n' >> '{}'", mark.display()),
        );
        // Not executable: must be ignored.
        fs::write(tmp.path().join("40-skip.sh"), "#!/bin/sh\nexit 9\n").expect("skip");

        let envs = vec![("XPM_ACTION".to_string(), "upgrade".to_string())];
        let outcome = run_transaction_hooks(tmp.path(), &envs).expect("run");

        assert_eq!(outcome.ran, vec!["10-first.sh", "30-last.sh"]);
        assert_eq!(outcome.failed, vec!["20-fail.sh"]);
        let log = fs::read_to_string(&mark).expect("log");
        assert_eq!(log, "first:upgrade\nlast\n");
    }
}
