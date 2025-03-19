// SPDX-License-Identifier: GPL-3.0-only

use clap::Parser;
use clap_stdin::MaybeStdin;

#[derive(Debug, Parser)]
#[clap(name="nix-check-deps", version=env!("CARGO_PKG_VERSION"),about=env!("CARGO_PKG_DESCRIPTION"), author=env!("CARGO_PKG_AUTHORS"))]
pub struct Cli {
    /// package to evaluate
    pub attr: MaybeStdin<String>,
}
