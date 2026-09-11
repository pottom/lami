//! One-off imperative steps.
//!
//! Every converging system has the same blind spot: the thing that has to
//! happen exactly once and cannot be described as a state. `sensors-detect`
//! writes a file whose contents only it knows; `virsh net-autostart` records
//! state inside libvirt that nothing should be writing by hand; an interactive
//! installer asks questions. None of these are "this file should look like
//! that", and pretending otherwise produces a config that lies.
//!
//! What makes it safe is the record: a migration that has run is written into
//! the state file by name, and never runs again.

use std::path::Path;
use std::process::Command;

use crate::error::{Error, Result};

/// The script's checksum, as `sha256sum` would print it.
///
/// Shelled out deliberately, rather than pulling in a hashing crate: the value
/// ends up in /var/lib/lami/state.json, and being able to check it with one
/// command you already have is the point of keeping that file readable.
pub fn checksum(script: &Path) -> Result<String> {
    let out = Command::new("sha256sum")
        .arg(script)
        .output()
        .map_err(|e| Error::Other(format!("cannot run sha256sum: {e}")))?;
    if !out.status.success() {
        return Err(Error::Other(format!(
            "cannot read {} to check it:\n{}",
            script.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_string())
}

/// Run a migration script.
///
/// Executed directly rather than through `bash`, so the shebang decides the
/// interpreter and a migration can be written in whatever suits it. It runs as
/// root, with the layer's own directory as the working directory, and is told
/// which machine it is on.
pub fn run(script: &Path, dir: &Path, host: &str, user: &str, home: &Path) -> Result<()> {
    if !script.is_file() {
        return Err(Error::Other(format!(
            "migration script not found: {}",
            script.display()
        )));
    }
    let executable = std::fs::metadata(script)
        .map(|m| {
            use std::os::unix::fs::PermissionsExt;
            m.permissions().mode() & 0o111 != 0
        })
        .unwrap_or(false);
    if !executable {
        return Err(Error::Other(format!(
            "{} is not executable.\n\
             It is run directly so that its shebang decides the interpreter:\n\
             \n  chmod +x {}",
            script.display(),
            script.display()
        )));
    }

    let status = Command::new(script)
        .current_dir(dir)
        .env("LAMI_HOST", host)
        .env("LAMI_USER", user)
        .env("LAMI_HOME", home)
        .status()
        .map_err(|e| Error::Other(format!("cannot run {}: {e}", script.display())))?;

    if !status.success() {
        return Err(Error::Other(format!(
            "{} exited with {status}.\n\
             It is NOT recorded as done, so it will be tried again next time.",
            script.display()
        )));
    }
    Ok(())
}
