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

/// Claim packages that are already here as somebody else's dependency.
///
/// `pacman -S --needed` will not do this: it sees the package installed, says
/// "up to date -- skipping", and leaves the install reason alone. The package
/// would then still be missing from `-Qqe`, so the next `diff` would report
/// the very same change -- apply would not be idempotent.
fn adopt_packages(names: &[String]) -> Result<()> {
    if names.is_empty() {
        return Ok(());
    }
    println!(
        "  pacman -D --asexplicit  ({} package(s) already present as dependencies)",
        names.len()
    );
    run(Command::new("pacman")
        .args(["-D", "--asexplicit"])
        .args(names.iter().map(|s| s.as_str())))
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
    let (repo, aur): (Vec<&String>, Vec<&String>) = names.iter().partition(|n| sync.contains(*n));

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
                aur.iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
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

pub fn run_apply(
    r: &Resolved<'_>,
    actor: &Actor,
    dry: bool,
    settings: &crate::config::Settings,
) -> Result<usize> {
    let report = diff::compute(r, &actor.home, &actor.name, settings)?;

    if report.changes.is_empty() {
        println!("Nothing to do -- the machine already matches the config.");
        // Still record what is managed. The state file describes what IS
        // declared, not what happened to change on this run -- otherwise a
        // no-op apply would leave prune with a stale picture.
        if !dry {
            record_state(r, actor)?;
        }
        return Ok(0);
    }

    let n = report.changes.len();
    println!(
        "{}\n",
        crate::color::dim(&format!("{n} change{}:", if n == 1 { "" } else { "s" }))
    );
    crate::diff::print(&report.changes);
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
    let adopt: Vec<String> = report
        .changes
        .iter()
        .filter_map(|c| match c {
            Change::AdoptPackage { name, .. } => Some(name.clone()),
            _ => None,
        })
        .collect();
    let package_changes = missing.len() + adopt.len();
    if package_changes > 0 {
        println!("{}", crate::color::bold("packages:"));
        install(&missing, actor)?;
        adopt_packages(&adopt)?;
    }

    // Everything from here on is measured against the machine as it is NOW.
    //
    // The first report was taken before those packages existed, and a unit
    // that is not installed cannot be reported as disabled -- so on a fresh
    // machine every service whose package this run just installed would have
    // been left alone, and `apply` would have needed a second run to
    // converge. The same goes for a file the new package brought with it.
    //
    // Found by installing this config into an empty VM: four services stayed
    // disabled and the next `diff` asked for them again.
    let report = if package_changes > 0 {
        diff::compute(r, &actor.home, &actor.name, settings)?
    } else {
        report
    };

    // --- files ------------------------------------------------------------
    let files: Vec<&Change> = report
        .changes
        .iter()
        .filter(|c| {
            matches!(
                c,
                Change::CreateFile { .. }
                    | Change::UpdateFile { .. }
                    | Change::FixPermissions { .. }
            )
        })
        .collect();
    if !files.is_empty() {
        println!("{}", crate::color::bold("files:"));
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
            let content = render::file(r, layer, f, settings, &actor.home, &actor.name)?;
            let pm = perms::with_overrides(
                perms::secret_aware(f, &actor.name),
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
    //
    // Order within this block matters: reload before anything else, so systemd
    // knows about a unit file that was just written; then set states; then
    // restart. Enabling a unit systemd has not re-read yet simply fails.
    let svc: Vec<&Change> = report
        .changes
        .iter()
        .filter(|c| {
            matches!(
                c,
                Change::SetUnitState { .. }
                    | Change::DaemonReload { .. }
                    | Change::RestartUnit { .. }
            )
        })
        .collect();

    if !svc.is_empty() {
        println!("{}", crate::color::bold("services:"));

        for c in &svc {
            if let Change::DaemonReload { scope } = c {
                run(systemd::cmd(*scope, &actor.name).arg("daemon-reload"))?;
                println!("  systemd reloaded");
            }
        }

        for c in &svc {
            if let Change::SetUnitState {
                unit, scope, want, ..
            } = c
            {
                use crate::config::UnitState;
                let verb = match want {
                    UnitState::Enabled => "enable",
                    UnitState::Disabled => "disable",
                    UnitState::Masked => "mask",
                };
                run(systemd::cmd(*scope, &actor.name).args([verb, unit]))?;
                println!("  {unit} {want}");
            }
        }

        // Last, and only for units that asked. lami still never STARTS a
        // service: a restart here is finishing a change to that unit's own
        // file, not deciding that it ought to be running.
        for c in &svc {
            if let Change::RestartUnit { unit, scope, .. } = c {
                run(systemd::cmd(*scope, &actor.name).args(["restart", unit]))?;
                println!("  {unit} restarted");
            }
        }
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
        println!("{}", crate::color::bold("hooks:"));
        for (cmd, because) in hooks {
            println!("  {cmd}   (because {because} changed)");
            let mut parts = cmd.split_whitespace();
            let Some(bin) = parts.next() else { continue };
            run(Command::new(bin).args(parts))?;
        }
    }

    record_state(r, actor)?;
    Ok(package_changes + report.changes.len())
}

/// Remember what is managed now, so that a later `prune` can tell "no longer
/// declared" apart from "never declared".
fn record_state(r: &Resolved<'_>, actor: &Actor) -> Result<()> {
    let mut st = crate::state::State {
        host: r.host.name.clone(),
        ..Default::default()
    };
    for (_, d) in r.packages() {
        st.packages.insert(d.name.clone());
    }
    for (_, d) in r.services() {
        let unit = systemd::qualify(&d.name);
        match d.scope {
            crate::config::Scope::User => st.user_services.insert(unit),
            crate::config::Scope::System => st.services.insert(unit),
        };
    }
    for (_, f) in r.files() {
        st.files
            .insert(render::target_path(f, &actor.home).display().to_string());
    }
    crate::state::save(&st)
}

/// Unused import guard: keeps systemd in scope for future service work.
#[allow(dead_code)]
fn _unused(_: &Path) {
    let _ = systemd::available();
}

/// What is no longer declared, but a previous apply recorded as managed.
pub struct Stale {
    pub packages: Vec<String>,
    pub services: Vec<String>,
    pub user_services: Vec<String>,
    pub files: Vec<String>,
}

impl Stale {
    pub fn is_empty(&self) -> bool {
        self.packages.is_empty()
            && self.services.is_empty()
            && self.user_services.is_empty()
            && self.files.is_empty()
    }
    pub fn len(&self) -> usize {
        self.packages.len() + self.services.len() + self.user_services.len() + self.files.len()
    }
}

/// Work out what a `prune` would remove.
///
/// The state file is the whole point: it is what distinguishes "lami put this
/// here and no longer wants it" from "somebody installed this by hand". Only
/// the first is ever a candidate.
pub fn stale(r: &Resolved<'_>, actor: &Actor) -> Stale {
    let prev = crate::state::state_load();

    let now_pkgs: std::collections::BTreeSet<String> =
        r.packages().iter().map(|(_, d)| d.name.clone()).collect();
    let mut now_svcs = std::collections::BTreeSet::new();
    let mut now_user_svcs = std::collections::BTreeSet::new();
    for (_, d) in r.services() {
        let unit = systemd::qualify(&d.name);
        match d.scope {
            crate::config::Scope::User => now_user_svcs.insert(unit),
            crate::config::Scope::System => now_svcs.insert(unit),
        };
    }
    let now_files: std::collections::BTreeSet<String> = r
        .files()
        .iter()
        .map(|(_, f)| render::target_path(f, &actor.home).display().to_string())
        .collect();

    Stale {
        packages: prev.packages.difference(&now_pkgs).cloned().collect(),
        services: prev.services.difference(&now_svcs).cloned().collect(),
        user_services: prev
            .user_services
            .difference(&now_user_svcs)
            .cloned()
            .collect(),
        files: prev.files.difference(&now_files).cloned().collect(),
    }
}

pub fn run_prune(s: &Stale, actor: &Actor) -> Result<()> {
    if !s.services.is_empty() || !s.user_services.is_empty() {
        println!("{}", crate::color::bold("services:"));
    }
    // Disabled, not stopped. Stopping a display manager out from under a
    // running session because a layer was edited would be indefensible.
    //
    // Scope is not cosmetic: `systemctl disable` in system scope for a unit
    // that was enabled in user scope succeeds and does nothing at all.
    for (scope, units) in [
        (crate::config::Scope::System, &s.services),
        (crate::config::Scope::User, &s.user_services),
    ] {
        if units.is_empty() {
            continue;
        }
        run(systemd::cmd(scope, &actor.name)
            .arg("disable")
            .args(units.iter().map(|x| x.as_str())))?;
        for u in units {
            match scope {
                crate::config::Scope::User => println!("  {u} disabled (user)"),
                _ => println!("  {u} disabled"),
            }
        }
    }
    if !s.files.is_empty() {
        println!("{}", crate::color::bold("files:"));
        for f in &s.files {
            match std::fs::remove_file(f) {
                Ok(()) => println!("  {f} removed"),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    println!("  {f} (already gone)")
                }
                Err(e) => println!("  {f} FAILED: {e}"),
            }
        }
    }
    if !s.packages.is_empty() {
        println!("{}", crate::color::bold("packages:"));
        // -Rs removes dependencies that nothing else needs; -n drops config
        // files pacman saved. Deliberately NOT --nosave on the pacman side of
        // /etc: pacsave files are the last line of defence against a mistake.
        run(Command::new("pacman")
            .args(["-Rs", "--noconfirm"])
            .args(s.packages.iter().map(|x| x.as_str())))?;
    }
    Ok(())
}
