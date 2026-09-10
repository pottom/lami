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
            State::Other(s) => write!(f, "{s}"),
            State::NotFound => write!(f, "not found"),
        }
    }
}

pub fn is_enabled(unit: &str) -> State {
    let out = match Command::new("systemctl").args(["is-enabled", unit]).output() {
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
