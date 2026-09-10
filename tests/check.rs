//! Tests for `lami check`.
//!
//! This command makes up for what was lost when the separate `aur` block went
//! away: since the config no longer states what comes from the AUR, we have to
//! CHECK that an AUR helper is present.

use std::process::Command;

fn has_pacman() -> bool {
    Command::new("pacman")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn lami_path(path: Option<&str>, args: &[&str]) -> (String, bool) {
    let exe = env!("CARGO_BIN_EXE_lami");
    let mut cmd = Command::new(exe);
    cmd.args(["--config-dir", "examples/minimal"]).args(args);
    if let Some(p) = path {
        cmd.env("PATH", p);
    }
    let out = cmd.output().expect("lami binary runs");
    (
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
        out.status.success(),
    )
}

#[test]
fn recognises_repo_packages() {
    if !has_pacman() {
        eprintln!("no pacman, skipping");
        return;
    }
    // sam has no rice layer, so every package of his comes from a repo.
    let (out, ok) = lami_path(None, &["--host", "sam", "check"]);
    assert!(ok, "{out}");
    assert!(out.contains("not in repos 0"), "{out}");
}

#[test]
fn separates_out_aur_packages() {
    if !has_pacman() {
        eprintln!("no pacman, skipping");
        return;
    }
    let (out, ok) = lami_path(None, &["--host", "frodo", "check"]);
    assert!(
        ok,
        "this machine has an AUR helper, so it should succeed:\n{out}"
    );
    assert!(out.contains("caelestia-shell"), "{out}");
}

#[test]
fn errors_without_an_aur_helper() {
    if !has_pacman() {
        eprintln!("no pacman, skipping");
        return;
    }
    // Expose pacman only, no AUR helper.
    let tmp = std::env::temp_dir().join(format!("lami-test-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    let link = tmp.join("pacman");
    let _ = std::fs::remove_file(&link);
    std::os::unix::fs::symlink("/usr/bin/pacman", &link).unwrap();

    let (out, ok) = lami_path(Some(tmp.to_str().unwrap()), &["--host", "frodo", "check"]);
    std::fs::remove_dir_all(&tmp).ok();

    assert!(!ok, "must exit with an error:\n{out}");
    assert!(out.contains("NO AUR helper"), "{out}");
    // The error should be actionable, not just a complaint.
    assert!(out.contains("makepkg -si"), "should offer a fix:\n{out}");
    assert!(
        out.contains("lami why"),
        "should point at where to fix it:\n{out}"
    );
}
