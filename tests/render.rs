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
    let (frodo, ok) = lami(&[
        "--host",
        "frodo",
        "render",
        "/etc/makepkg.conf.d/99-local.conf",
    ]);
    assert!(ok, "{frodo}");
    assert!(frodo.contains(r#"MAKEFLAGS="-j8""#), "{frodo}");

    let (sam, _) = lami(&[
        "--host",
        "sam",
        "render",
        "/etc/makepkg.conf.d/99-local.conf",
    ]);
    assert!(sam.contains(r#"MAKEFLAGS="-j16""#), "{sam}");
}

#[test]
fn a_conditional_file_follows_the_gpu() {
    let (frodo, _) = lami(&[
        "--host",
        "frodo",
        "render",
        "/etc/environment.d/50-gpu.conf",
    ]);
    assert!(frodo.contains("LIBVA_DRIVER_NAME=iHD"), "{frodo}");

    let (sam, _) = lami(&["--host", "sam", "render", "/etc/environment.d/50-gpu.conf"]);
    assert!(sam.contains("LIBVA_DRIVER_NAME=nvidia"), "{sam}");
}

#[test]
fn multiline_text_is_dedented() {
    // KDL v2 strips the indentation used to keep the config readable, so it
    // must not leak into the rendered file.
    let (out, _) = lami(&[
        "--host",
        "frodo",
        "render",
        "/etc/makepkg.conf.d/99-local.conf",
    ]);
    assert!(
        out.starts_with("#!/hint/bash"),
        "leading indent leaked:\n{out:?}"
    );
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

    let (out, ok) = lami(&["--host", "frodo", "render", "--out", dir.to_str().unwrap()]);
    assert!(ok, "{out}");

    // The tree mirrors absolute paths so it can be diffed against the live
    // system directly.
    assert!(dir.join("etc/pacman.conf").is_file(), "{out}");
    assert!(
        dir.join("etc/makepkg.conf.d/99-local.conf").is_file(),
        "{out}"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn an_unmanaged_path_is_an_error() {
    let (out, ok) = lami(&["--host", "frodo", "render", "/etc/not-managed"]);
    assert!(!ok, "must exit with an error:\n{out}");
    assert!(
        out.contains("--list"),
        "should point at how to see the paths:\n{out}"
    );
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
    std::fs::write(
        dir.join("hosts/h.kdl"),
        "description \"x\"\nlayers \"only\"\n",
    )
    .unwrap();
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

#[test]
fn piped_output_carries_no_escape_codes() {
    // Colour is meaning here, not decoration -- but a pipe or a file must get
    // clean text. Capturing output like this is exactly the non-terminal case,
    // so nothing may be coloured.
    let exe = env!("CARGO_BIN_EXE_lami");
    let out = std::process::Command::new(exe)
        .args([
            "--config-dir",
            "examples/minimal",
            "--host",
            "frodo",
            "show",
        ])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        !text.contains('\x1b'),
        "escape codes leaked into non-terminal output:\n{text:?}"
    );
}

#[test]
fn no_color_is_accepted_everywhere() {
    // --no-color is a global flag, so it has to work after the subcommand too.
    let exe = env!("CARGO_BIN_EXE_lami");
    for args in [vec!["--no-color", "list"], vec!["list", "--no-color"]] {
        let out = std::process::Command::new(exe)
            .args(["--config-dir", "examples/minimal"])
            .args(&args)
            .output()
            .unwrap();
        assert!(out.status.success(), "{args:?} failed");
    }
}

#[test]
fn a_single_file_renders_by_either_spelling() {
    // The config writes a home path as `~/...`; the shell hands lami the
    // expanded one. Tab completion produces the second, so it has to work.
    let home = std::env::var("HOME").unwrap_or_default();

    let (tilde, ok) = lami(&["--host", "sam", "render", "~/.local/bin/hello"]);
    assert!(ok, "{tilde}");

    let (abs, ok) = lami(&[
        "--host",
        "sam",
        "render",
        &format!("{home}/.local/bin/hello"),
    ]);
    assert!(ok, "{abs}");
    assert_eq!(tilde, abs, "same file, same output");

    // Raw on stdout, so it can be piped straight into a diff -- no header,
    // no path, nothing to strip.
    assert!(!tilde.contains("---"), "{tilde}");
}

#[test]
fn why_answers_whether_a_file_is_managed() {
    // "Is this tracked already?" has to work with the path you can see on
    // disk. The config writes a home path as `~/...`, but the shell expands
    // the tilde long before lami sees it and tab completion gives the
    // absolute form.
    let home = std::env::var("HOME").unwrap_or_default();

    let (out, ok) = lami(&["--host", "sam", "why", "~/.local/bin/hello"]);
    assert!(ok, "{out}");
    assert!(out.contains("(file)"), "{out}");

    let (out, ok) = lami(&["--host", "sam", "why", &format!("{home}/.local/bin/hello")]);
    assert!(ok, "{out}");
    assert!(
        out.contains("(file)"),
        "the absolute form finds it too:\n{out}"
    );

    // And the answer for one that is not managed says so plainly, rather than
    // leaving you to infer it from a declaration that did not appear.
    let (out, ok) = lami(&["--host", "sam", "why", "/etc/fstab"]);
    assert!(ok, "{out}");
    assert!(out.contains("not managed by lami"), "{out}");
    assert!(
        out.contains("render --list"),
        "points somewhere useful:\n{out}"
    );
}

#[test]
fn a_single_layer_can_be_rendered_on_its_own() {
    // "What does the rice layer actually write?" without reading the layer
    // file and following every from= by hand.
    let (all, ok) = lami(&["--host", "frodo", "render", "--list"]);
    assert!(ok, "{all}");
    let (one, ok) = lami(&["--host", "frodo", "render", "--layer", "gui", "--list"]);
    assert!(ok, "{one}");

    assert!(one.lines().count() < all.lines().count(), "{one}");
    assert!(
        one.lines().all(|l| l.contains("[gui]")),
        "only that layer's files:\n{one}"
    );
}

#[test]
fn a_layer_this_host_does_not_enable_is_an_error() {
    // Checked against the host's own layers, not the whole repo: asking about
    // a layer this machine does not enable is a question with an answer, and
    // an empty list is not it.
    let (out, ok) = lami(&["--host", "sam", "render", "--layer", "rice", "--list"]);
    assert!(!ok, "sam does not enable rice:\n{out}");
    assert!(
        out.contains("does not enable a layer called 'rice'"),
        "{out}"
    );
    assert!(out.contains("layers on this host"), "{out}");
}

#[test]
fn a_file_from_another_layer_says_which() {
    // "Not managed" and "managed, but not by that layer" are different
    // answers, and only one of them is true here.
    let (out, ok) = lami(&[
        "--host",
        "frodo",
        "render",
        "--layer",
        "gui",
        "/etc/pacman.conf",
    ]);
    assert!(!ok, "{out}");
    assert!(out.contains("but by layer 'core'"), "{out}");
}

#[test]
fn a_file_in_a_managed_directory_that_the_layer_lacks_is_reported() {
    // A `dir` says the directory belongs to a layer. Something that appears
    // in it and not in the layer is the one case where "what have I changed
    // that lami does not know about?" has a bounded answer -- everywhere else
    // the question would mean walking the whole filesystem.
    use std::fs;
    let dir = std::env::temp_dir().join(format!("lami-untracked-{}", std::process::id()));
    fs::remove_dir_all(&dir).ok();
    let target = dir.join("live");
    fs::create_dir_all(dir.join("cfg/hosts")).unwrap();
    fs::create_dir_all(dir.join("cfg/layers/only/files")).unwrap();
    fs::create_dir_all(&target).unwrap();

    fs::write(dir.join("cfg/layers/only/files/kept.conf"), "same\n").unwrap();
    fs::write(target.join("kept.conf"), "same\n").unwrap();
    fs::write(target.join("stranger.conf"), "nobody declared me\n").unwrap();

    fs::write(
        dir.join("cfg/hosts/testbox.kdl"),
        "description \"fixture\"\nlayers \"only\"\n",
    )
    .unwrap();
    fs::write(
        dir.join("cfg/layers/only/layer.kdl"),
        format!(
            "description \"fixture\"\n\ndir \"{}\" from=\"files\"\n",
            target.display()
        ),
    )
    .unwrap();

    let out = Command::new(env!("CARGO_BIN_EXE_lami"))
        .args(["--config-dir", dir.join("cfg").to_str().unwrap()])
        .args(["--host", "testbox"])
        .args(["diff", "--undeclared", "--no-color"])
        .output()
        .expect("lami runs");
    let text = String::from_utf8_lossy(&out.stdout).into_owned();

    assert!(text.contains("stranger.conf"), "{text}");
    assert!(
        !text.contains("kept.conf"),
        "the declared one is not:\n{text}"
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn patch_shows_what_differs_inside_a_file() {
    use std::fs;
    let dir = std::env::temp_dir().join(format!("lami-patch-{}", std::process::id()));
    fs::remove_dir_all(&dir).ok();
    fs::create_dir_all(dir.join("cfg/hosts")).unwrap();
    fs::create_dir_all(dir.join("cfg/layers/only")).unwrap();
    let target = dir.join("target.conf");
    fs::write(&target, "first\nSECOND\nthird\n").unwrap();
    fs::write(
        dir.join("cfg/hosts/testbox.kdl"),
        "description \"fixture\"\nlayers \"only\"\n",
    )
    .unwrap();
    fs::write(
        dir.join("cfg/layers/only/layer.kdl"),
        format!(
            "description \"fixture\"\n\nfile \"{}\" {{\n    text \"\"\"\n    first\n    second\n    third\n    \"\"\"\n}}\n",
            target.display()
        ),
    )
    .unwrap();

    let run = |args: &[&str]| {
        let out = Command::new(env!("CARGO_BIN_EXE_lami"))
            .args(["--config-dir", dir.join("cfg").to_str().unwrap()])
            .args(["--host", "testbox"])
            .arg("--no-color")
            .args(args)
            .output()
            .expect("lami runs");
        String::from_utf8_lossy(&out.stdout).into_owned()
    };

    // Without --patch the summary says only that it differs.
    let plain = run(&["diff"]);
    assert!(plain.contains("content differs"), "{plain}");
    assert!(!plain.contains("-SECOND"), "{plain}");

    let patched = run(&["diff", "--patch"]);
    assert!(patched.contains("-second"), "{patched}");
    assert!(patched.contains("+SECOND"), "{patched}");
    assert!(patched.contains("@@"), "a unified diff:\n{patched}");

    fs::remove_dir_all(&dir).ok();
}
