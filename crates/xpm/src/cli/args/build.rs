//! `build` arguments.

use std::path::PathBuf;

#[derive(Debug, clap::Args)]
pub struct BuildArgs {
    /// Directory containing PKGBUILD (or the PKGBUILD file itself).
    #[arg(value_name = "DIR", default_value = ".")]
    pub path: PathBuf,

    /// Directory for built packages (default: <DIR>/.xpm-packages).
    #[arg(long, value_name = "PATH")]
    pub pkgdest: Option<PathBuf>,

    /// makepkg source cache directory.
    #[arg(long, value_name = "PATH")]
    pub srcdest: Option<PathBuf>,

    /// makepkg build directory.
    #[arg(long, value_name = "PATH")]
    pub builddir: Option<PathBuf>,

    /// Skip the check() function.
    #[arg(long)]
    pub nocheck: bool,

    /// Remove existing package files from --pkgdest before building.
    #[arg(long)]
    pub clean: bool,

    /// Build inside a clean chroot (shared base-devel template + arch-chroot).
    #[arg(long)]
    pub chroot: bool,

    /// Base directory for the shared template and disposable build copies
    /// (default: /var/lib/archbuild/xpm-aur).
    #[arg(long, value_name = "PATH")]
    pub chroot_dir: Option<PathBuf>,

    /// Install an official package into the clean chroot before building.
    /// Can be repeated.
    #[arg(long = "chroot-install", value_name = "PKG")]
    pub chroot_install: Vec<String>,

    /// Pass --nodeps to makepkg in the chroot; install dependencies with
    /// --chroot-install / --preinstall first.
    #[arg(long)]
    pub nodeps: bool,

    /// Force a shared base-devel template refresh before building.
    #[arg(long)]
    pub chroot_refresh: bool,

    /// Maximum age of the shared template in days (0 refreshes every build).
    #[arg(long, value_name = "DAYS", default_value_t = 7)]
    pub chroot_ttl: u64,

    /// Install the built packages with xpm after a successful build.
    #[arg(long)]
    pub install: bool,

    /// Mark installed packages as dependencies.
    #[arg(long)]
    pub as_deps: bool,

    /// Mark installed packages as explicit.
    #[arg(long)]
    pub as_explicit: bool,

    /// Install a local package into the clean chroot before building.
    #[arg(long, value_name = "PKG")]
    pub preinstall: Vec<PathBuf>,

    /// Write the build result as JSON to this path.
    #[arg(long, value_name = "PATH")]
    pub json: Option<PathBuf>,

    /// Extra arguments passed through to makepkg after `--`.
    #[arg(last = true, allow_hyphen_values = true)]
    pub makepkg_args: Vec<String>,
}
