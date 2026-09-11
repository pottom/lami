//! Group membership: the fifth resource.
//!
//! Everything here is read-only -- joining a group needs root, and the suite
//! has to pass for an ordinary contributor. What can be checked without
//! privileges is that lami sees the three states correctly: already a member,
//! not a member, and no such group.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn out_of(cmd: &str, args: &[&str]) -> String {
    let out = Command::new(cmd).args(args).output().expect("runs");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// A group this user is definitely in: their own primary group.
fn a_group_i_am_in() -> String {
    out_of("id", &["-gn"])
}

/// A group that exists but this user is not in, or None on a machine where
/// there is no such thing.
fn a_group_i_am_not_in() -> Option<String> {
    let mine: Vec<String> = out_of("id", &["-nG"])
        .split_whitespace()
        .map(str::to_string)
        .collect();
    out_of("getent", &["group"])
        .lines()
        .filter_map(|l| l.split(':').next().map(str::to_string))
        .find(|g| !g.is_empty() && !mine.contains(g))
}

struct Fixture {
    dir: PathBuf,
}

impl Fixture {
    fn new(tag: &str, groups: &str) -> Fixture {
        let dir = std::env::temp_dir().join(format!("lami-groups-{}-{tag}", std::process::id()));
        fs::remove_dir_all(&dir).ok();
        fs::create_dir_all(dir.join("hosts")).unwrap();
        fs::create_dir_all(dir.join("layers/only")).unwrap();
        fs::write(
            dir.join("layers/only/layer.kdl"),
            format!("description \"fixture\"\n\ngroups {{\n{groups}}}\n"),
        )
        .unwrap();
        fs::write(
            dir.join("hosts/testbox.kdl"),
            "description \"fixture\"\nlayers \"only\"\n",
        )
        .unwrap();
        Fixture { dir }
    }

    fn run(&self, args: &[&str]) -> (String, bool) {
        let out = Command::new(env!("CARGO_BIN_EXE_lami"))
            .args(["--config-dir", self.dir.to_str().unwrap()])
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
        fs::remove_dir_all(&self.dir).ok();
    }
}

#[test]
fn a_membership_already_held_is_not_a_change() {
    let f = Fixture::new("held", &format!("    {}\n", a_group_i_am_in()));
    let (out, ok) = f.run(&["diff"]);
    assert!(ok, "{out}");
    assert!(!out.contains("join"), "{out}");
}

#[test]
fn a_missing_membership_is_a_change() {
    let Some(g) = a_group_i_am_not_in() else {
        eprintln!("this user is in every group on the machine, skipping");
        return;
    };
    let f = Fixture::new("missing", &format!("    {g}\n"));
    let (out, ok) = f.run(&["diff"]);
    assert!(ok, "{out}");
    assert!(out.contains("groups"), "{out}");
    assert!(out.contains("join"), "{out}");
    assert!(out.contains(&g), "{out}");
    assert!(out.contains("not a member"), "{out}");
}

#[test]
fn a_group_that_does_not_exist_is_a_problem_not_a_change() {
    // gpasswd cannot create a group, and neither should a config: the package
    // that needs one creates it, with the right gid. So this is somebody's
    // missing package, and apply has nothing it could do about it.
    let f = Fixture::new("ghost", "    lami-no-such-group\n");
    let (out, ok) = f.run(&["diff"]);
    assert!(ok, "{out}");
    assert!(out.contains("problems"), "{out}");
    assert!(out.contains("does not exist on this machine"), "{out}");
    // The word appears in the problem text; what must not appear is the
    // change line, which starts with the verb in its own column.
    assert!(!out.contains("  join "), "not offered as a change:\n{out}");
}

#[test]
fn why_explains_where_a_group_comes_from() {
    let g = a_group_i_am_in();
    let f = Fixture::new("why", &format!("    {g}\n"));
    let (out, ok) = f.run(&["why", &g]);
    assert!(ok, "{out}");
    assert!(out.contains("(group)"), "{out}");
    assert!(out.contains("layer 'only'"), "{out}");
}

#[test]
fn show_counts_them() {
    let f = Fixture::new("count", &format!("    {}\n", a_group_i_am_in()));
    let (out, ok) = f.run(&["show"]);
    assert!(ok, "{out}");
    assert!(out.contains("groups     1"), "{out}");
}

#[test]
fn a_conditional_group_follows_the_parameter() {
    let dir = std::env::temp_dir().join(format!("lami-groups-cond-{}", std::process::id()));
    fs::remove_dir_all(&dir).ok();
    fs::create_dir_all(dir.join("hosts")).unwrap();
    fs::create_dir_all(dir.join("layers/only")).unwrap();
    fs::write(
        dir.join("layers/only/layer.kdl"),
        "description \"fixture\"\n\
         params {\n    virt \"run virtual machines here\" default=off\n}\n\
         when virt=on {\n    groups { libvirt }\n}\n",
    )
    .unwrap();
    for (host, body) in [("off", ""), ("on", "virt on\n")] {
        fs::write(
            dir.join(format!("hosts/{host}.kdl")),
            format!("description \"fixture\"\nlayers \"only\"\n{body}"),
        )
        .unwrap();
    }

    let run = |h: &str| {
        let out = Command::new(env!("CARGO_BIN_EXE_lami"))
            .args(["--config-dir", dir.to_str().unwrap()])
            .args(["--host", h])
            .args(["show", "--no-color"])
            .output()
            .expect("runs");
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    assert!(run("off").contains("groups     0"), "{}", run("off"));
    assert!(run("on").contains("groups     1"), "{}", run("on"));

    fs::remove_dir_all(&dir).ok();
}
