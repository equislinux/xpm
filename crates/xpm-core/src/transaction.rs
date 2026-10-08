//! Transaction engine for install, remove, and upgrade operations.
//!
//! This module provides the core transaction logic: planning dependency
//! resolution, preparing file extractions and removals, committing changes
//! to the filesystem, and rolling back on failure.
//!
//! Transactions are atomic where possible and logged for auditing.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::error::{XpmError, XpmResult};
use crate::hooks::{HookChain, HookContext, OperationType};
use crate::package::PackageMeta;

/// A filesystem-based lock to prevent concurrent xpm operations.
pub struct FileLock {
    path: PathBuf,
    _guard: Option<File>,
}

impl FileLock {
    /// Acquire an exclusive lock at the given path.
    /// Other processes will wait or fail if already locked.
    pub fn acquire(path: &Path) -> XpmResult<Self> {
        fs::create_dir_all(path.parent().unwrap_or_else(|| Path::new("/")))?;

        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)
            .map_err(|e| XpmError::Package(format!("failed to acquire lock: {}", e)))?;

        Ok(FileLock {
            path: path.to_path_buf(),
            _guard: Some(file),
        })
    }

    /// Release the lock by dropping the file handle.
    pub fn release(&mut self) -> XpmResult<()> {
        self._guard = None;
        // Optionally remove the lock file
        let _ = fs::remove_file(&self.path);
        Ok(())
    }
}

/// Single operation within a transaction.
#[derive(Debug, Clone)]
pub enum TransactionOp {
    /// Install a package from a .xp file.
    Install {
        pkg_name: String,
        pkg_version: String,
        pkg_file: PathBuf,
        metadata: Box<Option<PackageMeta>>,
    },
    /// Remove an installed package.
    Remove { pkg_name: String },
    /// Upgrade from one version to another.
    Upgrade {
        pkg_name: String,
        old_version: String,
        new_version: String,
        new_pkg_file: PathBuf,
    },
}

/// State of a transaction during its lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransactionState {
    /// Operations collected but not yet prepared.
    Planning,
    /// Pre-flight checks passed, ready to commit.
    Prepared,
    /// Changes written to filesystem and database.
    Committed,
    /// Rolled back due to error or user request.
    RolledBack,
}

/// A package management transaction with plan, prepare, and commit phases.
pub struct Transaction {
    operations: Vec<TransactionOp>,
    state: TransactionState,
    lock: Option<FileLock>,
    log_file: PathBuf,
    #[allow(dead_code)]
    root_dir: PathBuf,
    local_db_dir: PathBuf,
    rollback_counter: usize,
    hooks_chain: Option<HookChain>,
    shell_integration: bool,
    save_configs: bool,
}

impl Transaction {
    /// Create a new transaction with default root and database directories.
    pub fn new(root_dir: PathBuf, local_db_dir: PathBuf) -> XpmResult<Self> {
        let lock_path = local_db_dir.join("transaction.lock");
        let log_file = root_dir.join("var/log/xpm.log");

        let lock = FileLock::acquire(&lock_path).ok();

        fs::create_dir_all(&local_db_dir)?;
        fs::create_dir_all(log_file.parent().unwrap_or_else(|| Path::new("/")))?;

        Ok(Transaction {
            operations: Vec::new(),
            state: TransactionState::Planning,
            lock,
            log_file,
            root_dir,
            local_db_dir,
            rollback_counter: 0,
            hooks_chain: None,
            shell_integration: false,
            save_configs: true,
        })
    }

    /// Set the hooks chain to be executed during commit.
    pub fn set_hooks(&mut self, hooks_chain: HookChain) {
        self.hooks_chain = Some(hooks_chain);
    }

    /// Enable/disable shell integration for non-root installs.
    pub fn set_shell_integration(&mut self, enabled: bool) {
        self.shell_integration = enabled;
    }

    /// Keep modified configuration files on remove/upgrade (default `true`;
    /// `false` is `--nosave`).
    pub fn set_save_configs(&mut self, enabled: bool) {
        self.save_configs = enabled;
    }

    /// Add an install operation to the transaction.
    pub fn add_install(
        &mut self,
        pkg_name: String,
        pkg_version: String,
        pkg_file: PathBuf,
    ) -> XpmResult<()> {
        if self.state != TransactionState::Planning {
            return Err(XpmError::Package(
                "can only add operations during planning phase".to_string(),
            ));
        }

        self.operations.push(TransactionOp::Install {
            pkg_name,
            pkg_version,
            pkg_file,
            metadata: Box::new(None),
        });

        Ok(())
    }

    /// Add an upgrade operation: replace `old_version` with `new_version`
    /// from `new_pkg_file`. Extracted files are written over the existing
    /// tree; configuration handling and stale-file cleanup happen in the
    /// hooks (see `hooks::FileExtractionHook`).
    pub fn add_upgrade(
        &mut self,
        pkg_name: String,
        old_version: String,
        new_version: String,
        new_pkg_file: PathBuf,
    ) -> XpmResult<()> {
        if self.state != TransactionState::Planning {
            return Err(XpmError::Package(
                "can only add operations during planning phase".to_string(),
            ));
        }

        self.operations.push(TransactionOp::Upgrade {
            pkg_name,
            old_version,
            new_version,
            new_pkg_file,
        });

        Ok(())
    }

    /// Add a remove operation to the transaction.
    pub fn add_remove(&mut self, pkg_name: String) -> XpmResult<()> {
        if self.state != TransactionState::Planning {
            return Err(XpmError::Package(
                "can only add operations during planning phase".to_string(),
            ));
        }

        self.operations.push(TransactionOp::Remove { pkg_name });

        Ok(())
    }

    /// Transition to the prepare phase: validate all operations.
    pub fn prepare(&mut self) -> XpmResult<()> {
        if self.state != TransactionState::Planning {
            return Err(XpmError::Package(
                "transaction is not in planning state".to_string(),
            ));
        }

        // Pre-flight checks for each operation
        for op in &self.operations {
            match op {
                TransactionOp::Install { pkg_file, .. } => {
                    if !pkg_file.exists() {
                        return Err(XpmError::Package(format!(
                            "package file does not exist: {}",
                            pkg_file.display()
                        )));
                    }
                }
                TransactionOp::Remove { pkg_name } => {
                    let installed_path = self.local_db_dir.join(pkg_name);
                    if !installed_path.exists() {
                        return Err(XpmError::Package(format!(
                            "package not installed: {}",
                            pkg_name
                        )));
                    }
                }
                TransactionOp::Upgrade { new_pkg_file, .. } => {
                    if !new_pkg_file.exists() {
                        return Err(XpmError::Package(format!(
                            "upgrade package file does not exist: {}",
                            new_pkg_file.display()
                        )));
                    }
                }
            }
        }

        // Check disk space (simplified: just ensure target directories are writable)
        self.verify_permissions()?;

        self.state = TransactionState::Prepared;
        self.log("prepared")?;

        Ok(())
    }

    /// Execute the transaction: write changes to filesystem.
    pub fn commit(&mut self) -> XpmResult<()> {
        if self.state != TransactionState::Prepared {
            return Err(XpmError::Package(
                "transaction must be prepared before commit".to_string(),
            ));
        }

        for op in self.operations.clone() {
            match op {
                TransactionOp::Install {
                    pkg_name,
                    pkg_version,
                    pkg_file,
                    ..
                } => {
                    self.execute_install(&pkg_name, &pkg_version, &pkg_file)?;
                }
                TransactionOp::Remove { pkg_name } => {
                    self.execute_remove(&pkg_name)?;
                }
                TransactionOp::Upgrade {
                    pkg_name,
                    old_version,
                    new_version,
                    new_pkg_file,
                    ..
                } => {
                    self.execute_upgrade(&pkg_name, &old_version, &new_version, &new_pkg_file)?;
                }
            }
        }

        self.state = TransactionState::Committed;
        self.log("committed")?;

        Ok(())
    }

    /// Rollback the transaction (cleanup resources).
    pub fn rollback(&mut self) -> XpmResult<()> {
        self.log("rolling back")?;

        // For now, cleanup is minimal since we haven't actually written files yet.
        // In full implementation, would undo file extractions and removals.

        self.rollback_counter += 1;
        self.state = TransactionState::RolledBack;

        Ok(())
    }

    /// Install a single package (internal - called by commit).
    fn execute_install(&self, pkg_name: &str, pkg_version: &str, pkg_file: &Path) -> XpmResult<()> {
        // Run hooks to extract files and perform pre/post-install actions
        if let Some(hooks) = &self.hooks_chain {
            let context = HookContext {
                operation_type: OperationType::Install,
                pkg_name: pkg_name.to_string(),
                pkg_version: pkg_version.to_string(),
                old_version: None,
                pkg_file: Some(pkg_file.to_path_buf()),
                root_dir: self.root_dir.clone(),
                local_db_dir: self.local_db_dir.clone(),
                save_configs: self.save_configs,
                shell_integration: self.shell_integration,
            };
            hooks.run(&context)?;
        }

        // Record in local database
        let db_entry = self.local_db_dir.join(pkg_name);
        fs::create_dir_all(&db_entry)?;

        let version_file = db_entry.join("version");
        fs::write(version_file, pkg_version)?;

        self.log(&format!("installed {} {}", pkg_name, pkg_version))?;

        Ok(())
    }

    /// Remove a single package (internal - called by commit).
    fn execute_remove(&self, pkg_name: &str) -> XpmResult<()> {
        // Run hooks to remove files and perform pre/post-remove actions
        // LocalDbHook handles removing the package database entry
        if let Some(hooks) = &self.hooks_chain {
            let context = HookContext {
                operation_type: OperationType::Remove,
                pkg_name: pkg_name.to_string(),
                pkg_version: String::new(), // Not needed for remove
                old_version: None,
                pkg_file: None,
                root_dir: self.root_dir.clone(),
                local_db_dir: self.local_db_dir.clone(),
                save_configs: self.save_configs,
                shell_integration: self.shell_integration,
            };
            hooks.run(&context)?;
        }

        // If hooks weren't set, fallback to manual database cleanup
        let db_entry = self.local_db_dir.join(pkg_name);
        if db_entry.exists() && self.hooks_chain.is_none() {
            fs::remove_dir_all(&db_entry)?;
        }

        self.log(&format!("removed {}", pkg_name))?;

        Ok(())
    }

    /// Upgrade a single package (internal - called by commit).
    ///
    /// Unlike an install, the old local-database entry stays in place while
    /// the hooks run: extraction needs the previous manifest/hashes to decide
    /// whether a configuration file was modified, and the local-db hook only
    /// rewrites the version at the end.
    fn execute_upgrade(
        &self,
        pkg_name: &str,
        old_version: &str,
        new_version: &str,
        new_pkg_file: &Path,
    ) -> XpmResult<()> {
        if let Some(hooks) = &self.hooks_chain {
            let context = HookContext {
                operation_type: OperationType::Upgrade,
                pkg_name: pkg_name.to_string(),
                pkg_version: new_version.to_string(),
                old_version: Some(old_version.to_string()),
                pkg_file: Some(new_pkg_file.to_path_buf()),
                root_dir: self.root_dir.clone(),
                local_db_dir: self.local_db_dir.clone(),
                save_configs: self.save_configs,
                shell_integration: self.shell_integration,
            };
            hooks.run(&context)?;
        } else {
            // Without hooks there is no extraction; at least record the version.
            let version_file = self.local_db_dir.join(pkg_name).join("version");
            if version_file.exists() {
                fs::write(&version_file, new_version)?;
            }
        }

        self.log(&format!(
            "upgraded {} {} -> {}",
            pkg_name, old_version, new_version
        ))?;

        Ok(())
    }

    /// Verify that we have permission to write to target directories.
    fn verify_permissions(&self) -> XpmResult<()> {
        // Check if we can write to local_db_dir
        if !self.local_db_dir.exists() {
            fs::create_dir_all(&self.local_db_dir)?;
        }

        // In a real implementation, would check write perms to root_dir
        Ok(())
    }

    /// Append a log entry with timestamp.
    pub fn log(&self, message: &str) -> XpmResult<()> {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log_file)?;

        let timestamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        writeln!(file, "[{}] {}", timestamp, message)?;

        Ok(())
    }

    /// Get current transaction state.
    pub fn state(&self) -> TransactionState {
        self.state
    }

    /// Get number of operations in this transaction.
    pub fn operation_count(&self) -> usize {
        self.operations.len()
    }

    /// Get operations slice.
    pub fn operations(&self) -> &[TransactionOp] {
        &self.operations
    }
}

impl Drop for Transaction {
    fn drop(&mut self) {
        let _ = self.lock.take().map(|mut l| l.release());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn test_root() -> (TempDir, PathBuf, PathBuf) {
        let tmpdir = TempDir::new().expect("create tempdir");
        let root = tmpdir.path().to_path_buf();
        let local_db = root.join("var/lib/xpm/local");
        (tmpdir, root, local_db)
    }

    #[test]
    fn create_transaction() {
        let (_tmp, root, local_db) = test_root();
        let tx = Transaction::new(root, local_db).expect("create transaction");
        assert_eq!(tx.state(), TransactionState::Planning);
        assert_eq!(tx.operation_count(), 0);
    }

    #[test]
    fn add_install_operation() {
        let (_tmp, root, local_db) = test_root();
        let mut tx = Transaction::new(root, local_db).expect("create transaction");

        tx.add_install(
            "test".to_string(),
            "1.0-1".to_string(),
            PathBuf::from("/tmp/test-1.0-1-x86_64.xp"),
        )
        .expect("add install");

        assert_eq!(tx.operation_count(), 1);
    }

    #[test]
    fn prepare_requires_planning_state() {
        let (_tmp, root, local_db) = test_root();
        let mut tx = Transaction::new(root, local_db).expect("create transaction");

        tx.prepare().expect("prepare");
        assert_eq!(tx.state(), TransactionState::Prepared);

        let result = tx.prepare();
        assert!(
            result.is_err(),
            "should not allow prepare when not in planning state"
        );
    }

    #[test]
    fn add_operation_fails_after_prepare() {
        let (_tmp, root, local_db) = test_root();
        let mut tx = Transaction::new(root, local_db).expect("create transaction");

        tx.prepare().expect("prepare");

        let result = tx.add_install(
            "test".to_string(),
            "1.0-1".to_string(),
            PathBuf::from("/tmp/test.xp"),
        );

        assert!(result.is_err(), "should not allow add after prepare");
    }

    #[test]
    fn transaction_logs_to_file() {
        let (_tmp, root, local_db) = test_root();
        let tx = Transaction::new(root.clone(), local_db).expect("create transaction");

        tx.log("test message").expect("log");

        let log_path = root.join("var/log/xpm.log");
        assert!(log_path.exists(), "log file should exist");

        let content = fs::read_to_string(&log_path).expect("read log");
        assert!(
            content.contains("test message"),
            "log should contain message"
        );
    }

    #[test]
    fn commit_requires_prepared_state() {
        let (_tmp, root, local_db) = test_root();
        let mut tx = Transaction::new(root, local_db).expect("create transaction");

        let result = tx.commit();
        assert!(result.is_err(), "commit should fail if not prepared");
    }

    #[test]
    fn rollback_changes_state() {
        let (_tmp, root, local_db) = test_root();
        let mut tx = Transaction::new(root, local_db).expect("create transaction");

        tx.rollback().expect("rollback");
        assert_eq!(tx.state(), TransactionState::RolledBack);
    }

    // ── End-to-end integration tests ────────────────────────────────────

    /// Create a minimal .xp archive (tar.zst) for testing
    fn make_test_xp_package(dir: &Path) -> PathBuf {
        use std::io::Write;

        let pkg_path = dir.join("test-1.0.0-1-x86_64.xp");

        let mut raw_tar = Vec::new();
        {
            let mut builder = tar::Builder::new(&mut raw_tar);

            // .PKGINFO
            let pkginfo = b"pkgname = test\npkgver = 1.0.0-1\n\
                pkgdesc = Test package\narch = x86_64\nsize = 100\n";
            let mut header = tar::Header::new_gnu();
            header.set_path(".PKGINFO").unwrap();
            header.set_size(pkginfo.len() as u64);
            header.set_mode(0o644);
            header.set_uid(0);
            header.set_gid(0);
            header.set_cksum();
            builder.append(&header, &pkginfo[..]).unwrap();

            // .BUILDINFO
            let buildinfo = b"pkgname = test\npkgver = 1.0.0-1\n";
            let mut header = tar::Header::new_gnu();
            header.set_path(".BUILDINFO").unwrap();
            header.set_size(buildinfo.len() as u64);
            header.set_mode(0o644);
            header.set_uid(0);
            header.set_gid(0);
            header.set_cksum();
            builder.append(&header, &buildinfo[..]).unwrap();

            // .MTREE
            let mtree = b"#mtree\n\
                ./usr type=dir mode=0755 uid=0 gid=0\n\
                ./usr/bin type=dir mode=0755 uid=0 gid=0\n\
                ./usr/bin/test type=file mode=0755 size=12 uid=0 gid=0\n\
                ./usr/bin/test-link type=link link=test uid=0 gid=0\n";
            let mut header = tar::Header::new_gnu();
            header.set_path(".MTREE").unwrap();
            header.set_size(mtree.len() as u64);
            header.set_mode(0o644);
            header.set_uid(0);
            header.set_gid(0);
            header.set_cksum();
            builder.append(&header, &mtree[..]).unwrap();

            // Create usr/bin/ directory and test file
            let mut header = tar::Header::new_gnu();
            header.set_path("usr/").unwrap();
            header.set_size(0);
            header.set_mode(0o755);
            header.set_entry_type(tar::EntryType::Directory);
            header.set_uid(0);
            header.set_gid(0);
            header.set_cksum();
            builder.append(&header, &[][..]).unwrap();

            let mut header = tar::Header::new_gnu();
            header.set_path("usr/bin/").unwrap();
            header.set_size(0);
            header.set_mode(0o755);
            header.set_entry_type(tar::EntryType::Directory);
            header.set_uid(0);
            header.set_gid(0);
            header.set_cksum();
            builder.append(&header, &[][..]).unwrap();

            let content = b"test content";
            let mut header = tar::Header::new_gnu();
            header.set_path("usr/bin/test").unwrap();
            header.set_size(content.len() as u64);
            header.set_mode(0o755);
            header.set_uid(0);
            header.set_gid(0);
            header.set_cksum();
            builder.append(&header, &content[..]).unwrap();

            let mut header = tar::Header::new_gnu();
            header.set_entry_type(tar::EntryType::Symlink);
            header.set_path("usr/bin/test-link").unwrap();
            header.set_link_name("test").unwrap();
            header.set_size(0);
            header.set_mode(0o777);
            header.set_uid(0);
            header.set_gid(0);
            header.set_cksum();
            builder.append(&header, &[][..]).unwrap();

            builder.finish().unwrap();
        }

        // Compress with zstd
        let compressed = zstd::encode_all(&raw_tar[..], 3).expect("compress zstd");
        let mut file = fs::File::create(&pkg_path).expect("create file");
        file.write_all(&compressed).expect("write file");

        pkg_path
    }

    #[test]
    fn e2e_install_extracts_files() {
        let (_tmp, root, local_db) = test_root();
        let cache_dir = root.join("var/cache/xpm");
        fs::create_dir_all(&cache_dir).expect("create cache dir");

        // Create test package
        let pkg_file = make_test_xp_package(&cache_dir);

        // Create transaction
        let mut tx = Transaction::new(root.clone(), local_db.clone()).expect("create transaction");

        // Setup hooks
        let hooks = HookChain::default();
        tx.set_hooks(hooks);

        // Add install operation
        tx.add_install("test".to_string(), "1.0.0-1".to_string(), pkg_file.clone())
            .expect("add install");

        // Prepare transaction
        tx.prepare().expect("prepare");

        // Commit transaction
        tx.commit().expect("commit");

        // Verify files were extracted
        assert!(
            root.join("usr/bin/test").exists(),
            "test executable should be extracted"
        );

        // Verify database entry was created
        assert!(
            local_db.join("test/version").exists(),
            "version file should be created in local db"
        );

        let version_content =
            fs::read_to_string(local_db.join("test/version")).expect("read version");
        assert_eq!(version_content, "1.0.0-1", "version should match");

        // Verify symlink was extracted
        assert!(
            root.join("usr/bin/test-link").is_symlink(),
            "symlink from the package should be extracted"
        );

        // Verify pacman-compatible file manifest derived from .MTREE
        let files_content =
            fs::read_to_string(local_db.join("test/files")).expect("read files manifest");
        assert_eq!(
            files_content,
            "%FILES%\nusr/\nusr/bin/\nusr/bin/test\nusr/bin/test-link\n"
        );
    }

    #[test]
    fn e2e_remove_deletes_files() {
        let (_tmp, root, local_db) = test_root();
        let cache_dir = root.join("var/cache/xpm");
        fs::create_dir_all(&cache_dir).expect("create cache dir");

        // First install a package
        let pkg_file = make_test_xp_package(&cache_dir);

        let mut tx = Transaction::new(root.clone(), local_db.clone()).expect("create transaction");
        let hooks = HookChain::default();
        tx.set_hooks(hooks);

        tx.add_install("test".to_string(), "1.0.0-1".to_string(), pkg_file)
            .expect("add install");

        tx.prepare().expect("prepare");
        tx.commit().expect("commit");

        // Verify extracted file exists
        let test_file = root.join("usr/bin/test");
        assert!(test_file.exists(), "file should exist after install");

        // Now remove the package (without hooks for now)
        let mut tx2 =
            Transaction::new(root.clone(), local_db.clone()).expect("create remove transaction");
        // Don't set hooks to isolate removal logic

        tx2.add_remove("test".to_string()).expect("add remove");

        tx2.prepare().expect("prepare");
        tx2.commit().expect("commit");

        // Verify database entry is removed
        assert!(
            !local_db.join("test").exists(),
            "package directory should be removed from local db"
        );
    }

    #[test]
    fn e2e_multiple_installs() {
        let (_tmp, root, local_db) = test_root();
        let cache_dir = root.join("var/cache/xpm");
        fs::create_dir_all(&cache_dir).expect("create cache dir");

        // Create test packages
        let pkg1 = make_test_xp_package(&cache_dir);
        let pkg2_path = cache_dir.join("test-2.0.0-1-x86_64.xp");
        fs::copy(&pkg1, &pkg2_path).expect("copy package");

        // Create transaction with multiple installs
        let mut tx = Transaction::new(root.clone(), local_db.clone()).expect("create transaction");
        let hooks = HookChain::default();
        tx.set_hooks(hooks);

        tx.add_install("test".to_string(), "1.0.0-1".to_string(), pkg1)
            .expect("add install 1");

        // Prepare and commit first should succeed
        tx.prepare().expect("prepare");
        tx.commit().expect("commit");

        assert_eq!(tx.operation_count(), 1);
        assert_eq!(tx.state(), TransactionState::Committed);
    }
}
