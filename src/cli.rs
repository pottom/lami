//! Command line interface.
//!
//! In this milestone every command is READ-ONLY. `apply` is deliberately
//! absent: the `diff` output has to be validated against the existing
//! metapac/decman/chezmoi setup before lami writes anything.

use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(
    name = "lami",
    about = "Layered, declarative system configuration for Arch Linux",
    version
)]
pub struct Cli {
    /// Config directory. Defaults to $XDG_CONFIG_HOME/lami
    #[arg(long, global = true, env = "LAMI_CONFIG_DIR")]
    pub config_dir: Option<PathBuf>,

    /// Resolve as a different host (for testing).
    #[arg(long, global = true, env = "LAMI_HOST")]
    pub host: Option<String>,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Show this host's resolved profile: layers and parameters.
    Show,

    /// Explain where a resource comes from and why this host gets it.
    Why {
        /// A package name or a service name.
        target: String,
    },

    /// List known hosts and layers.
    List,

    /// Check the config against the system: are all packages available, is an
    /// AUR helper needed. Changes nothing.
    Check,
}
