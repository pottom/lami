//! Tests for `lami apply`.
//!
//! Everything that needs root is skipped when not running as root, so the
//! suite still passes for an ordinary contributor. What can be checked without
//! privileges -- the refusal itself, and that --dry-run writes nothing -- is
//! checked always.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn is_root() -> bool {
    Command::new("id")
        .arg("-u")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "0")
        .unwrap_or(false)
}

struct Fixture {
    base: PathBuf,
}

impl Fixture {
    fn new(tag: &str, layer_body: &str) -> Fixture {
        let base = std::env::temp_dir().join(format!("lami-apply-{}-{tag}", std::process::id()));
        fs::remove_dir_all(&base).ok();
        let cfg = base.join("cfg");
        fs::create_dir_all(cfg.join("hosts")).unwrap();
        fs::create_dir_all(cfg.join("layers/only")).unwrap();
        fs::write(
            cfg.join("hosts/testbox.kdl"),
            "description \"fixture\"\nlayers \"only\"\n",
        )
        .unwrap();
        fs::write(
            cfg.join("layers/only/layer.kdl"),
            format!("description \"fixture\"\n\n{}", layer_body.replace("{base}", base.to_str().unwrap())),
        )
        .unwrap();
        Fixture { base }
    }

    fn run(&self, args: &[&str]) -> (String, bool) {
        let exe = env!("CARGO_BIN_EXE_lami");
        let out = Command::new(exe)
            .args(["--config-dir", self.base.join("cfg").to_str().unwrap()])
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
        fs::remove_dir_all(&self.base).ok();
    }
}

const ONE_FILE: &str = "file \"{base}/target\" {\n    text \"content\"\n}\n";

#[test]
fn apply_refuses_without_root() {
    if is_root() {
        eprintln!("running as root, skipping");
        return;
    }
    let f = Fixture::new("noroot", ONE_FILE);
    let (out, ok) = f.run(&["apply"]);
    assert!(!ok, "must refuse:\n{out}");
    assert!(out.contains("needs root"), "{out}");
    assert!(out.contains("sudo lami apply"), "should say how:\n{out}");
    assert!(!f.base.join("target").exists(), "nothing may be written");
}

#[test]
fn dry_run_needs_no_root_and_writes_nothing() {
    let f = Fixture::new("dry", ONE_FILE);
    let (out, ok) = f.run(&["apply", "--dry-run"]);
    assert!(ok, "{out}");
    assert!(out.contains("nothing was applied"), "{out}");
    assert!(!f.base.join("target").exists(), "nothing may be written");
}

#[test]
fn apply_writes_and_is_idempotent() {
    if !is_root() {
        eprintln!("not root, skipping");
        return;
    }
    let f = Fixture::new("write", ONE_FILE);

    let (out, ok) = f.run(&["apply"]);
    assert!(ok, "{out}");
    assert_eq!(fs::read_to_string(f.base.join("target")).unwrap(), "content\n");

    let (again, ok) = f.run(&["apply"]);
    assert!(ok, "{again}");
    assert!(
        again.contains("already matches"),
        "a second apply must do nothing:\n{again}"
    );
}

#[test]
fn an_explicit_mode_is_honoured() {
    if !is_root() {
        eprintln!("not root, skipping");
        return;
    }
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new(
        "mode",
        "file \"{base}/secret\" mode=\"0600\" {\n    text \"shh\"\n}\n",
    );
    let (out, ok) = f.run(&["apply"]);
    assert!(ok, "{out}");
    let mode = fs::metadata(f.base.join("secret")).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600, "got {mode:o}");
}

#[test]
fn a_hook_runs_only_after_its_file_changes() {
    if !is_root() {
        eprintln!("not root, skipping");
        return;
    }
    let f = Fixture::new(
        "hook",
        "file \"{base}/watched\" {\n    text \"v1\"\n}\n\n\
         on-change \"{base}/watched\" {\n    run \"touch {base}/ran\"\n}\n",
    );

    let (out, ok) = f.run(&["apply"]);
    assert!(ok, "{out}");
    assert!(f.base.join("ran").exists(), "the hook should have run:\n{out}");

    fs::remove_file(f.base.join("ran")).unwrap();
    let (again, ok) = f.run(&["apply"]);
    assert!(ok, "{again}");
    assert!(
        !f.base.join("ran").exists(),
        "nothing changed, so the hook must not run again:\n{again}"
    );
}
