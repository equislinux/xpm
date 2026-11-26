//! Filesystem lock preventing concurrent xpm operations.

use super::*;

/// A filesystem-based lock to prevent concurrent xpm operations.
pub struct FileLock {
    path: PathBuf,
    _guard: Option<File>,
}

fn lock_is_stale(path: &Path) -> bool {
    let Ok(metadata) = fs::metadata(path) else {
        return true;
    };
    let age = metadata
        .modified()
        .ok()
        .and_then(|modified| SystemTime::now().duration_since(modified).ok())
        .map(|age| age.as_secs())
        .unwrap_or(0);
    let raw = fs::read_to_string(path).unwrap_or_default();
    let pid = raw.trim();

    if let Ok(pid) = pid.parse::<u32>() {
        return !Path::new(&format!("/proc/{pid}")).exists();
    }

    // Legacy/empty lock: only reclaim it after a short grace period so a
    // concurrent creator that has not written its PID yet is not preempted.
    pid.is_empty() && age > 5
}

impl Drop for FileLock {
    fn drop(&mut self) {
        self._guard = None;
        let _ = fs::remove_file(&self.path);
    }
}

impl FileLock {
    /// Acquire an exclusive lock at the given path.
    /// Other processes will wait or fail if already locked.
    pub fn acquire(path: &Path) -> XpmResult<Self> {
        use std::os::unix::fs::OpenOptionsExt;

        fs::create_dir_all(path.parent().unwrap_or_else(|| Path::new("/")))?;

        for attempt in 0..2 {
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(path)
            {
                Ok(mut file) => {
                    let _ = writeln!(file, "{}", std::process::id());
                    let _ = file.flush();
                    return Ok(FileLock {
                        path: path.to_path_buf(),
                        _guard: Some(file),
                    });
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists && attempt == 0 => {
                    if lock_is_stale(path) {
                        let _ = fs::remove_file(path);
                        continue;
                    }
                    return Err(XpmError::Transaction(format!(
                        "unable to lock database: {} exists (another package manager may be running)",
                        path.display()
                    )));
                }
                Err(e) => {
                    return Err(XpmError::Transaction(format!(
                        "failed to acquire lock {}: {e}",
                        path.display()
                    )));
                }
            }
        }

        Err(XpmError::Transaction(format!(
            "unable to lock database: {}",
            path.display()
        )))
    }

    /// Release the lock by dropping the file handle.
    pub fn release(&mut self) -> XpmResult<()> {
        self._guard = None;
        // Optionally remove the lock file
        let _ = fs::remove_file(&self.path);
        Ok(())
    }
}
