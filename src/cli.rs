//! Parancssori felület.
//!
//! Az első mérföldkőben MINDEN parancs csak olvas. Az `apply` szándékosan
//! nincs itt: előbb a `diff` kimenetét kell hitelesíteni a jelenlegi
//! metapac/decman/chezmoi hármas ellen.

use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(
    name = "lami",
    about = "Rétegzett, deklaratív rendszerkonfiguráció Arch Linuxra",
    version
)]
pub struct Cli {
    /// A config könyvtára. Alapból: $XDG_CONFIG_HOME/lami
    #[arg(long, global = true, env = "LAMI_CONFIG_DIR")]
    pub config_dir: Option<PathBuf>,

    /// Más gép profiljával fusson (teszteléshez).
    #[arg(long, global = true, env = "LAMI_HOST")]
    pub host: Option<String>,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// A gép feloldott profilja: rétegek, paraméterek.
    Show,

    /// Honnan jön egy erőforrás, és miért kapja ez a gép.
    Why {
        /// Csomag neve, szolgáltatás vagy fájl útvonala.
        target: String,
    },

    /// Az ismert gépek és rétegek listája.
    List,

    /// A config ellenőrzése a rendszer ellen: van-e minden csomag, kell-e
    /// AUR helper. Semmit nem módosít.
    Check,
}
