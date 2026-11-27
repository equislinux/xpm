//! Per-operation filesystem execution and permission checks.

use super::*;

impl Transaction {
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
    pub(super) fn execute_install(
        &self,
        pkg_name: &str,
        pkg_version: &str,
        pkg_file: &Path,
        reason: InstallReason,
        origin: Option<&str>,
    ) -> XpmResult<()> {
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
                scriptlet_mode: self.scriptlet_mode,
                metadata: self.metadata_cache.get(pkg_file).cloned(),
                transaction_paths: self.transaction_paths.clone(),
                reason,
                origin: origin.map(str::to_string),
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
    pub(super) fn execute_remove(&self, pkg_name: &str) -> XpmResult<()> {
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
                scriptlet_mode: self.scriptlet_mode,
                metadata: None,
                transaction_paths: self.transaction_paths.clone(),
                reason: InstallReason::Explicit,
                origin: None,
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
    pub(super) fn execute_upgrade(
        &self,
        pkg_name: &str,
        old_version: &str,
        new_version: &str,
        new_pkg_file: &Path,
        reason: InstallReason,
        origin: Option<&str>,
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
                scriptlet_mode: self.scriptlet_mode,
                metadata: self.metadata_cache.get(new_pkg_file).cloned(),
                transaction_paths: self.transaction_paths.clone(),
                reason,
                origin: origin.map(str::to_string),
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
    pub(super) fn verify_permissions(&self) -> XpmResult<()> {
        // Check if we can write to local_db_dir
        if !self.local_db_dir.exists() {
            fs::create_dir_all(&self.local_db_dir)?;
        }

        // In a real implementation, would check write perms to root_dir
        Ok(())
    }
}
