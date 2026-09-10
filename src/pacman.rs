//! A rendszer csomagállapotának lekérdezése.
//!
//! Szándékosan SHELL-OUT, nem libalpm-kötés. A paru köti, és amikor a pacman
//! 7.1 a libalpm 16-ra váltott (2025-12), a paru öt hétig egyáltalán nem
//! fordult. Egy konfigurációkezelő toolnál ez különösen rossz: pont az az
//! eszköz esne ki, amivel javítanál. Cserébe a lami csomagjának nincs
//! megosztott könyvtár függősége.

use std::collections::BTreeSet;
use std::process::Command;

use crate::error::{Error, Result};

/// Az ismert AUR helperek, preferencia-sorrendben.
const AUR_HELPERS: &[&str] = &["paru", "yay", "pikaur", "aura"];

fn run(bin: &str, args: &[&str]) -> Result<String> {
    let out = Command::new(bin).args(args).output().map_err(|e| {
        Error::Other(format!(
            "a(z) '{bin}' nem futtatható: {e}\n\
             A lami Arch Linuxra készült, és a pacman jelenlétét feltételezi."
        ))
    })?;
    if !out.status.success() {
        return Err(Error::Other(format!(
            "'{bin} {}' hibával tért vissza:\n{}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Elérhető-e egyáltalán a pacman. Példa-config böngészéséhez nem kell.
pub fn available() -> bool {
    which("pacman").is_some()
}

fn which(bin: &str) -> Option<String> {
    let path = std::env::var("PATH").ok()?;
    for dir in path.split(':') {
        let p = std::path::Path::new(dir).join(bin);
        if p.is_file() {
            return Some(p.display().to_string());
        }
    }
    None
}

/// Minden csomagnév, ami elérhető a beállított sync repókból.
///
/// Ebből derül ki, mi jön a repóból és mi az AUR-ból -- ezért nincs külön
/// `aur` blokk a configban.
pub fn sync_packages() -> Result<BTreeSet<String>> {
    Ok(run("pacman", &["-Slq"])?
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect())
}

/// A gépre EXPLICIT telepített csomagok.
///
/// Szándékosan `-Qqe` és nem `-Qq`: egy csak függőségként fent lévő csomag
/// "hiányzik", mert ha a húzó csomag eltűnik, az orphan-takarítás elviszi.
pub fn explicit_packages() -> Result<BTreeSet<String>> {
    Ok(run("pacman", &["-Qqe"])?
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect())
}

/// A telepített AUR helper neve, ha van.
pub fn aur_helper() -> Option<&'static str> {
    AUR_HELPERS.iter().copied().find(|h| which(h).is_some())
}
