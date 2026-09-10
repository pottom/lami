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
    /// Declared, but the unit is not enabled.
    EnableService {
        unit: String,
        layer: String,
        current: State,
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
    pub fn line(&self) -> String {
        match self {
            Change::InstallPackage { name, layer } => format!("+ package  {name}  [{layer}]"),
            Change::EnableService {
                unit,
                layer,
                current,
            } => format!("+ service  {unit}  [{layer}]  (now: {current})"),
            Change::CreateFile { path, layer } => format!("+ file     {path}  [{layer}]"),
            Change::UpdateFile { path, layer } => format!("~ file     {path}  [{layer}]"),
            Change::FixPermissions {
                path,
                layer,
                want,
                have,
            } => format!("~ perms    {path}  [{layer}]  {have} -> {want}"),
            Change::RunHook {
                run,
                layer,
                because,
            } => format!("> run      {run}  [{layer}]  (because {because} changes)"),
        }
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
    let mut changes = Vec::new();
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

    // --- services ---------------------------------------------------------
    if systemd::available() {
        for (layer, d) in r.services() {
            let unit = systemd::qualify(&d.name);
            let state = systemd::is_enabled(&unit);
            if state != State::Enabled {
                changes.push(Change::EnableService {
                    unit,
                    layer: layer.name.clone(),
                    current: state,
                });
            }
        }
    } else {
        skipped.push("services (systemctl not available)".into());
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

    Ok(Report {
        changes,
        undeclared_packages,
        skipped,
    })
}
