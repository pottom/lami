//! Applying the config to the machine.
//!
//! Order matters: packages, then files, then services, then hooks. A service
//! cannot be enabled before its package exists, and a hook exists to react to
//! a file that has just changed.
//!
//! `apply` NEVER removes anything. Removal is `prune`, with its own
//! confirmation. A typo'd layer name or a half-written config should not be
//! able to uninstall your desktop.

use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::Command;

use crate::config::Resolved;
use crate::diff::{self, Change};
use crate::error::{Error, Result};
use crate::{pacman, perms, render, systemd, write};

/// Who we act as when dropping privileges.
pub struct Actor {
    pub name: String,
    pub uid: u32,
    pub gid: u32,
    pub home: std::path::PathBuf,
}

pub fn require_root() -> Result<()> {
    if unsafe { libc::geteuid() } != 0 {
        return Err(Error::Other(
            "apply needs root, because it writes to /etc and enables units.\n\
             \n  sudo lami apply\n\
             \nRead-only commands (status, diff, why, render) deliberately do not."
                .into(),
        ));
    }
    Ok(())
}

/// Install missing packages.
///
/// If everything is available from a repository, plain pacman does the job as
/// root. AUR packages need a helper, and every AUR helper refuses to run as
/// root -- rightly, since it builds untrusted PKGBUILDs. So for those we drop
/// to the invoking user.
fn install(names: &[String], actor: &Actor) -> Result<()> {
    if names.is_empty() {
        return Ok(());
    }
    let sync = pacman::sync_packages()?;
    let (repo, aur): (Vec<&String>, Vec<&String>) =
        names.iter().partition(|n| sync.contains(*n));

    if !repo.is_empty() {
        println!("  pacman -S --needed  ({} package(s))", repo.len());
        run(Command::new("pacman")
            .args(["-S", "--needed", "--noconfirm"])
            .args(repo.iter().map(|s| s.as_str())))?;
    }

    if !aur.is_empty() {
        let helper = pacman::aur_helper().ok_or_else(|| {
            Error::Other(format!(
                "{} package(s) need an AUR helper, and none is installed:\n  {}",
                aur.len(),
                aur.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")
            ))
        })?;
        println!(
            "  {helper} -S --needed  ({} package(s), as {})",
            aur.len(),
            actor.name
        );

        // Dropped to the invoking user. Rust's uid()/gid() set the group
        // before the user, which is the only order that works: setgid after
        // setuid would already be forbidden.
        //
        // HOME comes from passwd, not from the environment: under sudo $HOME
        // may still be the caller's, and the helper's build cache would end up
        // in the wrong place -- owned by root, in root's home.
        run(Command::new(helper)
            .args(["-S", "--needed", "--noconfirm"])
            .args(aur.iter().map(|s| s.as_str()))
            .uid(actor.uid)
            .gid(actor.gid)
            .env("HOME", &actor.home)
            .env("USER", &actor.name)
            .env("LOGNAME", &actor.name))?;
    }
    Ok(())
}

/// Run a command with our stdio, so its progress and prompts reach the user.
fn run(cmd: &mut Command) -> Result<()> {
    let status = cmd
        .status()
        .map_err(|e| Error::Other(format!("cannot run {:?}: {e}", cmd.get_program())))?;
    if !status.success() {
        return Err(Error::Other(format!(
            "{:?} failed with {status}",
            cmd.get_program()
        )));
    }
    Ok(())
}

pub fn run_apply(r: &Resolved<'_>, actor: &Actor, dry: bool) -> Result<usize> {
    let report = diff::compute(r, &actor.home, &actor.name)?;

    if report.changes.is_empty() {
        println!("Nothing to do -- the machine already matches the config.");
        return Ok(0);
    }

    println!("{} change(s):\n", report.changes.len());
    for c in &report.changes {
        println!("  {}", c.line());
    }
    if dry {
        println!("\n--dry-run: nothing was applied.");
        return Ok(0);
    }
    println!();

    // --- packages ---------------------------------------------------------
    let missing: Vec<String> = report
        .changes
        .iter()
        .filter_map(|c| match c {
            Change::InstallPackage { name, .. } => Some(name.clone()),
            _ => None,
        })
        .collect();
    if !missing.is_empty() {
        println!("packages:");
        install(&missing, actor)?;
    }

    // --- files ------------------------------------------------------------
    let files: Vec<&Change> = report
        .changes
        .iter()
        .filter(|c| {
            matches!(
                c,
                Change::CreateFile { .. } | Change::UpdateFile { .. } | Change::FixPermissions { .. }
            )
        })
        .collect();
    if !files.is_empty() {
        println!("files:");
        for (layer, f) in r.files() {
            let touched = files.iter().any(|c| match c {
                Change::CreateFile { path, .. }
                | Change::UpdateFile { path, .. }
                | Change::FixPermissions { path, .. } => *path == f.path,
                _ => false,
            });
            if !touched {
                continue;
            }
            let content = render::file(r, layer, f)?;
            let pm = perms::with_overrides(
                perms::infer(&f.path, &actor.name),
                f.owner.as_deref(),
                f.group.as_deref(),
                f.mode,
            );
            let target = render::target_path(f, &actor.home);
            write::write(&target, &content, &pm.owner, &pm.group, pm.mode)?;
            println!("  {}  ({:o} {}:{})", f.path, pm.mode, pm.owner, pm.group);
        }
    }

    // --- services ---------------------------------------------------------
    let units: Vec<String> = report
        .changes
        .iter()
        .filter_map(|c| match c {
            Change::EnableService { unit, .. } => Some(unit.clone()),
            _ => None,
        })
        .collect();
    if !units.is_empty() {
        println!("services:");
        // Enable only; never start. A unit that needs to be running now is a
        // decision for the operator, not a side effect of writing config.
        run(Command::new("systemctl")
            .arg("enable")
            .args(units.iter().map(|s| s.as_str())))?;
        for u in &units {
            println!("  {u} enabled");
        }
        let _ = Command::new("systemctl").arg("daemon-reload").status();
    }

    // --- hooks ------------------------------------------------------------
    let hooks: Vec<(&String, &String)> = report
        .changes
        .iter()
        .filter_map(|c| match c {
            Change::RunHook { run, because, .. } => Some((run, because)),
            _ => None,
        })
        .collect();
    if !hooks.is_empty() {
        println!("hooks:");
        for (cmd, because) in hooks {
            println!("  {cmd}   (because {because} changed)");
            let mut parts = cmd.split_whitespace();
            let Some(bin) = parts.next() else { continue };
            run(Command::new(bin).args(parts))?;
        }
    }

    Ok(report.changes.len())
}

/// Unused import guard: keeps systemd in scope for future service work.
#[allow(dead_code)]
fn _unused(_: &Path) {
    let _ = systemd::available();
}
