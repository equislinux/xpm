//! Installed-file verification.

use super::*;
use xpm_core::check;

pub(super) fn cmd_check(config: &XpmConfig, args: &cli::CheckArgs) -> Result<()> {
    let local_db_dir = config.options.db_path.join("local");
    let installed = read_installed_versions(&local_db_dir)?;

    let targets = if args.packages.is_empty() {
        let mut names: Vec<String> = installed.keys().cloned().collect();
        names.sort();
        names
    } else {
        let mut names = Vec::new();
        for name in &args.packages {
            if !installed.contains_key(name) {
                return Err(XpmError::PackageNotFound { name: name.clone() }.into());
            }
            if !names.contains(name) {
                names.push(name.clone());
            }
        }
        names
    };

    if args.repair {
        let mut failed = false;
        for name in &targets {
            match check::repair_package_manifest(&local_db_dir, name) {
                Ok(report) if report.skipped => {
                    println!("warning: {name}: no stored .MTREE; manifest left untouched");
                }
                Ok(report) if report.repaired => {
                    println!(
                        "repaired {name}: {} -> {} manifest entries",
                        report.old_entries, report.new_entries
                    );
                }
                Ok(_) => {
                    println!("{name}: file manifest already matches .MTREE");
                }
                Err(err) => {
                    eprintln!("error: {name}: {err:#}");
                    failed = true;
                }
            }
        }
        if failed {
            exit_with(1);
        }
        return Ok(());
    }

    let mut failed = false;
    for name in &targets {
        match check::check_package(&config.options.root_dir, &local_db_dir, name, args.full) {
            Ok(report) => {
                for path in &report.missing {
                    println!("warning: {name}: {path} (missing)");
                }
                if args.full {
                    for path in &report.altered {
                        println!("warning: {name}: {path} (altered)");
                    }
                    for path in &report.metadata_issues {
                        println!("warning: {name}: {path} (missing metadata)");
                    }
                    if report.metadata_missing {
                        println!("warning: {name}: no stored mtree; full check unavailable");
                    }
                    println!(
                        "{name}: {} total files, {} altered files",
                        report.total,
                        report.altered.len()
                    );
                } else {
                    println!(
                        "{name}: {} total files, {} missing files",
                        report.total,
                        report.missing.len()
                    );
                }
                if !report.is_ok(args.full) {
                    failed = true;
                }
            }
            Err(err) => {
                eprintln!("error: {name}: {err:#}");
                failed = true;
            }
        }
    }

    if failed {
        exit_with(1);
    }
    Ok(())
}
