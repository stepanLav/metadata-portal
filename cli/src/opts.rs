use clap::{Parser, Subcommand};
use std::path::PathBuf;

use crate::updater::source::UpdateSource;

#[derive(Parser)]
pub struct Opts {
    /// Path to config file
    #[clap(short, long, default_value = "config.toml")]
    pub config: PathBuf,

    #[clap(subcommand)]
    pub subcmd: SubCommand,
}

/// You can find all available commands below.
#[derive(Subcommand)]
pub enum SubCommand {
    /// Remove unused QR codes
    Clean,

    /// Generate json data file for frontend
    Collect,

    /// Sign unsigned QR codes.
    Sign,

    /// Check updates
    Update(UpdateOpts),

    /// Verify signed QR codes
    Verify,
}

#[derive(Parser)]
pub struct UpdateOpts {
    #[clap(short, long, default_value = "node")]
    pub source: UpdateSource,
}
