//! Integration tests for host resolution against examples/minimal.
//!
//! The fixture is deliberately NOT the author's personal config: it has to be
//! stable and hardware independent, or the tests would fail for any outside
//! contributor.

use std::path::Path;
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
fn fixture_exists() {
    assert!(Path::new("examples/minimal/hosts/frodo.kdl").is_file());
}

#[test]
fn needs_dependencies_are_resolved() {
    // rice needs gui, gui needs core -- but sam's layers list only has
    // core, tools and gui.
    let (out, ok) = lami(&["--host", "sam", "show"]);
    assert!(ok, "{out}");
    assert!(out.contains("core"), "{out}");
    assert!(out.contains("gui"), "{out}");
    assert!(
        !out.contains("rice"),
        "sam must not receive the rice layer:\n{out}"
    );
}

#[test]
fn layer_gating_works() {
    // Why this test exists: in the previous chezmoi-based setup the rice
    // files landed on every machine, because chezmoi has no concept of
    // layers. That must not happen here.
    let (sam, _) = lami(&["--host", "sam", "why", "caelestia-shell"]);
    assert!(sam.contains("is not declared"), "{sam}");

    let (frodo, _) = lami(&["--host", "frodo", "why", "caelestia-shell"]);
    assert!(frodo.contains("rice"), "frodo does get rice:\n{frodo}");
}

#[test]
fn the_gpu_condition_separates_hosts() {
    let (frodo, _) = lami(&["--host", "frodo", "why", "intel-media-driver"]);
    assert!(frodo.contains("gpu=intel"), "{frodo}");

    let (sam, _) = lami(&["--host", "sam", "why", "intel-media-driver"]);
    assert!(
        sam.contains("is not declared"),
        "sam is NVIDIA, must not get iHD:\n{sam}"
    );

    let (sam_nv, _) = lami(&["--host", "sam", "why", "nvidia-open"]);
    assert!(sam_nv.contains("gpu=nvidia"), "{sam_nv}");
}

#[test]
fn the_on_off_switch_works() {
    // In KDL v2 a bare `true` is no longer a keyword (`#true` is required),
    // but `ddc #true` reads worse than `ddc on`.
    //
    // `off` exists because it DOCUMENTS ITSELF: omitting the line would also
    // disable it, but then you cannot tell whether it was considered.
    let (frodo, _) = lami(&["--host", "frodo", "why", "ddcutil"]);
    assert!(frodo.contains("ddc=on"), "{frodo}");

    let (sam, _) = lami(&["--host", "sam", "why", "ddcutil"]);
    assert!(sam.contains("is not declared"), "sam has ddc off:\n{sam}");

    // `show` prints it the way the config spells it, not as true/false, and
    // names the layer that asked for it.
    let (show, _) = lami(&["--host", "sam", "show"]);
    assert!(show.contains("ddc "), "{show}");
    assert!(show.contains("off"), "{show}");
    assert!(show.contains("gui: external monitor brightness"), "{show}");
}

#[test]
fn an_unknown_host_is_an_error() {
    // There is no default profile: a typo'd hostname or a fresh VM must not
    // silently receive the wrong configuration.
    let (out, ok) = lami(&["--host", "nemletezik", "show"]);
    assert!(!ok, "must exit with an error:\n{out}");
    assert!(out.contains("nemletezik"), "{out}");
    assert!(out.contains("frodo"), "should list the known hosts:\n{out}");
}
