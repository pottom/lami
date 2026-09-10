//! Tests for `lami prune`.
//!
//! prune is the only command that removes anything, so most of what is tested
//! here is what it REFUSES to do.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

struct Fixture {
    base: PathBuf,
}

impl Fixture {
    fn new(tag: &str) -> Fixture {
        let base = std::env::temp_dir().join(format!("lami-prune-{}-{tag}", std::process::id()));
        fs::remove_dir_all(&base).ok();
        fs::create_dir_all(base.join("cfg/hosts")).unwrap();
        fs::create_dir_all(base.join("cfg/layers/only")).unwrap();
        fs::write(
            base.join("cfg/hosts/testbox.kdl"),
            "description \"f\"\nlayers \"only\"\n",
        )
        .unwrap();
        fs::write(
            base.join("cfg/layers/only/layer.kdl"),
            "description \"f\"\n\npackages {\n}\n",
        )
        .unwrap();
        Fixture { base }
    }

    /// Pretend a previous apply managed these things.
    fn seed_state(&self, packages: &[&str], files: &[&str]) {
        let arr = |v: &[&str]| {
            v.iter()
                .map(|x| format!("    \"{x}\""))
                .collect::<Vec<_>>()
                .join(",\n")
        };
        fs::write(
            self.base.join("state.json"),
            format!(
                "{{\n  \"host\": \"testbox\",\n  \"packages\": [\n{}\n  ],\n  \"files\": [\n{}\n  ],\n  \"services\": [\n\n  ]\n}}\n",
                arr(packages),
                arr(files)
            ),
        )
        .unwrap();
    }

    fn run(&self, args: &[&str]) -> (String, bool) {
        let exe = env!("CARGO_BIN_EXE_lami");
        let out = Command::new(exe)
            .args(["--config-dir", self.base.join("cfg").to_str().unwrap()])
            .args(["--host", "testbox"])
            .args(args)
            .env("LAMI_STATE", self.base.join("state.json"))
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
        fs::remove_dir_all(&self.base).ok();
    }
}

#[test]
fn without_state_there_is_nothing_to_prune() {
    // A fresh machine has managed nothing, so nothing can be stale. This must
    // not be an error, and above all must not offer to remove anything.
    let f = Fixture::new("nostate");
    let (out, ok) = f.run(&["prune"]);
    assert!(ok, "{out}");
    assert!(out.contains("Nothing to prune"), "{out}");
}

#[test]
fn only_previously_managed_things_are_offered() {
    // The entire reason the state file exists: a package installed by hand and
    // a package dropped from a layer look identical from outside. Only the
    // second may ever be removed.
    let f = Fixture::new("managed");
    f.seed_state(&["was-managed"], &[]);

    let (out, ok) = f.run(&["prune"]);
    assert!(ok, "{out}");
    assert!(out.contains("was-managed"), "{out}");
    assert!(
        !out.contains("some-hand-installed-package"),
        "only recorded items may be offered:\n{out}"
    );
}

#[test]
fn nothing_is_removed_without_force() {
    // Removal must be asked for twice: once by choosing this command, once by
    // saying --force. A tool that deletes because of a mistyped subcommand is
    // not one to trust with root.
    let f = Fixture::new("noforce");
    let target = f.base.join("doomed");
    fs::write(&target, "still here\n").unwrap();
    f.seed_state(&[], &[target.to_str().unwrap()]);

    let (out, ok) = f.run(&["prune"]);
    assert!(ok, "{out}");
    assert!(out.contains("Nothing was removed"), "{out}");
    assert!(out.contains("--force"), "should say how to go ahead:\n{out}");
    assert!(target.exists(), "the file must still be there");
}

#[test]
fn a_still_declared_package_is_never_stale() {
    let f = Fixture::new("declared");
    fs::write(
        f.base.join("cfg/layers/only/layer.kdl"),
        "description \"f\"\n\npackages {\n    kept\n}\n",
    )
    .unwrap();
    f.seed_state(&["kept", "dropped"], &[]);

    let (out, ok) = f.run(&["prune"]);
    assert!(ok, "{out}");
    assert!(out.contains("dropped"), "{out}");
    assert!(
        !out.lines().any(|l| l.trim() == "- package  kept"),
        "a still-declared package must not be offered:\n{out}"
    );
}
