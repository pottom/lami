//! One-off imperative steps.
//!
//! The blind spot every converging system has: the thing that has to happen
//! exactly once and cannot be described as a state. What matters here is that
//! it happens once and is remembered -- which can be checked without running
//! anything, by writing the state file the test wants to see.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

struct Fixture {
    base: PathBuf,
}

impl Fixture {
    fn new(tag: &str, layer_body: &str) -> Fixture {
        let base = std::env::temp_dir().join(format!("lami-mig-{}-{tag}", std::process::id()));
        fs::remove_dir_all(&base).ok();
        let cfg = base.join("cfg");
        fs::create_dir_all(cfg.join("hosts")).unwrap();
        fs::create_dir_all(cfg.join("layers/only/scripts")).unwrap();
        fs::write(
            cfg.join("hosts/testbox.kdl"),
            "description \"fixture\"\nlayers \"only\"\n",
        )
        .unwrap();
        fs::write(
            cfg.join("layers/only/layer.kdl"),
            format!("description \"fixture\"\n\n{layer_body}"),
        )
        .unwrap();
        let script = cfg.join("layers/only/scripts/step.sh");
        fs::write(&script, "#!/usr/bin/env bash\nexit 0\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        }
        Fixture { base }
    }

    /// Pretend a migration has already run, by writing the state file lami
    /// would have written. Its checksum is deliberately wrong unless given.
    fn mark_done(&self, name: &str, checksum: &str) {
        fs::write(
            self.base.join("state.json"),
            format!(
                "{{\n  \"host\": \"testbox\",\n  \"packages\": [\n\n  ],\n  \"groups\": [\n\n  ],\
                 \n  \"files\": [\n\n  ],\n  \"services\": [\n\n  ],\n  \"user_services\": [\n\n  ],\
                 \n  \"migrations\": {{\n    \"{name}\": \"{checksum}\"\n  }}\n}}\n"
            ),
        )
        .unwrap();
    }

    fn checksum(&self) -> String {
        let out = Command::new("sha256sum")
            .arg(self.base.join("cfg/layers/only/scripts/step.sh"))
            .output()
            .expect("sha256sum runs");
        String::from_utf8_lossy(&out.stdout)
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .to_string()
    }

    fn run(&self, args: &[&str]) -> (String, bool) {
        let out = Command::new(env!("CARGO_BIN_EXE_lami"))
            .args(["--config-dir", self.base.join("cfg").to_str().unwrap()])
            .args(["--host", "testbox"])
            .arg("--no-color")
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

const ONE: &str = "migration \"step\" from=\"scripts/step.sh\" \
                   because=\"writes a file only this script knows\"\n";

#[test]
fn one_that_has_never_run_is_a_change() {
    let f = Fixture::new("pending", ONE);
    let (out, ok) = f.run(&["diff"]);
    assert!(ok, "{out}");
    assert!(out.contains("migrations"), "{out}");
    assert!(out.contains("step"), "{out}");
    assert!(
        out.contains("writes a file only this script knows"),
        "the reason is in the diff, not just in the file:\n{out}"
    );
}

#[test]
fn one_that_has_run_is_not_offered_again() {
    // The whole point. It is keyed by name, so this holds however the script
    // is edited or moved afterwards.
    let f = Fixture::new("done", ONE);
    f.mark_done("step", &f.checksum());
    let (out, ok) = f.run(&["diff"]);
    assert!(ok, "{out}");
    assert!(!out.contains("  run "), "{out}");
}

#[test]
fn renaming_one_makes_it_run_again() {
    // The escape hatch, and a truthful record: under a new name it is a
    // different step, which is exactly what re-running it means.
    let f = Fixture::new("renamed", ONE);
    f.mark_done("step", &f.checksum());
    fs::write(
        f.base.join("cfg/layers/only/layer.kdl"),
        format!(
            "description \"fixture\"\n\n{}",
            ONE.replace("\"step\"", "\"step-again\"")
        ),
    )
    .unwrap();
    let (out, ok) = f.run(&["diff"]);
    assert!(ok, "{out}");
    assert!(out.contains("step-again"), "{out}");
}

#[test]
fn an_edited_script_is_reported_but_not_re_run() {
    let f = Fixture::new("edited", ONE);
    f.mark_done(
        "step",
        "0000000000000000000000000000000000000000000000000000000000000000",
    );

    let (check, ok) = f.run(&["check"]);
    assert!(ok, "{check}");
    assert!(check.contains("edited since they ran"), "{check}");

    let (diff, ok) = f.run(&["diff"]);
    assert!(ok, "{diff}");
    assert!(!diff.contains("  run "), "still not re-run:\n{diff}");
}

#[test]
fn why_says_whether_it_has_run() {
    let f = Fixture::new("why", ONE);
    let (out, ok) = f.run(&["why", "step"]);
    assert!(ok, "{out}");
    assert!(out.contains("(migration)"), "{out}");
    assert!(out.contains("has not run on this machine"), "{out}");

    f.mark_done("step", &f.checksum());
    let (out, _) = f.run(&["why", "step"]);
    assert!(out.contains("already run"), "{out}");
}

#[test]
fn a_migration_must_say_why_it_exists() {
    // A one-off runs once and is never seen again. If it does not explain
    // itself here, nothing does.
    let f = Fixture::new("nobecause", "migration \"step\" from=\"scripts/step.sh\"\n");
    let (out, ok) = f.run(&["show"]);
    assert!(!ok, "{out}");
    assert!(out.contains("does not say why it exists"), "{out}");
}

#[test]
fn a_migration_needs_a_script() {
    let f = Fixture::new("noscript", "migration \"step\" because=\"...\"\n");
    let (out, ok) = f.run(&["show"]);
    assert!(!ok, "{out}");
    assert!(out.contains("has no script"), "{out}");
}

#[test]
fn a_conditional_migration_follows_the_parameter() {
    let f = Fixture::new(
        "cond",
        &format!(
            "params {{\n    virt \"run virtual machines here\" default=off\n}}\n\
             when virt=on {{\n    {ONE}}}\n"
        ),
    );
    let (out, ok) = f.run(&["diff"]);
    assert!(ok, "{out}");
    assert!(!out.contains("  run "), "off by default:\n{out}");
}
