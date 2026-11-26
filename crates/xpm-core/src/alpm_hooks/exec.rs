//! Hook execution: argv splitting, chrooting and the pacman query shim.

use super::*;
use crate::shell::shell_quote;

/// Split a pacman hook `Exec` line into argv, honoring single/double quotes
/// and backslash escapes. Hook commands are not run through a shell.
pub(super) fn split_exec(input: &str) -> Result<Vec<String>, String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut escape = false;
    let mut started = false;

    for ch in input.chars() {
        if escape {
            current.push(ch);
            escape = false;
            started = true;
            continue;
        }
        match quote {
            Some('"') if ch == '\\' => escape = true,
            Some('"') if ch == '"' => quote = None,
            Some('\'') if ch == '\'' => quote = None,
            Some(_) => current.push(ch),
            None if ch == '\\' => escape = true,
            None if ch == '"' || ch == '\'' => {
                quote = Some(ch);
                started = true;
            }
            None if ch.is_whitespace() => {
                if started || !current.is_empty() {
                    args.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            None => {
                current.push(ch);
                started = true;
            }
        }
    }

    if escape {
        return Err("trailing backslash".to_string());
    }
    if quote.is_some() {
        return Err("unterminated quote".to_string());
    }
    if started || !current.is_empty() {
        args.push(current);
    }
    Ok(args)
}

fn chroot_command(command: &mut Command, root: &Path) -> Result<(), String> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::process::CommandExt;

    let root = CString::new(root.as_os_str().as_bytes())
        .map_err(|_| "root path contains NUL".to_string())?;
    unsafe {
        command.pre_exec(move || {
            libc::umask(0o022);
            if libc::chroot(root.as_ptr()) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            if libc::chdir(c"/".as_ptr()) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    Ok(())
}

/// A temporary directory with a `pacman` script that delegates to xpm.
///
/// Arch ALPM hooks (notably mkinitcpio) query installed packages through
/// `pacman -Q...`. xpm owns the local database, so hooks must query it
/// directly instead of a stale pacman database. Prepending this directory to
/// PATH during hook execution keeps those hooks working without writing
/// pacman's database. The shim is only installed for host-root transactions;
/// chrooted hooks keep the guest's own tools.
pub(super) struct PacmanShim {
    pub(super) dir: PathBuf,
}

impl PacmanShim {
    pub(super) fn new(db_path: &Path) -> Option<Self> {
        if std::env::var_os(ENV_PACMAN_SHIM).is_some_and(|value| value == "0") {
            return None;
        }
        let xpm = std::env::current_exe().ok()?;
        if !xpm.is_file() {
            return None;
        }
        let xpm = xpm.to_str()?;
        let db_path = db_path.to_str()?;
        let script = format!(
            "#!/bin/sh\nexec {} --dbpath {} \"$@\"\n",
            shell_quote(xpm),
            shell_quote(db_path)
        );
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let base = std::env::temp_dir();
        for attempt in 0..8 {
            let dir = base.join(format!(
                "xpm-pacman-shim-{}-{stamp}-{attempt}",
                std::process::id()
            ));
            match fs::create_dir(&dir) {
                Ok(()) => {
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        let _ = fs::set_permissions(&dir, fs::Permissions::from_mode(0o700));
                    }
                    let shim = dir.join("pacman");
                    let installed = fs::write(&shim, script.as_bytes())
                        .and_then(|()| {
                            #[cfg(unix)]
                            {
                                use std::os::unix::fs::PermissionsExt;
                                fs::set_permissions(&shim, fs::Permissions::from_mode(0o700))
                            }
                            #[cfg(not(unix))]
                            {
                                Ok(())
                            }
                        })
                        .is_ok();
                    if installed {
                        return Some(Self { dir });
                    }
                    let _ = fs::remove_dir_all(&dir);
                    return None;
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(_) => return None,
            }
        }
        None
    }

    pub(super) fn path_env(&self) -> OsString {
        let inherited = std::env::var_os("PATH").unwrap_or_else(|| {
            OsString::from("/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin")
        });
        let mut path = self.dir.as_os_str().to_os_string();
        path.push(":");
        path.push(inherited);
        path
    }
}

impl Drop for PacmanShim {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
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
    root: &Path,
    db_path: &Path,
    mut installed: impl FnMut(&str) -> bool,
) -> XpmResult<Vec<HookFailure>> {
    let chroot = root != Path::new("/") && std::env::var_os("XPM_ALPM_HOOKS_NO_CHROOT").is_none();
    let shim = if chroot {
        None
    } else {
        PacmanShim::new(db_path)
    };
    let mut failures = Vec::new();

    for hook in hooks {
        if !hook.runs_at(when) || !hook.matches(tx, &mut installed) {
            continue;
        }

        let argv = match split_exec(&hook.exec) {
            Ok(argv) if !argv.is_empty() => argv,
            Ok(_) => {
                failures.push(HookFailure {
                    name: hook.name.clone(),
                    abort_on_fail: hook.abort_on_fail,
                    error: "empty Exec".to_string(),
                });
                continue;
            }
            Err(error) => {
                failures.push(HookFailure {
                    name: hook.name.clone(),
                    abort_on_fail: hook.abort_on_fail,
                    error: format!("invalid Exec: {error}"),
                });
                continue;
            }
        };

        let mut command = Command::new(&argv[0]);
        command.args(&argv[1..]);
        command.env("XPM_ALPM_HOOK", &hook.name);
        if let Some(shim) = &shim {
            command.env("PATH", shim.path_env());
        }
        command.current_dir("/");
        if chroot {
            if let Err(error) = chroot_command(&mut command, root) {
                failures.push(HookFailure {
                    name: hook.name.clone(),
                    abort_on_fail: hook.abort_on_fail,
                    error,
                });
                continue;
            }
        }

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
