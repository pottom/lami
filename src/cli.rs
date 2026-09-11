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

    /// Git URL of the config repo.
    ///
    /// If the local working copy is missing, it is cloned. `lami clone`
    /// records this permanently; passing it here is a one-off override of
    /// what ~/.config/lami.kdl says.
    #[arg(long, global = true, env = "LAMI_REPO")]
    pub repo: Option<String>,

    /// Resolve as a different host (for testing).
    #[arg(long, global = true, env = "LAMI_HOST")]
    pub host: Option<String>,

    /// Never colour the output.
    ///
    /// Colour is on by default, and already turns itself off when the output
    /// is not a terminal or when NO_COLOR is set.
    #[arg(long, global = true)]
    pub no_color: bool,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Clone your config repo and remember where it went.
    ///
    /// Writes ~/.config/lami.kdl, so every later command finds the repo on
    /// its own. This is the first thing to run on a new machine.
    Clone {
        /// Git URL, as you would give to `git clone`.
        url: String,

        /// Where to put the working copy. Defaults to ~/.config/lami.
        #[arg(long)]
        path: Option<PathBuf>,
    },

    /// Write a host file for a new machine.
    ///
    /// The layers you choose decide what has to be in it: lami asks them what
    /// they need to know, and writes those parameters out with their
    /// descriptions and defaults already in place. Nothing is guessed -- the
    /// required ones are left blank for you to fill in.
    Init {
        /// The machine's hostname. The file is named after it, and that name
        /// is what `lami diff` looks for on the machine itself.
        name: String,

        /// Which layers it gets. Their `needs` are added automatically.
        #[arg(long, value_delimiter = ',', required = true)]
        layers: Vec<String>,

        /// Start from another host's answers, for a machine much like one you
        /// already have.
        #[arg(long)]
        like: Option<String>,
    },

    /// Fast-forward the config repo to its remote.
    ///
    /// Uncommitted local changes are left alone; a diverged history is
    /// reported rather than merged.
    Pull,

    /// Commit everything in the config repo and push it.
    ///
    /// The counterpart of `capture`: capture writes into the repo, push
    /// sends it to the other machines.
    Push {
        /// Commit message. Defaults to "<hostname>: config update".
        #[arg(long, short)]
        message: Option<String>,
    },

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

    /// Show what differs between the config and this machine.
    ///
    /// Nothing is changed. This is the gate before `apply`: its output has to
    /// line up with what your existing tooling reports.
    Diff {
        /// Also list what is on the machine that no layer declares: packages,
        /// units, group memberships, and files sitting in a managed directory.
        #[arg(long)]
        undeclared: bool,

        /// Show what actually differs inside each changed file, as a diff.
        #[arg(long, short)]
        patch: bool,
    },

    /// Bring the machine in line with the config.
    ///
    /// Installs missing packages, writes managed files, enables declared
    /// services and runs any hook whose watched file changed -- in that order.
    ///
    /// Never removes anything. That is what `prune` is for.
    Apply {
        /// Show what would be done and stop.
        #[arg(long)]
        dry_run: bool,
    },

    /// Pull a change made on this machine back into the repo.
    ///
    /// With no arguments it lists what could be captured and how. Naming a
    /// package files it into a layer.
    Capture {
        /// The package to file into a layer. Repeat it, or separate names
        /// with commas, to file several at once.
        #[arg(long, value_delimiter = ',')]
        package: Vec<String>,

        /// The unit to file into a layer. Repeatable, like --package.
        #[arg(long, value_delimiter = ',')]
        service: Vec<String>,

        /// File every package this machine has that no layer declares.
        #[arg(long)]
        all: bool,

        /// A managed file whose live content should be copied back.
        #[arg(long)]
        file: Option<String>,

        /// Which layer it belongs in.
        #[arg(long)]
        layer: Option<String>,

        /// Show the edit without writing it.
        #[arg(long)]
        dry_run: bool,
    },

    /// Remove what lami used to manage but no longer declares.
    ///
    /// Only ever touches things a previous `apply` recorded as managed. A
    /// package you installed by hand is never a candidate, because lami never
    /// claimed it.
    Prune {
        /// Show what would be removed and stop. This is the default.
        #[arg(long)]
        dry_run: bool,

        /// Actually remove. Asks for confirmation unless --yes is given too.
        #[arg(long)]
        force: bool,

        /// Skip the confirmation prompt.
        #[arg(long)]
        yes: bool,
    },

    /// Render managed files without installing them.
    ///
    /// With no arguments, every file this host would get is printed with a
    /// header. Name a path to print just that one, raw and pipeable. With
    /// `--out` the files are written into a directory tree instead, so you can
    /// diff them against the live system with your own tools.
    Render {
        /// Render only this target path.
        target: Option<String>,

        /// Render only what this layer declares.
        ///
        /// Answers "what does the rice layer actually write?" without reading
        /// the layer file and following every `from=` by hand.
        #[arg(long)]
        layer: Option<String>,

        /// Write into this directory instead of printing.
        #[arg(long, short)]
        out: Option<PathBuf>,

        /// Only list the paths that would be rendered.
        #[arg(long)]
        list: bool,
    },
}
