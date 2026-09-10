//! Comparing the declared state against the machine.
//!
//! This is the gate before anything is ever written: the output has to line up
//! with what the existing tooling reports before `apply` is trusted at all.

use crate::config::Resolved;
use crate::error::Result;
use crate::systemd::{self, State};
use crate::{pacman, perms, render};

/// One difference between what is declared and what the machine has.
#[derive(Debug)]
pub enum Change {
    /// Declared, but not installed as an explicit package.
    InstallPackage { name: String, layer: String },
    /// A unit is not in the state the config asks for.
    SetUnitState {
        unit: String,
        layer: String,
        scope: crate::config::Scope,
        want: crate::config::UnitState,
        current: State,
    },
    /// systemd has to be told to re-read its unit files.
    DaemonReload { scope: crate::config::Scope },
    /// A unit whose own file changed, and which asked to be restarted for it.
    RestartUnit {
        unit: String,
        scope: crate::config::Scope,
        because: String,
    },
    /// Declared, but the file is missing.
    CreateFile { path: String, layer: String },
    /// Declared, and the file on disk differs in content.
    UpdateFile { path: String, layer: String },
    /// Content matches, but ownership or mode does not.
    FixPermissions {
        path: String,
        layer: String,
        want: String,
        have: String,
    },
    /// A watched file will change, so this command has to run afterwards.
    RunHook { run: String, layer: String, because: String },
}

impl Change {
    /// Which section this belongs under.
    pub fn kind(&self) -> &'static str {
        match self {
            Change::InstallPackage { .. } => "packages",
            Change::SetUnitState { .. }
            | Change::DaemonReload { .. }
            | Change::RestartUnit { .. } => "services",
            Change::CreateFile { .. }
            | Change::UpdateFile { .. }
            | Change::FixPermissions { .. } => "files",
            Change::RunHook { .. } => "hooks",
        }
    }

    /// What would happen, as a verb. The verb carries the meaning, so the
    /// output reads the same without colour -- which matters more than usual
    /// here, because a themed terminal may render "green" as anything at all.
    pub fn verb(&self) -> &'static str {
        match self {
            Change::InstallPackage { .. } => "install",
            Change::SetUnitState { want, .. } => match want {
                crate::config::UnitState::Enabled => "enable",
                crate::config::UnitState::Disabled => "disable",
                crate::config::UnitState::Masked => "mask",
            },
            Change::DaemonReload { .. } => "reload",
            Change::RestartUnit { .. } => "restart",
            Change::CreateFile { .. } => "create",
            Change::UpdateFile { .. } => "write",
            Change::FixPermissions { .. } => "chmod",
            Change::RunHook { .. } => "run",
        }
    }

    /// What the change acts on.
    pub fn subject(&self) -> String {
        match self {
            Change::InstallPackage { name, .. } => name.clone(),
            Change::SetUnitState { unit, .. } | Change::RestartUnit { unit, .. } => unit.clone(),
            Change::DaemonReload { scope } => match scope {
                crate::config::Scope::System => "systemd".into(),
                crate::config::Scope::User => "systemd --user".into(),
            },
            Change::CreateFile { path, .. }
            | Change::UpdateFile { path, .. }
            | Change::FixPermissions { path, .. } => path.clone(),
            Change::RunHook { run, .. } => run.clone(),
        }
    }

    /// Why this counts as a change: the machine's current state, in words.
    pub fn reason(&self) -> String {
        match self {
            Change::InstallPackage { layer, .. } => {
                format!("declared in {layer}, not installed")
            }
            Change::SetUnitState {
                layer,
                current,
                scope,
                ..
            } => {
                let s = match scope {
                    crate::config::Scope::System => "",
                    crate::config::Scope::User => "user unit, ",
                };
                format!("{s}declared in {layer}, currently {current}")
            }
            Change::DaemonReload { .. } => "a unit file changed".into(),
            Change::RestartUnit { because, .. } => format!("because {because} changed"),
            Change::CreateFile { layer, .. } => format!("from {layer}, does not exist yet"),
            Change::UpdateFile { layer, .. } => format!("from {layer}, content differs"),
            Change::FixPermissions {
                layer, want, have, ..
            } => format!("from {layer}, mode is {have}, should be {want}"),
            Change::RunHook { because, .. } => format!("because {because} changes"),
        }
    }

    fn paint(&self, s: &str) -> String {
        use crate::color::{action, added, changed};
        match self {
            Change::InstallPackage { .. } | Change::CreateFile { .. } => added(s),
            Change::SetUnitState { want, .. } => match want {
                crate::config::UnitState::Enabled => added(s),
                _ => changed(s),
            },
            Change::UpdateFile { .. } | Change::FixPermissions { .. } => changed(s),
            Change::RunHook { .. }
            | Change::DaemonReload { .. }
            | Change::RestartUnit { .. } => action(s),
        }
    }

    /// One aligned line: verb, subject, reason.
    pub fn line(&self, width: usize) -> String {
        let subject = self.subject();
        format!(
            "  {:<8} {:<width$}  {}",
            self.paint(self.verb()),
            subject,
            crate::color::dim(&self.reason()),
            width = width
        )
    }
}

/// Print a report grouped by kind, so like things line up together.
pub fn print(changes: &[Change]) {
    // A minimum width so the reason column stays put even with a single short
    // entry -- otherwise the subject and its explanation run together and
    // there is nothing for the eye to follow down.
    let width = changes
        .iter()
        .map(|c| c.subject().chars().count())
        .max()
        .unwrap_or(0)
        .clamp(24, 48);

    let mut last = "";
    for c in changes {
        if c.kind() != last {
            if !last.is_empty() {
                println!();
            }
            println!("{}", crate::color::bold(c.kind()));
            last = c.kind();
        }
        println!("{}", c.line(width));
    }
}

pub struct Report {
    pub changes: Vec<Change>,
    /// Explicitly installed packages that no layer declares. Never removed by
    /// `apply`; this is what `capture` offers to file into a layer.
    pub undeclared_packages: Vec<String>,
    pub skipped: Vec<String>,
}

/// Compare a file's actual ownership and mode against what is wanted.
fn permission_change(
    f: &crate::config::FileDecl,
    target: &std::path::Path,
    user: &str,
    layer: &str,
) -> Option<Change> {
    use std::os::unix::fs::MetadataExt;

    let want = perms::with_overrides(
        perms::secret_aware(f, user),
        f.owner.as_deref(),
        f.group.as_deref(),
        f.mode,
    );
    let md = std::fs::metadata(target).ok()?;
    let have_mode = md.mode() & 0o7777;

    // Only the mode is compared for now: resolving uid/gid to names needs
    // passwd lookups that are not worth it before `apply` exists.
    if have_mode == want.mode {
        return None;
    }
    Some(Change::FixPermissions {
        path: f.path.clone(),
        layer: layer.to_string(),
        want: format!("{:o}", want.mode),
        have: format!("{have_mode:o}"),
    })
}

pub fn compute(
    r: &Resolved<'_>,
    home: &std::path::Path,
    user: &str,
    settings: &crate::config::Settings,
) -> Result<Report> {
    let mut changes: Vec<Change> = Vec::new();
    let mut undeclared_packages = Vec::new();
    let mut skipped = Vec::new();

    // --- packages ---------------------------------------------------------
    if pacman::available() {
        let explicit = pacman::explicit_packages()?;
        let mut declared = std::collections::BTreeSet::new();

        for (layer, d) in r.packages() {
            declared.insert(d.name.clone());
            if !explicit.contains(&d.name) {
                changes.push(Change::InstallPackage {
                    name: d.name.clone(),
                    layer: layer.name.clone(),
                });
            }
        }
        undeclared_packages = explicit.difference(&declared).cloned().collect();
    } else {
        skipped.push("packages (pacman not available)".into());
    }

    // --- files ------------------------------------------------------------
    let mut touched: Vec<String> = Vec::new();

    for (layer, f) in r.files() {
        let want = render::file(r, layer, f, settings)?;
        let target = render::target_path(f, home);
        match std::fs::read_to_string(&target) {
            Ok(have) if have == want => {
                // Content is right; ownership and mode may still not be.
                if let Some(c) = permission_change(f, &target, user, &layer.name) {
                    changes.push(c);
                }
            }
            Ok(_) => {
                touched.push(f.path.clone());
                changes.push(Change::UpdateFile {
                    path: f.path.clone(),
                    layer: layer.name.clone(),
                })
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                touched.push(f.path.clone());
                changes.push(Change::CreateFile {
                    path: f.path.clone(),
                    layer: layer.name.clone(),
                })
            }
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                // Read-only commands run as the invoking user, so a
                // root-only file simply cannot be compared here. Say so
                // rather than pretending it matches.
                skipped.push(format!("{} (not readable as this user)", f.path));
            }
            Err(_) => {
                touched.push(f.path.clone());
                changes.push(Change::UpdateFile {
                    path: f.path.clone(),
                    layer: layer.name.clone(),
                })
            }
        }
    }

    // --- services ---------------------------------------------------------
    // Deliberately after files: a unit file lami is about to write may not
    // exist yet, and asking systemd about it first would say "not found".
    if systemd::available() {
        use crate::config::UnitState;
        for (layer, d) in r.services() {
            let unit = systemd::qualify(&d.name);
            let current = systemd::is_enabled_in(d.scope, user, &unit);
            let matches_want = matches!(
                (&d.state, &current),
                (UnitState::Enabled, State::Enabled)
                    | (UnitState::Disabled, State::Disabled)
                    | (UnitState::Masked, State::Masked)
            );
            // A unit lami is about to create does not exist yet; the state
            // change is real, it just cannot be observed before the write.
            let will_exist = current != State::NotFound
                || touched
                    .iter()
                    .any(|p| systemd::unit_of_path(p).as_deref() == Some(unit.as_str()));

            if !matches_want && will_exist {
                changes.push(Change::SetUnitState {
                    unit: unit.clone(),
                    layer: layer.name.clone(),
                    scope: d.scope,
                    want: d.state,
                    current: current.clone(),
                });
            }

            if d.restart_on_change {
                if let Some(p) = touched
                    .iter()
                    .find(|p| systemd::unit_of_path(p).as_deref() == Some(unit.as_str()))
                {
                    changes.push(Change::RestartUnit {
                        unit: unit.clone(),
                        scope: d.scope,
                        because: p.clone(),
                    });
                }
            }
        }

        // One reload per scope whose unit directory was written to. Forgetting
        // this leaves the unit silently running its old definition.
        for scope in [crate::config::Scope::System, crate::config::Scope::User] {
            if touched
                .iter()
                .any(|p| systemd::is_unit_path(p) == Some(scope))
            {
                changes.push(Change::DaemonReload { scope });
            }
        }
    } else {
        skipped.push("services (systemctl not available)".into());
    }

    // --- hooks ------------------------------------------------------------
    // A hook is only worth running if one of the files it watches is actually
    // going to change. Running mkinitcpio -P on every apply would work, but it
    // takes a minute and would hide what actually happened.
    for (layer, h) in r.hooks() {
        if let Some(w) = h.watch.iter().find(|w| touched.contains(w)) {
            changes.push(Change::RunHook {
                run: h.run.clone(),
                layer: layer.name.clone(),
                because: w.clone(),
            });
        }
    }

    // Group by kind so the output reads in sections rather than as a jumble.
    changes.sort_by_key(|c| match c.kind() {
        "packages" => 0,
        "files" => 1,
        "services" => 2,
        _ => 3,
    });

    Ok(Report {
        changes,
        undeclared_packages,
        skipped,
    })
}
