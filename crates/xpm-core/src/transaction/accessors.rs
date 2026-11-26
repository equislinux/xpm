//! Logging and state accessors.

use super::*;

impl Transaction {
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
