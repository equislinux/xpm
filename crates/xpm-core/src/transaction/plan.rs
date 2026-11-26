//! Planning phase: collect install/upgrade/remove operations.

use super::*;

impl Transaction {
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
}
