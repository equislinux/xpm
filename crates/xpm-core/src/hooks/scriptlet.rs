//! Package `.INSTALL` scriptlet hooks and shell integration.

use super::*;

/// Execute pre-operation package scriptlets from `.INSTALL` files.
///
/// Runs before file changes: `pre_install` (install), `pre_upgrade` (upgrade), `pre_remove` (remove).
pub struct PreScriptletHook;

impl Hook for PreScriptletHook {
    fn name(&self) -> &str {
        "pre-scriptlet"
    }

    fn run(&self, context: &HookContext) -> XpmResult<()> {
        let Some(script_data) = load_install_script(context)? else {
            return Ok(());
        };

        let functions: &[&str] = match context.operation_type {
            OperationType::Install => &["pre_install"],
            OperationType::Upgrade => &["pre_upgrade"],
            OperationType::Remove => &["pre_remove"],
        };

        if functions.is_empty() {
            return Ok(());
        }

        run_scriptlet_functions(context, &script_data, functions)
    }
}

/// Execute post-operation package scriptlets from `.INSTALL` files.
///
/// Runs after file changes: `post_install` (install), `post_upgrade` (upgrade), `post_remove` (remove).
pub struct PostScriptletHook;

impl Hook for PostScriptletHook {
    fn name(&self) -> &str {
        "post-scriptlet"
    }

    fn run(&self, context: &HookContext) -> XpmResult<()> {
        let Some(script_data) = load_install_script(context)? else {
            return Ok(());
        };

        let functions: &[&str] = match context.operation_type {
            OperationType::Install => &["post_install"],
            OperationType::Upgrade => &["post_upgrade"],
            OperationType::Remove => &["post_remove"],
        };

        if functions.is_empty() {
            return Ok(());
        }

        run_scriptlet_functions(context, &script_data, functions)
    }
}

pub(super) fn load_install_script(context: &HookContext) -> XpmResult<Option<Vec<u8>>> {
    if let Some(metadata) = &context.metadata {
        // Cached metadata is authoritative: a missing .INSTALL must not
        // trigger another decompression of the package.
        return Ok(metadata.install_script.clone());
    }

    if let Some(pkg_file) = &context.pkg_file {
        if let Some(data) = crate::package::reader::read_raw_entry(pkg_file, ".INSTALL")? {
            return Ok(Some(data));
        }
    }

    let local_install = context.local_db_dir.join(&context.pkg_name).join("install");
    if local_install.exists() {
        return Ok(Some(fs::read(local_install)?));
    }

    Ok(None)
}

pub(super) fn script_defines_function(script_data: &[u8], function: &str) -> bool {
    let text = String::from_utf8_lossy(script_data);
    text.lines().any(|line| {
        let line = line.trim();
        let line = line.strip_prefix("function ").unwrap_or(line);
        line.strip_prefix(function)
            .map(|rest| rest.trim_start().starts_with("()"))
            .unwrap_or(false)
    })
}

pub(super) fn run_scriptlet_functions(
    context: &HookContext,
    script_data: &[u8],
    functions: &[&str],
) -> XpmResult<()> {
    let Some(function) = functions.first().copied() else {
        return Ok(());
    };
    if !script_defines_function(script_data, function) {
        return Ok(());
    }

    if context.scriptlet_mode == ScriptletMode::Skip {
        tracing::debug!(package = %context.pkg_name, "scriptlets disabled; skipping");
        return Ok(());
    }

    let chrooted = context.root_dir != Path::new("/");
    if chrooted && !context.root_dir.join("bin/sh").exists() {
        let message = format!(
            "target {} has no /bin/sh; cannot run .INSTALL scriptlet for {}",
            context.root_dir.display(),
            context.pkg_name
        );
        if context.scriptlet_mode == ScriptletMode::Require {
            return Err(XpmError::Package(message));
        }
        tracing::warn!(
            package = %context.pkg_name,
            root = %context.root_dir.display(),
            "skipping .INSTALL script: target has no /bin/sh (no host fallback)"
        );
        return Ok(());
    }

    let tmp_root = context.root_dir.join("tmp");
    if !tmp_root.exists() {
        use std::os::unix::fs::DirBuilderExt;
        let mut builder = fs::DirBuilder::new();
        builder.mode(0o1777).recursive(true);
        builder.create(&tmp_root)?;
    }
    let unique = format!(
        ".xpm-scriptlet-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    );
    let tmp_dir = tmp_root.join(&unique);
    fs::create_dir(&tmp_dir)?;
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&tmp_dir, fs::Permissions::from_mode(0o700))?;
    }
    let script_path = tmp_dir.join(".INSTALL");
    fs::write(&script_path, script_data)?;
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&script_path, fs::Permissions::from_mode(0o644))?;
    }

    let inside_script = format!("/tmp/{unique}/.INSTALL");
    let mut args = vec![context.pkg_version.clone()];
    if function.contains("upgrade") {
        args.push(context.old_version.clone().unwrap_or_default());
    }
    let quoted_args = args
        .iter()
        .map(|arg| shell_quote(arg))
        .collect::<Vec<_>>()
        .join(" ");
    let command = format!(
        ". {script}; if [ -n \"$(command -v {function})\" ]; then {function} {args}; fi",
        script = shell_quote(&inside_script),
        function = shell_quote(function),
        args = quoted_args,
    );

    let mut cmd = Command::new("/bin/sh");
    cmd.arg("-c").arg(&command);
    cmd.env_clear()
        .env("PATH", "/usr/local/sbin:/usr/local/bin:/usr/bin:/bin")
        .env("HOME", "/root")
        .env("SHELL", "/bin/sh")
        .env("LANG", "C")
        .env("SHLVL", "1")
        .env("XPM_ROOT_DIR", "/")
        .env("XPM_PKG_NAME", &context.pkg_name)
        .env("XPM_PKG_VERSION", &context.pkg_version);

    if chrooted {
        use std::ffi::CString;
        use std::os::unix::process::CommandExt;
        let root = CString::new(context.root_dir.as_os_str().as_bytes()).map_err(|_| {
            XpmError::Package(format!(
                "root path contains NUL: {}",
                context.root_dir.display()
            ))
        })?;
        let root_path = root.clone();
        unsafe {
            cmd.pre_exec(move || {
                libc::umask(0o022);
                if libc::chroot(root_path.as_ptr()) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                if libc::chdir(c"/".as_ptr()) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    } else {
        cmd.current_dir("/");
    }

    let result = cmd.status();
    let _ = fs::remove_dir_all(&tmp_dir);

    match result {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => {
            if context.scriptlet_mode == ScriptletMode::Require {
                return Err(XpmError::Package(format!(
                    "package scriptlet for {} exited with {status}",
                    context.pkg_name
                )));
            }
            tracing::warn!(
                package = %context.pkg_name,
                status = %status,
                "package scriptlet failed; continuing (pacman-compatible)"
            );
            Ok(())
        }
        Err(error) => {
            if context.scriptlet_mode == ScriptletMode::Require {
                return Err(XpmError::Package(format!(
                    "could not execute package scriptlet for {}: {error}",
                    context.pkg_name
                )));
            }
            tracing::warn!(
                package = %context.pkg_name,
                error = %error,
                "could not execute package scriptlet; continuing (pacman-compatible)"
            );
            Ok(())
        }
    }
}

pub(super) fn ensure_shell_shim(relative_path: &str, target: &Path) -> XpmResult<Option<PathBuf>> {
    let home = match std::env::var_os("HOME") {
        Some(h) => PathBuf::from(h),
        None => return Ok(None),
    };

    let local_bin = home.join(".local/bin");
    fs::create_dir_all(&local_bin)?;

    let Some(bin_name) = Path::new(relative_path).file_name() else {
        return Ok(None);
    };

    let shim_path = local_bin.join(bin_name);
    if shim_path.exists() || shim_path.symlink_metadata().is_ok() {
        let _ = fs::remove_file(&shim_path);
    }

    std::os::unix::fs::symlink(target, &shim_path)?;
    Ok(Some(shim_path))
}

pub(super) fn ensure_shell_path_on_bash_zsh() -> XpmResult<()> {
    let home = match std::env::var_os("HOME") {
        Some(h) => PathBuf::from(h),
        None => return Ok(()),
    };

    let marker = "# xpm shell integration";
    let export_line = "export PATH=\"$HOME/.local/bin:$PATH\"";

    for rc in [".bashrc", ".zshrc"] {
        let rc_path = home.join(rc);
        if !rc_path.exists() {
            continue;
        }

        let content = fs::read_to_string(&rc_path).unwrap_or_default();
        if content.contains(marker) {
            continue;
        }

        let mut append = String::new();
        if !content.ends_with('\n') {
            append.push('\n');
        }
        append.push_str(marker);
        append.push('\n');
        append.push_str(export_line);
        append.push('\n');

        let mut merged = content;
        merged.push_str(&append);
        fs::write(rc_path, merged)?;
    }

    Ok(())
}
