//! Querying systemd unit state.
//!
//! Shells out to `systemctl` rather than talking D-Bus. The decisive reason is
//! user scope: reaching a user's systemd instance over D-Bus from a root
//! process means locating /run/user/<uid>/bus and setting
//! DBUS_SESSION_BUS_ADDRESS, and that socket only exists if the user has a live
//! session or lingering enabled. `systemctl --user -M <user>@` routes via
//! systemd-machined and simply works.

use std::process::Command;

/// Unit names may be written without their suffix in the config, because
/// `services { greetd }` reads better than `services { greetd.service }`.
pub fn qualify(name: &str) -> String {
    const SUFFIXES: &[&str] = &[
        ".service", ".socket", ".timer", ".target", ".mount", ".path", ".slice",
    ];
    if SUFFIXES.iter().any(|s| name.ends_with(s)) {
        name.to_string()
    } else {
        format!("{name}.service")
    }
}

pub fn available() -> bool {
    Command::new("systemctl")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// What `systemctl is-enabled` reports for a unit.
///
/// systemctl exits non-zero for "disabled" and for unknown units alike, so the
/// stdout word is what matters, not the exit status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    Enabled,
    Disabled,
    Masked,
    /// static, masked, indirect, generated, ...
    Other(String),
    /// No such unit on this system.
    NotFound,
}

impl std::fmt::Display for State {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            State::Enabled => write!(f, "enabled"),
            State::Disabled => write!(f, "disabled"),
            State::Masked => write!(f, "masked"),
            State::Other(s) => write!(f, "{s}"),
            State::NotFound => write!(f, "not found"),
        }
    }
}

/// Build a systemctl invocation for the right scope.
///
/// User units are reached with `--user -M <user>@`, which routes through
/// systemd-machined. Talking to the user's bus directly would mean locating
/// /run/user/<uid>/bus and setting DBUS_SESSION_BUS_ADDRESS, and that socket
/// only exists while the user has a live session or lingering enabled -- so it
/// would work when run from a terminal and fail from a boot-time context.
pub fn cmd(scope: crate::config::Scope, user: &str) -> Command {
    let mut c = Command::new("systemctl");
    if scope == crate::config::Scope::User {
        c.args(["--user", "-M", &format!("{user}@")]);
    }
    c
}

pub fn is_enabled_in(scope: crate::config::Scope, user: &str, unit: &str) -> State {
    let out = match cmd(scope, user).args(["is-enabled", unit]).output() {
        Ok(o) => o,
        Err(_) => return State::NotFound,
    };
    classify(&String::from_utf8_lossy(&out.stdout))
}

fn classify(word: &str) -> State {
    match word.trim() {
        "enabled" | "enabled-runtime" => State::Enabled,
        "disabled" => State::Disabled,
        "masked" | "masked-runtime" => State::Masked,
        "" | "not-found" => State::NotFound,
        other => State::Other(other.to_string()),
    }
}

/// Whether a path is a systemd unit directory, in which case writing to it
/// needs a daemon-reload afterwards.
///
/// Forgetting the reload is a nasty failure: the unit silently keeps running
/// its old definition, and nothing says so. Inferring it from the path means
/// it cannot be forgotten.
pub fn is_unit_path(path: &str) -> Option<crate::config::Scope> {
    use crate::config::Scope;
    if path.starts_with("/etc/systemd/system/") || path.starts_with("/usr/lib/systemd/system/") {
        return Some(Scope::System);
    }
    if path.starts_with("~/.config/systemd/user/") || path.contains("/.config/systemd/user/") {
        return Some(Scope::User);
    }
    None
}

/// The unit a path belongs to, so a change to a drop-in restarts the right
/// thing: `/etc/systemd/system/foo.service.d/x.conf` -> `foo.service`.
pub fn unit_of_path(path: &str) -> Option<String> {
    let name = path.rsplit('/').find(|p| !p.is_empty())?;
    if let Some(dir) = path.split('/').rev().nth(1) {
        if let Some(base) = dir.strip_suffix(".d") {
            return Some(base.to_string());
        }
    }
    if name.contains('.') {
        Some(name.to_string())
    } else {
        None
    }
}

pub fn is_enabled(unit: &str) -> State {
    let out = match Command::new("systemctl")
        .args(["is-enabled", unit])
        .output()
    {
        Ok(o) => o,
        Err(_) => return State::NotFound,
    };
    let word = String::from_utf8_lossy(&out.stdout).trim().to_string();
    match word.as_str() {
        "enabled" | "enabled-runtime" => State::Enabled,
        "disabled" => State::Disabled,
        "" | "not-found" => State::NotFound,
        other => State::Other(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Scope;

    #[test]
    fn a_suffix_is_only_added_when_missing() {
        assert_eq!(qualify("greetd"), "greetd.service");
        assert_eq!(qualify("fstrim.timer"), "fstrim.timer");
        assert_eq!(qualify("pcscd.socket"), "pcscd.socket");
    }

    #[test]
    fn unit_directories_are_recognised() {
        assert_eq!(
            is_unit_path("/etc/systemd/system/backup.timer"),
            Some(Scope::System)
        );
        assert_eq!(
            is_unit_path("~/.config/systemd/user/sync.service"),
            Some(Scope::User)
        );
        assert_eq!(is_unit_path("/etc/pacman.conf"), None);
    }

    #[test]
    fn a_dropin_belongs_to_its_unit() {
        // Writing /etc/systemd/system/foo.service.d/override.conf has to
        // restart foo.service, not something called override.conf.
        assert_eq!(
            unit_of_path("/etc/systemd/system/foo.service.d/override.conf").as_deref(),
            Some("foo.service")
        );
        assert_eq!(
            unit_of_path("/etc/systemd/system/backup.timer").as_deref(),
            Some("backup.timer")
        );
    }
}

/// A unit somebody turned on, or masked, against its package's preset.
///
/// This is the units' answer to `pacman -Qqe`, and it needs the same care: a
/// machine has hundreds of unit files, and almost none of their states are
/// decisions.
///
/// Arch does not run `systemctl preset-all`, so a unit sitting at `disabled`
/// says nothing at all -- it is where everything starts, and "deliberately
/// off" cannot be told from "never touched". Reporting those would bury the
/// real answer in sixty lines of systemd's own units. So only two states
/// count as a choice: enabled where the preset says otherwise, and masked,
/// which no preset ever asks for.
#[derive(Debug, Clone)]
pub struct Deviation {
    pub unit: String,
    pub scope: crate::config::Scope,
    pub state: crate::config::UnitState,
}

/// Every unit this machine has been told to enable or mask.
///
/// Read from the directory systemd reserves for exactly that -- what an
/// administrator changed -- rather than inferred from `list-unit-files`.
///
/// The first attempt compared each unit's state with its package's preset, on
/// the theory that a disagreement is a decision. On Arch it is not: nothing
/// runs `systemctl preset-all`, so half of systemd's own units sit at
/// `disabled` with a preset of `enabled` and would have been reported as
/// deliberate choices. The symlinks under /etc say what was actually done,
/// with no guessing.
pub fn deviations(scope: crate::config::Scope, home: &std::path::Path) -> Vec<Deviation> {
    use crate::config::{Scope, UnitState};

    let root = match scope {
        Scope::System => std::path::PathBuf::from("/etc/systemd/system"),
        // NOT /etc/systemd/user: that is the global default for every user,
        // and on Arch it is where package presets land. This is the one the
        // person sitting here chose.
        Scope::User => home.join(".config/systemd/user"),
    };

    let mut out = Vec::new();
    let mut walk = vec![root.clone()];
    while let Some(dir) = walk.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            let path = e.path();
            let Ok(meta) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if meta.is_dir() {
                // Only one level down: *.wants/ and *.requires/ hold the
                // enablement symlinks, and nothing nests deeper.
                if dir == root {
                    walk.push(path);
                }
                continue;
            }
            if !meta.is_symlink() {
                // A real file here is a unit written by hand or by lami, not
                // a statement about some other unit's state.
                continue;
            }
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            let masked = std::fs::read_link(&path)
                .map(|t| t == std::path::Path::new("/dev/null"))
                .unwrap_or(false);
            // A symlink at the top level that is not a mask is an alias, not
            // an enablement.
            let state = if masked {
                UnitState::Masked
            } else if dir != root {
                UnitState::Enabled
            } else {
                continue;
            };
            out.push(Deviation {
                unit: name.to_string(),
                scope,
                state,
            });
        }
    }
    out.sort_by(|a, b| a.unit.cmp(&b.unit));
    out.dedup_by(|a, b| a.unit == b.unit && a.state == b.state);
    out
}

/// The units a unit's `[Install] Also=` drags along with it.
///
/// `systemctl enable NetworkManager` also enables
/// NetworkManager-dispatcher.service, and enabling one virtqemud socket
/// enables its -ro and -admin siblings. Those are consequences of a decision,
/// not decisions, and offering them for capture would be noise.
///
/// Read from `systemctl cat`, which is the file as it actually applies --
/// drop-ins and /etc overrides included. `systemctl show` does not expose the
/// property at all.
pub fn also_of(scope: crate::config::Scope, user: &str, unit: &str) -> Vec<String> {
    let out = match cmd(scope, user).args(["cat", unit]).output() {
        Ok(o) if o.status.success() => o,
        _ => return Vec::new(),
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| l.trim().strip_prefix("Also=").map(str::to_string))
        .flat_map(|v| v.split_whitespace().map(qualify).collect::<Vec<_>>())
        .collect()
}
