//! Tests for `lami diff`.
//!
//! These build a throwaway config in a temp directory so that the assertions
//! do not depend on whatever happens to be installed on the machine running
//! the suite.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

struct Fixture {
    dir: PathBuf,
    target: PathBuf,
}

impl Fixture {
    /// A config with one host, one layer, and one managed file pointing at
    /// `target`, whose content is the literal "hello".
    fn new(tag: &str) -> Fixture {
        let base = std::env::temp_dir().join(format!("lami-diff-{}-{tag}", std::process::id()));
        fs::remove_dir_all(&base).ok();
        let dir = base.join("config");
        fs::create_dir_all(dir.join("hosts")).unwrap();
        fs::create_dir_all(dir.join("layers/only")).unwrap();

        let target = base.join("managed.conf");

        fs::write(
            dir.join("hosts/testbox.kdl"),
            "description \"fixture\"\nlayers \"only\"\n",
        )
        .unwrap();
        fs::write(
            dir.join("layers/only/layer.kdl"),
            format!(
                "description \"fixture layer\"\n\nfile \"{}\" {{\n    text \"hello\"\n}}\n",
                target.display()
            ),
        )
        .unwrap();

        Fixture { dir, target }
    }

    fn run(&self, args: &[&str]) -> (String, bool) {
        let exe = env!("CARGO_BIN_EXE_lami");
        let out = Command::new(exe)
            .args(["--config-dir", self.dir.to_str().unwrap()])
            .args(["--host", "testbox"])
            .args(args)
            .output()
            .expect("lami binary runs");
        (
            format!(
                "{}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            ),
            out.status.success(),
        )
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(base) = self.dir.parent() {
            fs::remove_dir_all(base).ok();
        }
    }
}

#[test]
fn a_missing_file_is_reported_as_created() {
    let f = Fixture::new("missing");
    assert!(!f.target.exists());

    let (out, ok) = f.run(&["diff"]);
    assert!(ok, "{out}");
    assert!(out.contains("create"), "{out}");
    assert!(out.contains("managed.conf"), "{out}");
}

#[test]
fn a_matching_file_produces_no_change() {
    let f = Fixture::new("match");
    // Rendered content always ends with exactly one newline.
    fs::write(&f.target, "hello\n").unwrap();

    let (out, ok) = f.run(&["diff"]);
    assert!(ok, "{out}");
    // Two spellings, depending on whether the machine running the test has
    // packages no layer declares. Both start the same way.
    assert!(
        out.contains("Nothing to"),
        "a byte-identical file must not show up:\n{out}"
    );
}

#[test]
fn a_differing_file_is_reported_as_updated() {
    let f = Fixture::new("differ");
    fs::write(&f.target, "something else\n").unwrap();

    let (out, ok) = f.run(&["diff"]);
    assert!(ok, "{out}");
    assert!(out.contains("write"), "{out}");
}

#[test]
fn diff_changes_nothing_on_disk() {
    let f = Fixture::new("readonly");
    fs::write(&f.target, "something else\n").unwrap();
    let before = fs::read_to_string(&f.target).unwrap();

    f.run(&["diff"]);
    f.run(&["diff", "--undeclared"]);

    let after = fs::read_to_string(&f.target).unwrap();
    assert_eq!(before, after, "diff must not write anything");
}

#[test]
fn undeclared_packages_exclude_the_declared_ones() {
    // Against the real example config on a machine with pacman: whatever is
    // declared must never also be listed as undeclared.
    let has_pacman = Path::new("/usr/bin/pacman").exists();
    if !has_pacman {
        eprintln!("no pacman, skipping");
        return;
    }
    let exe = env!("CARGO_BIN_EXE_lami");
    let out = Command::new(exe)
        .args(["--config-dir", "examples/minimal", "--host", "frodo"])
        .args(["diff", "--undeclared"])
        .output()
        .expect("lami binary runs");
    let text = String::from_utf8_lossy(&out.stdout);

    // hyprland is declared by the gui layer, so it must not appear in the
    // undeclared list even though it is explicitly installed here.
    let undeclared_section = text.split("declared by no layer").nth(1).unwrap_or("");
    assert!(
        !undeclared_section.lines().any(|l| l.trim() == "hyprland"),
        "a declared package leaked into the undeclared list:\n{text}"
    );
}

/// A fixture with a file plus a hook watching it.
fn hook_fixture(tag: &str) -> (Fixture, PathBuf) {
    let f = Fixture::new(tag);
    let target = f.target.clone();
    fs::write(
        f.dir.join("layers/only/layer.kdl"),
        format!(
            "description \"fixture layer\"\n\n\
             file \"{0}\" {{\n    text \"hello\"\n}}\n\n\
             on-change \"{0}\" {{\n    run \"echo changed\"\n}}\n",
            target.display()
        ),
    )
    .unwrap();
    (f, target)
}

#[test]
fn a_hook_fires_only_when_its_file_changes() {
    // Running every hook on every apply would work, but mkinitcpio -P takes a
    // minute and would bury what actually happened.
    let (f, target) = hook_fixture("hook-fires");
    fs::write(&target, "something else\n").unwrap();

    let (out, ok) = f.run(&["diff"]);
    assert!(ok, "{out}");
    assert!(out.contains("run"), "the hook should fire:\n{out}");
    assert!(out.contains("because"), "it should say why:\n{out}");
}

#[test]
fn a_hook_stays_quiet_when_nothing_changes() {
    let (f, target) = hook_fixture("hook-quiet");
    fs::write(&target, "hello\n").unwrap();

    let (out, ok) = f.run(&["diff"]);
    assert!(ok, "{out}");
    assert!(
        out.contains("Nothing to"),
        "no file changed, so no hook should run:\n{out}"
    );
}

#[test]
fn a_wrong_mode_is_reported_even_when_content_matches() {
    let f = Fixture::new("mode");
    fs::write(&f.target, "hello\n").unwrap();

    // Default for a path outside a home directory is 0644.
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&f.target, fs::Permissions::from_mode(0o600)).unwrap();

    let (out, ok) = f.run(&["diff"]);
    assert!(ok, "{out}");
    assert!(out.contains("chmod"), "{out}");
    assert!(
        out.contains("mode is 600, should be 644"),
        "should say both modes in words:\n{out}"
    );
}
