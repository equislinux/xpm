//! Small helpers: root command wrapper, ids, paths, dates and quoting.

use super::*;

pub(super) fn xpm_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// Hash the host configuration that `xpm bootstrap` copies into the template.
/// A changed repository list must recreate the template, not just upgrade it.
pub(super) fn config_hash() -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    let path = Path::new("/etc/xpm.conf");
    if let Ok(bytes) = fs::read(path) {
        path.hash(&mut hasher);
        bytes.hash(&mut hasher);
    }
    format!("{:016x}", hasher.finish())
}

pub(super) fn write_marker(work: &Path, name: &str, value: &str) -> BuildResult<()> {
    write_root_file(&work.join(name), value.as_bytes())
}

pub(super) fn write_root_file(path: &Path, bytes: &[u8]) -> BuildResult<()> {
    let mut child = root_command("tee")
        .arg(path)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .map_err(|err| BuildError::Command(format!("write {}: {err}", path.display())))?;
    if let Some(stdin) = child.stdin.as_mut() {
        stdin
            .write_all(bytes)
            .map_err(|err| BuildError::Command(format!("write {}: {err}", path.display())))?;
    }
    let status = child
        .wait()
        .map_err(|err| BuildError::Command(format!("write {}: {err}", path.display())))?;
    if !status.success() {
        return Err(BuildError::Command(format!(
            "could not write {}",
            path.display()
        )));
    }
    Ok(())
}

pub(super) fn open_file_lock(path: &Path) -> BuildResult<File> {
    if !path.exists() {
        let _ = root_command("touch").arg(path).status();
        let _ = root_command("chmod").args(["0644"]).arg(path).status();
    }
    let file = OpenOptions::new()
        .read(true)
        .open(path)
        .or_else(|_| OpenOptions::new().read(true).write(true).open(path))
        .map_err(|source| BuildError::Io {
            path: path.display().to_string(),
            source,
        })?;
    let locked = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
    if locked != 0 {
        let source = std::io::Error::last_os_error();
        return Err(BuildError::Command(format!(
            "could not lock {}: {source}",
            path.display()
        )));
    }
    Ok(file)
}

pub(super) fn run_root<const N: usize>(program: &str, args: [&OsStr; N]) -> BuildResult<()> {
    let status = root_command(program)
        .args(args)
        .status()
        .map_err(|err| BuildError::Command(format!("{program}: {err}")))?;
    if status.success() {
        Ok(())
    } else {
        Err(BuildError::Command(format!(
            "{program} exited with status {:?}",
            status.code()
        )))
    }
}

pub(crate) fn root_command(program: impl AsRef<OsStr>) -> Command {
    if effective_uid() == 0 {
        Command::new(program)
    } else {
        let mut command = Command::new("sudo");
        command.arg(program);
        command
    }
}

pub(super) fn current_xpm() -> BuildResult<PathBuf> {
    std::env::current_exe()
        .map_err(|err| BuildError::Command(format!("resolve the xpm executable: {err}")))
}

pub(super) fn command_exists(program: &str) -> bool {
    Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {program} >/dev/null 2>&1"))
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

pub(crate) fn effective_uid() -> u32 {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata("/proc/self")
        .map(|metadata| metadata.uid())
        .unwrap_or(0)
}

pub(super) fn build_user_uid() -> u32 {
    invoking_uid().unwrap_or(1000)
}

pub(super) fn artifact_user_uid() -> u32 {
    if effective_uid() == 0 {
        invoking_uid().unwrap_or(0)
    } else {
        effective_uid()
    }
}

pub(super) fn invoking_uid() -> Option<u32> {
    std::env::var("SUDO_UID")
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|uid| *uid != 0)
}

pub(super) fn current_arch() -> String {
    std::env::consts::ARCH.to_string()
}

pub(super) fn now_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

pub(super) fn now_epoch_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0)
}

pub(super) fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
