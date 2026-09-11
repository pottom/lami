//! Group membership.
//!
//! The fifth resource, and the last thing a machine's configuration consists
//! of that had no home here. It came up over libvirt, where the documented way
//! to let somebody manage VMs is to add them to the `libvirt` group -- and the
//! only way to express that in a config was not to.
//!
//! Shelled out to, like everything else: `id` and `getent` go through NSS, so
//! a group that comes from LDAP or sssd is seen exactly as the rest of the
//! system sees it. Reading /etc/group directly would quietly work on this
//! machine and quietly not on somebody else's.

use std::collections::BTreeSet;
use std::process::Command;

use crate::error::{Error, Result};

fn run(bin: &str, args: &[&str]) -> Result<String> {
    let out = Command::new(bin)
        .args(args)
        .output()
        .map_err(|e| Error::Other(format!("cannot run '{bin}': {e}")))?;
    if !out.status.success() {
        return Err(Error::Other(format!(
            "'{bin} {}' failed:\n{}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The groups a user is in, primary group included.
pub fn of_user(user: &str) -> Result<BTreeSet<String>> {
    Ok(run("id", &["-nG", user])?
        .split_whitespace()
        .map(str::to_string)
        .collect())
}

/// Every group that exists on this machine.
///
/// Needed because a declared group that does not exist is a different problem
/// from one the user is merely not in: `gpasswd` cannot create it, and the
/// package that should have is missing.
pub fn existing() -> Result<BTreeSet<String>> {
    Ok(run("getent", &["group"])?
        .lines()
        .filter_map(|l| l.split(':').next())
        .filter(|n| !n.is_empty())
        .map(str::to_string)
        .collect())
}

pub fn available() -> bool {
    Command::new("id")
        .arg("-nG")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Add a user to a group.
pub fn add(user: &str, group: &str) -> Result<()> {
    run("gpasswd", &["-a", user, group])?;
    Ok(())
}

/// Remove a user from a group. Only ever called by `prune`, and only for a
/// membership a previous `apply` recorded as its own.
pub fn remove(user: &str, group: &str) -> Result<()> {
    run("gpasswd", &["-d", user, group])?;
    Ok(())
}
