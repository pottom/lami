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
