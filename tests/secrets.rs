//! Encrypted sources, end to end.
//!
//! The interface is the file name: a source ending in `.age` is decrypted on
//! the way out and re-encrypted on the way back. There is no flag to forget,
//! which is the point -- but it only holds if every path through the code
//! honours it, and one of them did not.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn have_age() -> bool {
    Command::new("age")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

struct Fixture {
    base: PathBuf,
    recipient: String,
}

impl Fixture {
    fn new(tag: &str, plaintext: &str) -> Fixture {
        let base = std::env::temp_dir().join(format!("lami-secret-{}-{tag}", std::process::id()));
        fs::remove_dir_all(&base).ok();
        let cfg = base.join("cfg");
        fs::create_dir_all(cfg.join("hosts")).unwrap();
        fs::create_dir_all(cfg.join("layers/only/files")).unwrap();

        // The identity lives outside the config directory, which is not a
        // detail: lami refuses one inside it.
        let identity = base.join("identity.txt");
        let out = Command::new("age-keygen")
            .arg("-o")
            .arg(&identity)
            .output()
            .expect("age-keygen runs");
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&identity, fs::Permissions::from_mode(0o600)).unwrap();
        }
        let pub_out = Command::new("age-keygen")
            .args(["-y"])
            .arg(&identity)
            .output()
            .expect("age-keygen -y runs");
        let recipient = String::from_utf8_lossy(&pub_out.stdout).trim().to_string();

        // Encrypt the secret into the layer.
        let source = cfg.join("layers/only/files/secret.age");
        let enc = Command::new("age")
            .args(["--encrypt", "--recipient", &recipient, "--output"])
            .arg(&source)
            .arg("-")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .spawn()
            .and_then(|mut c| {
                use std::io::Write;
                c.stdin.as_mut().unwrap().write_all(plaintext.as_bytes())?;
                c.wait()
            })
            .expect("age runs");
        assert!(enc.success());

        let target = base.join("target");
        fs::write(
            cfg.join("config.kdl"),
            format!(
                "age {{\n    identity \"{}\"\n    recipient \"{recipient}\"\n}}\n",
                identity.display()
            ),
        )
        .unwrap();
        fs::write(
            cfg.join("hosts/testbox.kdl"),
            "description \"fixture\"\nlayers \"only\"\n",
        )
        .unwrap();
        fs::write(
            cfg.join("layers/only/layer.kdl"),
            format!(
                "description \"fixture\"\n\nfile \"{}\" from=\"files/secret.age\"\n",
                target.display()
            ),
        )
        .unwrap();

        Fixture { base, recipient }
    }

    fn source(&self) -> PathBuf {
        self.base.join("cfg/layers/only/files/secret.age")
    }

    fn run(&self, args: &[&str]) -> (String, bool) {
        let out = Command::new(env!("CARGO_BIN_EXE_lami"))
            .args(["--config-dir", self.base.join("cfg").to_str().unwrap()])
            .args(["--host", "testbox"])
            .arg("--no-color")
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

#[test]
fn an_encrypted_source_renders_as_plaintext() {
    if !have_age() {
        eprintln!("age not installed, skipping");
        return;
    }
    let f = Fixture::new("render", "hunter2\nsecond line\n");
    let (out, ok) = f.run(&["render", f.base.join("target").to_str().unwrap()]);
    assert!(ok, "{out}");
    assert_eq!(out, "hunter2\nsecond line\n");
}

#[test]
fn a_decrypted_file_is_never_group_readable() {
    if !have_age() {
        return;
    }
    let f = Fixture::new("perms", "hunter2\n");
    let (out, ok) = f.run(&["why", f.base.join("target").to_str().unwrap()]);
    assert!(ok, "{out}");
    // 0600 regardless of what the path rules would otherwise give it.
    assert!(out.contains("600"), "{out}");
    assert!(out.contains("encrypted"), "and it says why:\n{out}");
}

#[test]
fn capture_re_encrypts_instead_of_writing_the_secret_through() {
    // The bug this test exists for: capture wrote the live file straight over
    // the source, so capturing an .age file put the secret into the repository
    // in the clear -- under a name ending in .age, which is the last place
    // anybody would look for a leak.
    if !have_age() {
        return;
    }
    let f = Fixture::new("capture", "hunter2\n");
    let target = f.base.join("target");
    fs::write(&target, "hunter2\nchanged on the machine\n").unwrap();

    let (out, ok) = f.run(&["capture", "--file", target.to_str().unwrap()]);
    assert!(ok, "{out}");

    let written = fs::read(f.source()).unwrap();
    assert!(
        written.starts_with(b"age-encryption.org/v1"),
        "the source must still be an age file"
    );
    assert!(
        !String::from_utf8_lossy(&written).contains("hunter2"),
        "the secret must not be in the repository in the clear"
    );

    // And the content actually round-tripped.
    let (rendered, ok) = f.run(&["render", target.to_str().unwrap()]);
    assert!(ok, "{rendered}");
    assert_eq!(rendered, "hunter2\nchanged on the machine\n");
}

#[test]
fn capture_does_not_print_the_secret() {
    // A terminal is scrollback, and a diff of ciphertext would say nothing
    // anyway: age picks a fresh file key every time.
    if !have_age() {
        return;
    }
    let f = Fixture::new("quiet", "hunter2\n");
    let target = f.base.join("target");
    fs::write(&target, "hunter2\nchanged\n").unwrap();
    let (out, ok) = f.run(&["capture", "--file", target.to_str().unwrap(), "--dry-run"]);
    assert!(ok, "{out}");
    assert!(!out.contains("hunter2"), "{out}");
    assert!(out.contains("re-encrypted"), "{out}");
}

#[test]
fn an_identity_inside_the_config_repo_is_refused() {
    // The mistake that would be silent and permanent. ~/.config/lami is
    // commonly a symlink to the repo, so this has to follow symlinks rather
    // than compare the paths as written.
    if !have_age() {
        return;
    }
    let f = Fixture::new("inside", "hunter2\n");
    fs::write(
        f.base.join("cfg/config.kdl"),
        format!(
            "age {{\n    identity \"identity.txt\"\n    recipient \"{}\"\n}}\n",
            f.recipient
        ),
    )
    .unwrap();
    let (out, ok) = f.run(&["show"]);
    assert!(!ok, "{out}");
    assert!(out.contains("inside the config repository"), "{out}");
    assert!(out.contains("publish your private key"), "{out}");
}
