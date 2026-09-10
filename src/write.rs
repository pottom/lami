//! Writing a managed file safely.
//!
//! Three things have to hold at once, and each of them has bitten real tools:
//!
//! 1. **Atomic.** A half-written /etc/sudoers.d entry or fstab is a bad day.
//!    Write to a temp file, then rename over the target.
//!
//! 2. **Same filesystem.** rename(2) cannot cross filesystems, and on a stock
//!    Arch btrfs layout `@` and `@home` ARE separate filesystems for rename
//!    even though they live on one device. /tmp is usually tmpfs, so staging
//!    there and renaming into $HOME fails with EXDEV on most Arch installs.
//!    The temp file therefore goes in the TARGET's own directory.
//!
//! 3. **No TOCTOU.** We are a root process writing into directories an
//!    unprivileged user controls. Between a stat and an open, the path can be
//!    swapped for a symlink to /etc/shadow. So ownership and mode are set on
//!    the FILE DESCRIPTOR, never on a path.

use std::io::Write;
use std::os::unix::io::AsRawFd;
use std::path::Path;

use crate::error::{Error, Result};

/// Look up a user's uid, or None if there is no such user.
pub fn uid_of(name: &str) -> Option<u32> {
    let c = std::ffi::CString::new(name).ok()?;
    let pw = unsafe { libc::getpwnam(c.as_ptr()) };
    if pw.is_null() {
        None
    } else {
        Some(unsafe { (*pw).pw_uid })
    }
}

/// Look up a group's gid, or None if there is no such group.
pub fn gid_of(name: &str) -> Option<u32> {
    let c = std::ffi::CString::new(name).ok()?;
    let gr = unsafe { libc::getgrnam(c.as_ptr()) };
    if gr.is_null() {
        None
    } else {
        Some(unsafe { (*gr).gr_gid })
    }
}

/// Write `content` to `target` with the given ownership and mode.
pub fn write(target: &Path, content: &str, owner: &str, group: &str, mode: u32) -> Result<()> {
    let parent = target.parent().ok_or_else(|| {
        Error::Other(format!("{} has no parent directory", target.display()))
    })?;

    std::fs::create_dir_all(parent).map_err(|source| Error::Io {
        path: parent.to_path_buf(),
        source,
    })?;

    let uid = uid_of(owner)
        .ok_or_else(|| Error::Other(format!("no such user: {owner}")))?;
    let gid = gid_of(group)
        .ok_or_else(|| Error::Other(format!("no such group: {group}")))?;

    // In the target's own directory, so the rename below cannot hit EXDEV.
    let mut tmp = tempfile::NamedTempFile::new_in(parent).map_err(|source| Error::Io {
        path: parent.to_path_buf(),
        source,
    })?;

    tmp.write_all(content.as_bytes()).map_err(|source| Error::Io {
        path: target.to_path_buf(),
        source,
    })?;

    let fd = tmp.as_file().as_raw_fd();

    // Ownership first: chown clears the setuid and setgid bits, so doing it
    // after chmod would silently drop them.
    if unsafe { libc::fchown(fd, uid, gid) } != 0 {
        return Err(Error::Other(format!(
            "cannot set owner on {}: {}",
            target.display(),
            std::io::Error::last_os_error()
        )));
    }
    if unsafe { libc::fchmod(fd, mode) } != 0 {
        return Err(Error::Other(format!(
            "cannot set mode on {}: {}",
            target.display(),
            std::io::Error::last_os_error()
        )));
    }

    tmp.as_file().sync_all().map_err(|source| Error::Io {
        path: target.to_path_buf(),
        source,
    })?;

    tmp.persist(target)
        .map_err(|e| Error::Other(format!("cannot replace {}: {}", target.display(), e.error)))?;

    // Without this the rename itself is not durable across a power loss.
    if let Ok(dir) = std::fs::File::open(parent) {
        let _ = dir.sync_all();
    }
    Ok(())
}
