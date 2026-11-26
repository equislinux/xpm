//! `bootstrap` arguments.

#[derive(Debug, clap::Args)]
pub struct BootstrapArgs {
    /// Packages to install after filesystem and bash (default: base-devel).
    #[arg(value_name = "PACKAGE")]
    pub packages: Vec<String>,
}
