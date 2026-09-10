//! Querying the system's package state.
//!
//! Deliberately shells out instead of linking libalpm. paru links it, and when
//! pacman 7.1 moved to libalpm 16 (December 2025) paru would not even compile
//! for five weeks. For a configuration management tool that is uniquely bad:
//! the casualty would be the very tool you reach for to repair the system.
//! In exchange, lami's package has no shared library dependency at all.

use std::collections::BTreeSet;
use std::process::Command;

use crate::error::{Error, Result};

/// Known AUR helpers, in order of preference.
const AUR_HELPERS: &[&str] = &["paru", "yay", "pikaur", "aura"];

fn run(bin: &str, args: &[&str]) -> Result<String> {
    let out = Command::new(bin).args(args).output().map_err(|e| {
        Error::Other(format!(
            "cannot run '{bin}': {e}\n\
             lami targets Arch Linux and expects pacman to be present."
        ))
    })?;
    if !out.status.success() {
        return Err(Error::Other(format!(
            "'{bin} {}' failed:\n{}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Whether pacman is available at all. Browsing an example config does not
/// require it.
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

/// Every package name available from the configured sync repositories.
///
/// This is how lami tells repo packages from AUR packages, which is why the
/// config has no separate `aur` block.
pub fn sync_packages() -> Result<BTreeSet<String>> {
    Ok(run("pacman", &["-Slq"])?
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect())
}

/// Packages installed EXPLICITLY on this machine.
///
/// `-Qqe` rather than `-Qq` on purpose: a package present only as a dependency
/// counts as missing, because an orphan sweep will take it away as soon as
/// whatever pulled it in disappears.
pub fn explicit_packages() -> Result<BTreeSet<String>> {
    Ok(run("pacman", &["-Qqe"])?
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect())
}

/// The installed AUR helper, if any.
pub fn aur_helper() -> Option<&'static str> {
    AUR_HELPERS.iter().copied().find(|h| which(h).is_some())
}
