//! The config repo: cloning it, finding it again, and syncing it.
//!
//! Everything here runs against a bare repo in a temp directory, so the tests
//! never touch the network or the author's real config.

use std::path::Path;
use std::process::Command;

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "test")
        .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
        .env("GIT_COMMITTER_NAME", "test")
        .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Run lami with a throwaway XDG_CONFIG_HOME, so the pointer file it reads and
/// writes is the test's own.
fn lami(xdg: &Path, args: &[&str]) -> (String, bool) {
    let out = Command::new(env!("CARGO_BIN_EXE_lami"))
        .args(args)
        .env("XDG_CONFIG_HOME", xdg)
        .env("LAMI_HOST", "sam")
        .env("GIT_AUTHOR_NAME", "test")
        .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
        .env("GIT_COMMITTER_NAME", "test")
        .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
        .env_remove("LAMI_CONFIG_DIR")
        .env_remove("LAMI_REPO")
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

/// A bare repo holding a copy of examples/minimal, plus a working copy of it.
fn origin(root: &Path) -> std::path::PathBuf {
    let work = root.join("seed");
    std::fs::create_dir_all(&work).unwrap();
    copy_tree(Path::new("examples/minimal"), &work);

    git(&work, &["init", "-q", "-b", "main"]);
    git(&work, &["add", "-A"]);
    git(&work, &["commit", "-qm", "seed"]);

    let bare = root.join("origin.git");
    let out = Command::new("git")
        .args(["clone", "--bare", "-q"])
        .arg(&work)
        .arg(&bare)
        .output()
        .expect("git runs");
    assert!(out.status.success());
    bare
}

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let dst = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_tree(&e.path(), &dst);
        } else {
            std::fs::copy(e.path(), dst).unwrap();
        }
    }
}

fn tmp() -> tempfile::TempDir {
    tempfile::tempdir().expect("temp dir")
}

#[test]
fn clone_records_where_the_repo_went() {
    let t = tmp();
    let bare = origin(t.path());
    let xdg = t.path().join("xdg");
    let dest = t.path().join("elsewhere/lami-config");

    let (out, ok) = lami(
        &xdg,
        &[
            "clone",
            &bare.to_string_lossy(),
            "--path",
            &dest.to_string_lossy(),
        ],
    );
    assert!(ok, "{out}");
    assert!(dest.join("hosts").is_dir(), "{out}");

    // The pointer file is what makes every later command work without flags.
    let ptr = std::fs::read_to_string(xdg.join("lami.kdl")).expect("pointer written");
    assert!(ptr.contains(&bare.to_string_lossy().to_string()), "{ptr}");
    assert!(ptr.contains(&dest.to_string_lossy().to_string()), "{ptr}");

    // ... and it does: no --config-dir here.
    let (out, ok) = lami(&xdg, &["show"]);
    assert!(ok, "{out}");
    assert!(out.contains("host: sam"), "{out}");
    assert!(out.contains("config repo:"), "{out}");
}

#[test]
fn clone_defaults_to_the_config_directory() {
    let t = tmp();
    let bare = origin(t.path());
    let xdg = t.path().join("xdg");

    let (out, ok) = lami(&xdg, &["clone", &bare.to_string_lossy()]);
    assert!(ok, "{out}");
    assert!(xdg.join("lami/hosts").is_dir(), "{out}");
}

#[test]
fn a_repo_url_alone_is_enough_to_bootstrap() {
    // The fresh-machine case: nothing cloned, nothing recorded, one flag.
    let t = tmp();
    let bare = origin(t.path());
    let xdg = t.path().join("xdg");

    let (out, ok) = lami(&xdg, &["--repo", &bare.to_string_lossy(), "show"]);
    assert!(ok, "{out}");
    assert!(out.contains("host: sam"), "{out}");
    assert!(xdg.join("lami/hosts").is_dir(), "{out}");
    // A one-off override is not recorded.
    assert!(!xdg.join("lami.kdl").exists(), "{out}");
}

#[test]
fn cloning_over_a_different_repo_is_refused() {
    let t = tmp();
    let bare = origin(t.path());
    let other = origin(&t.path().join("other"));
    let xdg = t.path().join("xdg");
    let dest = t.path().join("dest");

    let (_, ok) = lami(
        &xdg,
        &[
            "clone",
            &bare.to_string_lossy(),
            "--path",
            &dest.to_string_lossy(),
        ],
    );
    assert!(ok);

    let (out, ok) = lami(
        &xdg,
        &[
            "clone",
            &other.to_string_lossy(),
            "--path",
            &dest.to_string_lossy(),
        ],
    );
    assert!(!ok, "{out}");
    assert!(out.contains("different one"), "{out}");
}

#[test]
fn cloning_the_same_repo_again_is_harmless() {
    let t = tmp();
    let bare = origin(t.path());
    let xdg = t.path().join("xdg");
    let dest = t.path().join("dest");
    let args = [
        "clone",
        &*bare.to_string_lossy(),
        "--path",
        &*dest.to_string_lossy(),
    ];

    assert!(lami(&xdg, &args).1);
    let (out, ok) = lami(&xdg, &args);
    assert!(ok, "{out}");
    assert!(out.contains("already cloned"), "{out}");
}

#[test]
fn push_commits_and_pull_receives() {
    let t = tmp();
    let bare = origin(t.path());

    // Two machines, one origin.
    let a = t.path().join("a");
    let b = t.path().join("b");
    let xdg_a = a.join("xdg");
    let xdg_b = b.join("xdg");
    assert!(
        lami(
            &xdg_a,
            &[
                "clone",
                &bare.to_string_lossy(),
                "--path",
                &a.join("cfg").to_string_lossy()
            ]
        )
        .1
    );
    assert!(
        lami(
            &xdg_b,
            &[
                "clone",
                &bare.to_string_lossy(),
                "--path",
                &b.join("cfg").to_string_lossy()
            ]
        )
        .1
    );

    // Nothing to send yet.
    let (out, ok) = lami(&xdg_a, &["push"]);
    assert!(ok, "{out}");
    assert!(out.contains("nothing to push"), "{out}");

    std::fs::write(
        a.join("cfg/hosts/bilbo.kdl"),
        "description \"another\"\nlayers \"base\"\n",
    )
    .unwrap();
    let (out, ok) = lami(&xdg_a, &["push", "-m", "add bilbo"]);
    assert!(ok, "{out}");
    assert!(out.contains("staging"), "{out}");

    // The other machine sees it.
    let (out, ok) = lami(&xdg_b, &["pull"]);
    assert!(ok, "{out}");
    assert!(b.join("cfg/hosts/bilbo.kdl").is_file(), "{out}");
}

#[test]
fn pull_keeps_uncommitted_work() {
    let t = tmp();
    let bare = origin(t.path());
    let xdg = t.path().join("xdg");
    let cfg = t.path().join("cfg");
    assert!(
        lami(
            &xdg,
            &[
                "clone",
                &bare.to_string_lossy(),
                "--path",
                &cfg.to_string_lossy()
            ]
        )
        .1
    );

    // Deliberately not a valid host file: syncing the repo must not depend on
    // the config parsing, or a broken config would also block its own fix.
    std::fs::write(cfg.join("hosts/scratch.kdl"), "description \"wip\"\n").unwrap();
    let (out, ok) = lami(&xdg, &["pull"]);
    assert!(ok, "{out}");
    assert!(out.contains("uncommitted"), "{out}");
    assert!(
        cfg.join("hosts/scratch.kdl").is_file(),
        "the local edit survives"
    );
}

#[test]
fn a_missing_repo_says_how_to_get_one() {
    let t = tmp();
    let (out, ok) = lami(&t.path().join("empty"), &["show"]);
    assert!(!ok, "{out}");
    assert!(out.contains("lami clone"), "{out}");
}
