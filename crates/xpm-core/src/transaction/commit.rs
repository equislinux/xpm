//! Commit phase: extract, remove and journal the transaction.

use super::*;

impl Transaction {
    /// Execute the transaction: write changes to filesystem.
    ///
    /// Pure installs without `.INSTALL` scriptlets and without shell shims are
    /// independent: their hook chains touch separate package directories and
    /// files already checked for conflicts. They are extracted in parallel in
    /// consecutive batches; upgrades, removals and scriptlet packages remain
    /// strictly sequential to preserve lifecycle ordering.
    pub fn commit(&mut self) -> XpmResult<()> {
        if self.state != TransactionState::Prepared {
            return Err(XpmError::Package(
                "transaction must be prepared before commit".to_string(),
            ));
        }

        let operations = self.operations.clone();
        let total = operations.len();
        let install_total = operations
            .iter()
            .filter(|op| {
                matches!(
                    op,
                    TransactionOp::Install { .. } | TransactionOp::Upgrade { .. }
                )
            })
            .count();
        let remove_total = operations
            .iter()
            .filter(|op| matches!(op, TransactionOp::Remove { .. }))
            .count();
        let (label, progress_total) = if remove_total == 0 {
            ("Installing packages", install_total)
        } else if install_total == 0 {
            ("Removing packages", remove_total)
        } else {
            ("Applying package changes", total)
        };
        let operation_progress = (progress_total > 0)
            .then(|| OperationProgress::new(label, progress_total, BarFill::Slash));

        let mut index = 0;
        while index < total {
            let op = &operations[index];
            if self.fast_install_metadata(op).is_some() {
                let mut batch = Vec::new();
                while index < total {
                    let candidate = &operations[index];
                    let Some(metadata) = self.fast_install_metadata(candidate) else {
                        break;
                    };
                    batch.push((candidate.clone(), metadata));
                    index += 1;
                }
                self.commit_parallel_batch(&batch)?;
                for (op, _) in &batch {
                    if let TransactionOp::Install {
                        pkg_name,
                        pkg_version,
                        ..
                    } = op
                    {
                        self.write_version(pkg_name, pkg_version)?;
                        self.log(&format!("installed {} {}", pkg_name, pkg_version))?;
                    }
                    if let Some(progress) = &operation_progress {
                        progress.advance();
                    }
                }
                continue;
            }

            print_operation_progress(index + 1, total, op);
            match op.clone() {
                TransactionOp::Install {
                    pkg_name,
                    pkg_version,
                    pkg_file,
                    reason,
                    origin,
                    ..
                } => {
                    self.execute_install(
                        &pkg_name,
                        &pkg_version,
                        &pkg_file,
                        reason,
                        origin.as_deref(),
                    )?;
                }
                TransactionOp::Remove { pkg_name } => {
                    self.execute_remove(&pkg_name)?;
                }
                TransactionOp::Upgrade {
                    pkg_name,
                    old_version,
                    new_version,
                    new_pkg_file,
                    reason,
                    origin,
                    ..
                } => {
                    self.execute_upgrade(
                        &pkg_name,
                        &old_version,
                        &new_version,
                        &new_pkg_file,
                        reason,
                        origin.as_deref(),
                    )?;
                }
            }
            if let Some(progress) = &operation_progress {
                progress.advance();
            }
            index += 1;
        }
        if let Some(progress) = &operation_progress {
            progress.finish();
        }

        self.state = TransactionState::Committed;
        self.log("committed")?;

        Ok(())
    }

    /// Metadata for a pure install that can safely be extracted in parallel.
    pub(super) fn fast_install_metadata(&self, op: &TransactionOp) -> Option<Arc<PackageMetadata>> {
        if self.shell_integration || self.hooks_chain.is_none() {
            return None;
        }
        let TransactionOp::Install { pkg_file, .. } = op else {
            return None;
        };
        let metadata = self.metadata_cache.get(pkg_file)?;
        // Any .INSTALL scriptlet keeps the package on the sequential path:
        // pre/post hooks and package order must remain observable.
        if metadata.install_script.is_some() {
            return None;
        }
        Some(metadata.clone())
    }

    /// Extract a batch of pure installs using all available cores (capped).
    pub(super) fn commit_parallel_batch(
        &self,
        batch: &[(TransactionOp, Arc<PackageMetadata>)],
    ) -> XpmResult<()> {
        let Some(hooks) = self.hooks_chain.as_ref() else {
            return Err(XpmError::Transaction(
                "parallel install batch requires a hook chain".to_string(),
            ));
        };
        let workers = extraction_workers(batch.len());
        if workers <= 1 {
            for (op, metadata) in batch {
                run_fast_install(self, hooks, op, metadata)?;
            }
            return Ok(());
        }

        let next = AtomicUsize::new(0);
        let errors: Mutex<Option<String>> = Mutex::new(None);
        let root_dir = self.root_dir.clone();
        let local_db_dir = self.local_db_dir.clone();
        let save_configs = self.save_configs;
        let scriptlet_mode = self.scriptlet_mode;
        std::thread::scope(|scope| {
            for _ in 0..workers {
                scope.spawn(|| loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some((op, metadata)) = batch.get(index) else {
                        break;
                    };
                    let result = run_fast_install_parts(
                        hooks,
                        &root_dir,
                        &local_db_dir,
                        save_configs,
                        scriptlet_mode,
                        op,
                        metadata,
                    );
                    if let Err(err) = result {
                        if let Ok(mut slot) = errors.lock() {
                            if slot.is_none() {
                                *slot = Some(format!("{err}"));
                            }
                        }
                        break;
                    }
                });
            }
        });

        let errors = errors
            .into_inner()
            .unwrap_or_else(|poison| poison.into_inner());
        if let Some(err) = errors {
            return Err(XpmError::Transaction(format!(
                "parallel install failed: {err}"
            )));
        }
        Ok(())
    }

    pub(super) fn write_version(&self, pkg_name: &str, pkg_version: &str) -> XpmResult<()> {
        let db_entry = self.local_db_dir.join(pkg_name);
        fs::create_dir_all(&db_entry)?;
        fs::write(db_entry.join("version"), pkg_version)?;
        Ok(())
    }
}
