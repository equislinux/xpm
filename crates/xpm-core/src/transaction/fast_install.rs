//! Parallel extraction helpers for pure installs.

use super::*;

pub(super) fn extraction_workers(batch_len: usize) -> usize {
    let env_workers = std::env::var("XPM_EXTRACT_WORKERS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0);
    let available = std::thread::available_parallelism()
        .map(|value| value.get())
        .unwrap_or(4);
    env_workers
        .unwrap_or_else(|| available.min(8))
        .min(batch_len)
        .max(1)
}

pub(super) fn run_fast_install(
    tx: &Transaction,
    hooks: &HookChain,
    op: &TransactionOp,
    metadata: &Arc<PackageMetadata>,
) -> XpmResult<()> {
    run_fast_install_parts(
        hooks,
        &tx.root_dir,
        &tx.local_db_dir,
        tx.save_configs,
        tx.scriptlet_mode,
        op,
        metadata,
    )
}

pub(super) fn run_fast_install_parts(
    hooks: &HookChain,
    root_dir: &Path,
    local_db_dir: &Path,
    save_configs: bool,
    scriptlet_mode: ScriptletMode,
    op: &TransactionOp,
    metadata: &Arc<PackageMetadata>,
) -> XpmResult<()> {
    let TransactionOp::Install {
        pkg_name,
        pkg_version,
        pkg_file,
        reason,
        origin,
        ..
    } = op
    else {
        return Err(XpmError::Transaction(
            "parallel batch contains a non-install operation".to_string(),
        ));
    };
    let context = HookContext {
        operation_type: OperationType::Install,
        pkg_name: pkg_name.clone(),
        pkg_version: pkg_version.clone(),
        old_version: None,
        pkg_file: Some(pkg_file.clone()),
        root_dir: root_dir.to_path_buf(),
        local_db_dir: local_db_dir.to_path_buf(),
        save_configs,
        shell_integration: false,
        scriptlet_mode,
        metadata: Some(metadata.clone()),
        transaction_paths: None,
        reason: *reason,
        origin: origin.clone(),
    };
    hooks.run(&context)
}

pub(super) fn print_operation_progress(index: usize, total: usize, op: &TransactionOp) {
    if !progress::verbose() {
        return;
    }
    match op {
        TransactionOp::Install {
            pkg_name,
            pkg_version,
            ..
        } => {
            eprintln!("({index}/{total}) installing {pkg_name} {pkg_version}");
        }
        TransactionOp::Upgrade {
            pkg_name,
            new_version,
            ..
        } => {
            eprintln!("({index}/{total}) upgrading {pkg_name} {new_version}");
        }
        TransactionOp::Remove { pkg_name } => {
            eprintln!("({index}/{total}) removing {pkg_name}");
        }
    }
}
