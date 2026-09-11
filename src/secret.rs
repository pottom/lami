//! Encrypted source files.
//!
//! A source file whose name ends in `.age` is decrypted on the way out. That
//! is the whole interface: no flag to remember, no way to accidentally commit
//! a secret in the clear because you forgot to mark it.
//!
//! Shells out to the `age` binary rather than linking a library, for the same
//! reason lami shells out to pacman: the Rust `age` crate is still marked
//! BETA, while `age` itself is a small, stable, packaged tool whose file
//! format is specified. Calling it also means a YubiKey works through
//! age-plugin-yubikey without lami knowing anything about smartcards.

use std::path::Path;
use std::process::{Command, Stdio};

use crate::error::{Error, Result};

/// Whether this source is encrypted, decided purely by its name.
pub fn is_encrypted(p: &Path) -> bool {
    p.extension().is_some_and(|e| e == "age")
}

fn age_available() -> bool {
    Command::new("age")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Decrypt an `.age` file using the configured identity.
pub fn decrypt(file: &Path, identity: Option<&Path>) -> Result<String> {
    if !age_available() {
        return Err(Error::Other(format!(
            "{} is encrypted, but `age` is not installed.\n\n  sudo pacman -S age",
            file.display()
        )));
    }
    let identity = identity.ok_or_else(|| {
        Error::Other(format!(
            "{} is encrypted, but no age identity is configured.\n\
             \nAdd one to config.kdl at the root of your config directory:\n\
             \n  age {{\n      identity \"~/.config/age/lami.txt\"\n  }}\n\
             \nKeep the key OUTSIDE the config repository: that directory gets\n\
             pushed, and a private key in it would go with it.",
            file.display()
        ))
    })?;

    if !identity.exists() {
        return Err(Error::Other(format!(
            "the age identity {} does not exist.\n\
             \nOn a new machine this is the one thing that cannot be automated:\n\
             the key has to get there some other way -- a YubiKey, a password\n\
             manager, or a USB stick.",
            identity.display()
        )));
    }

    // A private key anyone on the machine can read is not a private key. age
    // does not check this, and ssh's refusal to use a group-readable key has
    // taught everyone what the right behaviour is.
    if let Ok(meta) = std::fs::metadata(identity) {
        use std::os::unix::fs::PermissionsExt;
        let mode = meta.permissions().mode() & 0o777;
        if mode & 0o077 != 0 {
            return Err(Error::Other(format!(
                "the age identity {} is mode {mode:04o}, readable by others.\n\
                 \n  chmod 600 {}",
                identity.display(),
                identity.display()
            )));
        }
    }

    let out = Command::new("age")
        .arg("--decrypt")
        .arg("--identity")
        .arg(identity)
        .arg(file)
        .output()
        .map_err(|e| Error::Other(format!("cannot run age: {e}")))?;

    if !out.status.success() {
        return Err(Error::Other(format!(
            "cannot decrypt {}:\n{}",
            file.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    String::from_utf8(out.stdout)
        .map_err(|_| Error::Other(format!("{} did not decrypt to text", file.display())))
}

/// Encrypt content to the configured recipients, for `capture`.
pub fn encrypt(content: &str, recipients: &[String], dest: &Path) -> Result<()> {
    if recipients.is_empty() {
        return Err(Error::Other(format!(
            "{} is encrypted, but no age recipients are configured.\n\
             \nAdd at least one to config.kdl:\n\
             \n  age {{\n      recipient \"age1...\"\n  }}",
            dest.display()
        )));
    }
    let mut cmd = Command::new("age");
    cmd.arg("--encrypt");
    for r in recipients {
        cmd.arg("--recipient").arg(r);
    }
    cmd.arg("--output").arg(dest);
    cmd.stdin(Stdio::piped());

    let mut child = cmd
        .spawn()
        .map_err(|e| Error::Other(format!("cannot run age: {e}")))?;
    {
        use std::io::Write;
        let stdin = child.stdin.as_mut().expect("piped");
        stdin
            .write_all(content.as_bytes())
            .map_err(|e| Error::Other(format!("cannot write to age: {e}")))?;
    }
    let status = child
        .wait()
        .map_err(|e| Error::Other(format!("age failed: {e}")))?;
    if !status.success() {
        return Err(Error::Other(format!("age failed with {status}")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encryption_is_decided_by_the_file_name() {
        // No flag to forget. A source is encrypted because of what it is
        // called, so there is no way to commit a secret in the clear by
        // omitting an attribute.
        assert!(is_encrypted(Path::new("files/ssh-config.age")));
        assert!(!is_encrypted(Path::new("files/pacman.conf")));
    }
}
