//! Prepare phase: validate operations before touching the filesystem.

use super::*;

impl Transaction {
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
}
