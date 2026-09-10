//! Tests for `lami render`.
//!
//! Rendering exists before `apply` on purpose: seeing exactly what would be
//! written, before anything is written, is the cheapest way to catch a
//! template mistake.

use std::process::Command;

fn lami(args: &[&str]) -> (String, bool) {
    let exe = env!("CARGO_BIN_EXE_lami");
    let out = Command::new(exe)
        .args(["--config-dir", "examples/minimal"])
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

#[test]
fn lists_the_managed_paths() {
    let (out, ok) = lami(&["--host", "frodo", "render", "--list"]);
    assert!(ok, "{out}");
    assert!(out.contains("/etc/pacman.conf"), "{out}");
    assert!(out.contains("/etc/hostname"), "{out}");
}

#[test]
fn substitutes_host_parameters() {
    // cpu_threads is 8 on frodo and 16 on sam.
    let (frodo, ok) = lami(&["--host", "frodo", "render", "/etc/makepkg.conf.d/99-local.conf"]);
    assert!(ok, "{frodo}");
    assert!(frodo.contains(r#"MAKEFLAGS="-j8""#), "{frodo}");

    let (sam, _) = lami(&["--host", "sam", "render", "/etc/makepkg.conf.d/99-local.conf"]);
    assert!(sam.contains(r#"MAKEFLAGS="-j16""#), "{sam}");
}

#[test]
fn a_conditional_file_follows_the_gpu() {
    let (frodo, _) = lami(&["--host", "frodo", "render", "/etc/environment.d/50-gpu.conf"]);
    assert!(frodo.contains("LIBVA_DRIVER_NAME=iHD"), "{frodo}");

    let (sam, _) = lami(&["--host", "sam", "render", "/etc/environment.d/50-gpu.conf"]);
    assert!(sam.contains("LIBVA_DRIVER_NAME=nvidia"), "{sam}");
}

#[test]
fn multiline_text_is_dedented() {
    // KDL v2 strips the indentation used to keep the config readable, so it
    // must not leak into the rendered file.
    let (out, _) = lami(&["--host", "frodo", "render", "/etc/makepkg.conf.d/99-local.conf"]);
    assert!(out.starts_with("#!/hint/bash"), "leading indent leaked:\n{out:?}");
}

#[test]
fn rendered_files_end_with_a_newline() {
    // /etc/hostname from a single-line `text` had no trailing newline, which
    // would differ from what every other tool writes and show up as a
    // permanent spurious diff.
    let (out, _) = lami(&["--host", "frodo", "render", "/etc/hostname"]);
    assert_eq!(out, "frodo\n", "{out:?}");
}

#[test]
fn writes_a_tree_that_mirrors_the_target_paths() {
    let dir = std::env::temp_dir().join(format!("lami-render-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();

    let (out, ok) = lami(&[
        "--host",
        "frodo",
        "render",
        "--out",
        dir.to_str().unwrap(),
    ]);
    assert!(ok, "{out}");

    // The tree mirrors absolute paths so it can be diffed against the live
    // system directly.
    assert!(dir.join("etc/pacman.conf").is_file(), "{out}");
    assert!(dir.join("etc/makepkg.conf.d/99-local.conf").is_file(), "{out}");

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn an_unmanaged_path_is_an_error() {
    let (out, ok) = lami(&["--host", "frodo", "render", "/etc/not-managed"]);
    assert!(!ok, "must exit with an error:\n{out}");
    assert!(out.contains("--list"), "should point at how to see the paths:\n{out}");
}

#[test]
fn trailing_blank_lines_are_preserved() {
    // Only a MISSING newline is added; existing ones are never removed.
    // /etc/conf.d/snapper as shipped by the snapper package ends with a blank
    // line, and an earlier version of this normalisation silently ate it --
    // which would rewrite the user's file to satisfy our own tidiness.
    let dir = std::env::temp_dir().join(format!("lami-nl-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(dir.join("hosts")).unwrap();
    std::fs::create_dir_all(dir.join("layers/only/files")).unwrap();

    std::fs::write(dir.join("layers/only/files/keep"), "line\n\n").unwrap();
    std::fs::write(dir.join("hosts/h.kdl"), "description \"x\"\nlayers \"only\"\n").unwrap();
    std::fs::write(
        dir.join("layers/only/layer.kdl"),
        "description \"x\"\nfile \"/tmp/lami-nl-target\" from=\"files/keep\"\n",
    )
    .unwrap();

    let exe = env!("CARGO_BIN_EXE_lami");
    let out = std::process::Command::new(exe)
        .args(["--config-dir", dir.to_str().unwrap()])
        .args(["--host", "h", "render", "/tmp/lami-nl-target"])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert_eq!(text, "line\n\n", "the blank line must survive: {text:?}");

    std::fs::remove_dir_all(&dir).ok();
}
