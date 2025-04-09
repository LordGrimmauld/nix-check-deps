// SPDX-License-Identifier: GPL-3.0-only

use clap::{ArgAction, Parser};
use clap_stdin::MaybeStdin;

#[derive(Debug, Parser)]
#[clap(name="nix-check-deps", version=env!("CARGO_PKG_VERSION"),about=env!("CARGO_PKG_DESCRIPTION"), author=env!("CARGO_PKG_AUTHORS"))]
pub struct Cli {
    /// package to evaluate
    pub attr: MaybeStdin<String>,

    /// Don't scan for c header files in use
    #[clap(long = "no-check-headers", action = ArgAction::SetFalse)]
    pub check_headers: bool,

    /// Don't scan for deps listed in pyproject
    #[clap(long = "no-check-pyproject", action = ArgAction::SetFalse)]
    pub check_pyproject: bool,

    /// Don't scan for programs used in shebang lines
    #[clap(long = "no-check-shebangs", action = ArgAction::SetFalse)]
    pub check_shebangs: bool,

    /// Don't check binaries and shared objects for their library paths
    #[clap(long = "no-check-shared_objects", action = ArgAction::SetFalse)]
    pub check_shared_objects: bool,

    /// output used C/C++ headers
    #[arg(long, default_value_t = false)]
    pub list_used_headers: bool,

    /// skips check of dependencies in use
    #[arg(long, default_value_t = false)]
    pub skip_dep_usage_check: bool,

    /// output json
    #[arg(long, default_value_t = false)]
    pub json: bool,

    /// scan all transient dependents of a derivation
    #[arg(long, default_value_t = false)]
    pub tree: bool,

    /// drv names to skip
    #[arg(long, default_value_t = String::from(""))]
    pub skip: String,

    /// number of packages to check at once [broken]
    #[arg(long, short, default_value_t = 1)]
    pub jobs: usize,
}
