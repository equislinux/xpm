//! Clean-chroot builds.
//!
//! The design separates three things that used to be mixed together:
//!
//! * one immutable `base-devel` template per architecture;
//! * a shared package cache (and the host sync database) bind-mounted into the
//!   disposable root;
//! * one cheap copy per build (btrfs snapshot, reflink, overlayfs or a plain
//!   copy as the last resort).
//!
//! The template is only refreshed under an exclusive lock and never mutated by
//! a build. Build dependencies are installed inside the disposable copy with
//! `xpm`, so `makepkg` can run with `--nodeps` and the host stays clean.

use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::{
    collect_packages, BuildArtifacts, BuildError, BuildOptions, BuildResult, BuiltPackage,
};

const STAMP_SCHEMA: u32 = 1;
const SECONDS_PER_DAY: u64 = 24 * 60 * 60;

mod cleanup;
mod clone;
mod mounts;
mod prepare;
mod run;
mod template;
mod util;

use cleanup::*;
use clone::*;
use mounts::*;
use prepare::*;
use run::*;
use template::*;
use util::*;

/// Shared `base-devel` template and the disposable roots derived from it.
pub(crate) fn build_chroot(
    source_dir: &Path,
    options: &BuildOptions,
) -> BuildResult<BuildArtifacts> {
    if !command_exists("arch-chroot") {
        return Err(BuildError::Command(
            "clean builds require arch-chroot (arch-install-scripts)".into(),
        ));
    }

    let build_uid = build_user_uid();
    let artifact_uid = artifact_user_uid();
    let template_dir = template_dir(options);
    let template_root = template_dir.join("root");
    let stamp_path = template_dir.join("template.json");

    let lock = open_template_lock(&template_dir)?;
    ensure_template(&template_root, &stamp_path, options, &lock)?;
    cleanup_stale_works(&template_dir);

    let work = template_dir.join(format!("work-{}-{}", std::process::id(), now_epoch_nanos()));
    run_root("mkdir", [OsStr::new("-p"), work.as_os_str()])?;
    // Held for the lifetime of the build so stale-work cleanup never removes
    // a root that is still in use.
    let _work_lock = open_file_lock(&work.join(".lock"))?;
    let kind = clone_template(&template_root, &work, &lock)?;
    lock.unlock();

    let guard = WorkGuard::new(work.clone(), kind);
    prepare_work(&work, source_dir, options, build_uid)?;
    run_makepkg(&work, options)?;
    let packages = collect_chroot_artifacts(&work, options, artifact_uid)?;
    drop(guard);

    if packages.is_empty() {
        return Err(BuildError::NoArtifacts(
            options.pkgdest.display().to_string(),
        ));
    }

    Ok(BuildArtifacts {
        source_dir: source_dir.to_path_buf(),
        pkgbase: crate::read_pkgbase(source_dir),
        packages,
    })
}

#[cfg(test)]
mod tests;
