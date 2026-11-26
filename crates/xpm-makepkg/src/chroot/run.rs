//! Running makepkg inside the disposable root and collecting artifacts.

use super::*;

pub(super) fn run_makepkg(work: &Path, options: &BuildOptions) -> BuildResult<()> {
    let root = work.join("root");
    let makepkg = makepkg_line(options);
    let environment = makepkg_environment(options);
    let command = format!("cd /build && {environment} {makepkg}");
    tracing::info!(chroot = %root.display(), "building in clean chroot");
    let status = root_command("arch-chroot")
        .arg(&root)
        .args(["runuser", "-u", "build", "--", "bash", "-lc", &command])
        .status()
        .map_err(|err| BuildError::Command(format!("arch-chroot: {err}")))?;
    if !status.success() {
        return Err(BuildError::Command(format!(
            "clean chroot build exited with status {:?}",
            status.code()
        )));
    }
    Ok(())
}

pub(super) fn makepkg_line(options: &BuildOptions) -> String {
    let mut makepkg = String::from("makepkg --noconfirm --cleanbuild --force --noprogressbar");
    if options.chroot_nodeps {
        makepkg.push_str(" --nodeps");
    }
    if !options.run_checks {
        makepkg.push_str(" --nocheck");
    }
    for arg in &options.extra_args {
        makepkg.push(' ');
        makepkg.push_str(&shell_quote(arg));
    }
    makepkg
}

pub(super) fn makepkg_environment(options: &BuildOptions) -> String {
    let mut environment = String::from(
        "env -u LDFLAGS -u CFLAGS -u CXXFLAGS HOME=/home/build \
         PACMAN=/usr/local/bin/pacman PKGDEST=/pkgdest PKGEXT=.pkg.tar.zst",
    );
    if options.srcdest.is_some() {
        environment.push_str(" SRCDEST=/srcdest SRCPKGDEST=/srcdest");
    }
    if options.builddir.is_some() {
        environment.push_str(" BUILDDIR=/builddir");
    }
    environment
}

pub(super) fn collect_chroot_artifacts(
    work: &Path,
    options: &BuildOptions,
    artifact_uid: u32,
) -> BuildResult<Vec<BuiltPackage>> {
    let pkgdest = work.join("root/pkgdest");
    fs::create_dir_all(&options.pkgdest).map_err(|source| BuildError::Io {
        path: options.pkgdest.display().to_string(),
        source,
    })?;
    let entries = match fs::read_dir(&pkgdest) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => {
            return Err(BuildError::Io {
                path: pkgdest.display().to_string(),
                source,
            });
        }
    };
    for entry in entries.flatten() {
        let source = entry.path();
        let Some(name) = source.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !name.ends_with(".pkg.tar.zst") {
            continue;
        }
        let dest = options.pkgdest.join(name);
        fs::copy(&source, &dest).map_err(|source_error| BuildError::Io {
            path: dest.display().to_string(),
            source: source_error,
        })?;
        let _ = root_command("chown")
            .arg(format!("{artifact_uid}:{artifact_uid}"))
            .arg(&dest)
            .status();
    }
    let packages = collect_packages(&options.pkgdest, None)?;
    Ok(packages)
}
