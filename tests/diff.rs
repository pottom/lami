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
    assert!(out.contains("+ file"), "{out}");
    assert!(out.contains("managed.conf"), "{out}");
}

#[test]
fn a_matching_file_produces_no_change() {
    let f = Fixture::new("match");
    // Rendered content always ends with exactly one newline.
    fs::write(&f.target, "hello\n").unwrap();

    let (out, ok) = f.run(&["diff"]);
    assert!(ok, "{out}");
    assert!(
        !out.contains("file"),
        "a byte-identical file must not show up:\n{out}"
    );
}

#[test]
fn a_differing_file_is_reported_as_updated() {
    let f = Fixture::new("differ");
    fs::write(&f.target, "something else\n").unwrap();

    let (out, ok) = f.run(&["diff"]);
    assert!(ok, "{out}");
    assert!(out.contains("~ file"), "{out}");
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
