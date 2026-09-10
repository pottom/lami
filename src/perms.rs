//! Working out a managed file's ownership and mode.
//!
//! Almost every file wants the obvious thing, so the obvious thing is the
//! default and only the exceptions are written down. This keeps a layer file
//! about *what* is managed rather than about chmod arithmetic.
//!
//! The rules are deliberately few. Inference that cannot be recited from
//! memory is worse than no inference at all, because then you have to look it
//! up anyway -- and `lami why` prints which rule applied, so it never has to
//! be guessed.

/// Ownership and mode, and why they are what they are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Perms {
    pub owner: String,
    pub group: String,
    pub mode: u32,
    /// Human-readable justification, shown by `lami why`.
    pub reason: String,
}

/// The default for a target path, before any explicit override.
///
/// `user` is the name the tool is acting for -- the owner of `~` paths.
pub fn infer(path: &str, user: &str) -> Perms {
    let home = |mode: u32, reason: &str| Perms {
        owner: user.to_string(),
        group: user.to_string(),
        mode,
        reason: reason.to_string(),
    };
    let root = |mode: u32, reason: &str| Perms {
        owner: "root".into(),
        group: "root".into(),
        mode,
        reason: reason.to_string(),
    };

    if let Some(rest) = path.strip_prefix("~/") {
        // Secrets must not be group- or world-readable, and several tools
        // refuse to start if they are.
        if rest.starts_with(".ssh/") {
            return home(0o600, "under ~/.ssh, which must not be readable by others");
        }
        if rest.starts_with(".gnupg/") {
            return home(0o600, "under ~/.gnupg, which must not be readable by others");
        }
        // Scripts placed on PATH are meant to be run.
        if rest.starts_with(".local/bin/") {
            return home(0o755, "under ~/.local/bin, so it is executable");
        }
        return home(0o644, "under the user's home");
    }

    // sudo silently ignores a drop-in that is not exactly 0440.
    if path.starts_with("/etc/sudoers.d/") {
        return root(0o440, "a sudoers drop-in, which sudo requires to be 0440");
    }
    if path.starts_with("/usr/local/bin/") || path.starts_with("/usr/local/sbin/") {
        return root(0o755, "on the system PATH, so it is executable");
    }

    root(0o644, "a system file")
}

/// Apply explicit overrides from the config on top of the inferred defaults.
pub fn with_overrides(
    mut p: Perms,
    owner: Option<&str>,
    group: Option<&str>,
    mode: Option<u32>,
) -> Perms {
    let mut explicit: Vec<&str> = Vec::new();
    if let Some(o) = owner {
        p.owner = o.to_string();
        explicit.push("owner");
    }
    if let Some(g) = group {
        p.group = g.to_string();
        explicit.push("group");
    }
    if let Some(m) = mode {
        p.mode = m;
        explicit.push("mode");
    }
    if !explicit.is_empty() {
        p.reason = format!("{} set explicitly in the layer", explicit.join(" and "));
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sudoers_dropins_are_0440() {
        // sudo skips a drop-in with any other mode, silently.
        assert_eq!(infer("/etc/sudoers.d/10-pottom", "pottom").mode, 0o440);
    }

    #[test]
    fn ordinary_etc_files_are_0644_root() {
        let p = infer("/etc/pacman.conf", "pottom");
        assert_eq!((p.mode, p.owner.as_str()), (0o644, "root"));
    }

    #[test]
    fn home_files_belong_to_the_user() {
        let p = infer("~/.config/foo.conf", "pottom");
        assert_eq!((p.mode, p.owner.as_str()), (0o644, "pottom"));
    }

    #[test]
    fn scripts_on_path_are_executable() {
        assert_eq!(infer("~/.local/bin/my-script", "pottom").mode, 0o755);
    }

    #[test]
    fn secrets_are_not_readable_by_others() {
        assert_eq!(infer("~/.ssh/config", "pottom").mode, 0o600);
    }

    #[test]
    fn an_explicit_mode_wins_and_says_so() {
        // /etc/snapper/configs/root is 0640, which no path rule could guess.
        let p = with_overrides(infer("/etc/snapper/configs/root", "pottom"), None, None, Some(0o640));
        assert_eq!(p.mode, 0o640);
        assert!(p.reason.contains("explicitly"), "{}", p.reason);
    }
}
