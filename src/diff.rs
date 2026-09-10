//! Comparing the declared state against the machine.
//!
//! This is the gate before anything is ever written: the output has to line up
//! with what the existing tooling reports before `apply` is trusted at all.

use crate::config::Resolved;
use crate::error::Result;
use crate::systemd::{self, State};
use crate::{pacman, render};

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
    /// Declared, and the file on disk differs.
    UpdateFile { path: String, layer: String },
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

pub fn compute(r: &Resolved<'_>, home: &std::path::Path) -> Result<Report> {
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
    for (layer, f) in r.files() {
        let want = render::file(r, layer, f)?;
        let target = render::target_path(f, home);
        match std::fs::read_to_string(&target) {
            Ok(have) if have == want => {}
            Ok(_) => changes.push(Change::UpdateFile {
                path: f.path.clone(),
                layer: layer.name.clone(),
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
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
            Err(_) => changes.push(Change::UpdateFile {
                path: f.path.clone(),
                layer: layer.name.clone(),
            }),
        }
    }

    Ok(Report {
        changes,
        undeclared_packages,
        skipped,
    })
}
