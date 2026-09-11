//! The contract between a host file and the layers it enables.
//!
//! Before `params`, the two sides could only agree by convention: a host could
//! set something nothing read, a layer could test something nothing set, and
//! both stayed quiet. These tests are about making each of those loud.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

struct Fixture {
    dir: PathBuf,
}

impl Fixture {
    /// A config with one layer and one host, both written by the caller.
    fn new(tag: &str, layer: &str, host: &str) -> Fixture {
        let dir = std::env::temp_dir().join(format!("lami-params-{}-{tag}", std::process::id()));
        fs::remove_dir_all(&dir).ok();
        fs::create_dir_all(dir.join("hosts")).unwrap();
        fs::create_dir_all(dir.join("layers/only")).unwrap();
        fs::write(
            dir.join("layers/only/layer.kdl"),
            format!("description \"fixture\"\n\n{layer}"),
        )
        .unwrap();
        fs::write(
            dir.join("hosts/testbox.kdl"),
            format!("description \"fixture\"\nlayers \"only\"\n{host}"),
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

const GPU: &str = "params {\n    gpu \"which driver\" one-of=\"intel amd\"\n}\n";

#[test]
fn a_required_parameter_the_host_omits_is_an_error() {
    // No default profile, here as everywhere else: a machine that does not say
    // which GPU it has should stop, not install the wrong driver.
    let f = Fixture::new("missing", GPU, "");
    let (out, ok) = f.run(&["show"]);
    assert!(!ok, "{out}");
    assert!(out.contains("does not set 'gpu'"), "{out}");
    assert!(
        out.contains("which driver"),
        "the layer's own words:\n{out}"
    );
}

#[test]
fn a_value_outside_one_of_is_an_error() {
    let f = Fixture::new("typo", GPU, "gpu \"intle\"\n");
    let (out, ok) = f.run(&["show"]);
    assert!(!ok, "{out}");
    assert!(out.contains("not one of its values"), "{out}");
    assert!(out.contains("intel, amd"), "{out}");
}

#[test]
fn a_default_fills_in_for_a_line_the_host_leaves_out() {
    let f = Fixture::new(
        "default",
        "params {\n    ddc \"brightness over DDC/CI\" default=off\n}\n\
         when ddc=on {\n    packages { ddcutil }\n}\n",
        "",
    );
    let (out, ok) = f.run(&["show"]);
    assert!(ok, "{out}");
    assert!(out.contains("off"), "{out}");
    assert!(
        out.contains("default"),
        "it says the value came from one:\n{out}"
    );
}

#[test]
fn a_default_can_be_overridden() {
    let f = Fixture::new(
        "override",
        "params {\n    ddc \"brightness over DDC/CI\" default=off\n}\n\
         when ddc=on {\n    packages { ddcutil }\n}\n",
        "ddc on\n",
    );
    let (out, ok) = f.run(&["why", "ddcutil"]);
    assert!(ok, "{out}");
    assert!(out.contains("ddc=on"), "{out}");
}

#[test]
fn a_when_on_an_undeclared_parameter_is_an_error() {
    // This is the whole reason the block exists. `when hostname="frodo"` and
    // `when gpu="intle"` used to do exactly what a correct condition that
    // happens not to hold does: nothing, silently.
    let f = Fixture::new(
        "unknown",
        "when nosuchthing=\"x\" {\n    packages { ghost }\n}\n",
        "",
    );
    let (out, ok) = f.run(&["show"]);
    assert!(!ok, "{out}");
    assert!(
        out.contains("no layer declares a parameter called 'nosuchthing'"),
        "{out}"
    );
}

#[test]
fn hostname_is_always_available_to_when() {
    let f = Fixture::new(
        "hostname",
        "when hostname=\"testbox\" {\n    packages { only-here }\n}\n",
        "",
    );
    let (out, ok) = f.run(&["why", "only-here"]);
    assert!(ok, "{out}");
    assert!(out.contains("hostname=testbox"), "{out}");
}

#[test]
fn hostname_cannot_be_set_by_hand() {
    // The file's name is the truth. If a host file could disagree with it,
    // /etc/hostname and the profile it was resolved from could differ.
    let f = Fixture::new("sethostname", GPU, "gpu \"intel\"\nhostname \"other\"\n");
    let (out, ok) = f.run(&["show"]);
    assert!(!ok, "{out}");
    assert!(out.contains("cannot be set"), "{out}");
}

#[test]
fn a_parameter_nothing_reads_is_reported_but_not_fatal() {
    // The opposite of a missing one, and deliberately not an error: a template
    // may read it, or a layer this host does not enable may declare it.
    let f = Fixture::new(
        "unread",
        GPU,
        "gpu \"intel\"\nmonitors {\n    \"HDMI-A-1\"\n}\n",
    );
    let (out, ok) = f.run(&["check"]);
    assert!(ok, "{out}");
    assert!(out.contains("parameters nothing reads"), "{out}");
    assert!(out.contains("monitors"), "{out}");
}

#[test]
fn a_list_parameter_written_as_a_string_is_refused() {
    // KDL cannot tell one string from a list of one, so a template looping
    // over the scalar form silently produces nothing.
    let f = Fixture::new(
        "scalarlist",
        "params {\n    monitors \"one line per output\" list=#true\n}\n",
        "monitors \"HDMI-A-1, preferred, 0x0, 1\"\n",
    );
    let (out, ok) = f.run(&["show"]);
    assert!(!ok, "{out}");
    assert!(out.contains("written as a block"), "{out}");
}

#[test]
fn a_parameter_without_a_description_is_refused() {
    let f = Fixture::new("nodesc", "params {\n    gpu one-of=\"intel amd\"\n}\n", "");
    let (out, ok) = f.run(&["show"]);
    assert!(!ok, "{out}");
    assert!(out.contains("no description"), "{out}");
}

#[test]
fn a_default_outside_its_own_allowed_values_is_refused() {
    // Otherwise the mistake only surfaces on the one host that omits the line.
    let f = Fixture::new(
        "baddefault",
        "params {\n    gpu \"which driver\" one-of=\"intel amd\" default=\"nvidia\"\n}\n",
        "",
    );
    let (out, ok) = f.run(&["show"]);
    assert!(!ok, "{out}");
    assert!(out.contains("not one of the allowed values"), "{out}");
}

#[test]
fn init_writes_what_the_layers_ask_for() {
    // The point of `init` is not the file but the question: the layers are
    // asked what they need to know, so a new host arrives already listing it.
    let dir = std::env::temp_dir().join(format!("lami-init-{}", std::process::id()));
    fs::remove_dir_all(&dir).ok();
    fs::create_dir_all(dir.join("hosts")).unwrap();
    fs::create_dir_all(dir.join("layers/base")).unwrap();
    fs::create_dir_all(dir.join("layers/desk")).unwrap();
    fs::write(
        dir.join("layers/base/layer.kdl"),
        "description \"base\"\nparams {\n    ucode \"CPU microcode\" one-of=\"intel amd none\"\n}\n",
    )
    .unwrap();
    fs::write(
        dir.join("layers/desk/layer.kdl"),
        "description \"desk\"\nneeds \"base\"\n\
         params {\n    gpu \"which driver\" one-of=\"intel amd\"\n\
         \x20   ddc \"brightness over DDC/CI\" default=off\n}\n",
    )
    .unwrap();

    let run = |args: &[&str]| {
        let out = Command::new(env!("CARGO_BIN_EXE_lami"))
            .args(["--config-dir", dir.to_str().unwrap()])
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
    };

    // Only `desk` is named; `base` comes along through `needs`, and so does
    // the parameter it declares.
    let (out, ok) = run(&["init", "newbox", "--layers", "desk"]);
    assert!(ok, "{out}");
    assert!(
        out.contains("ucode"),
        "a needed layer's parameter too:\n{out}"
    );
    assert!(out.contains("gpu"), "{out}");

    let written = fs::read_to_string(dir.join("hosts/newbox.kdl")).unwrap();
    assert!(written.contains("layers \"desk\""), "{written}");
    assert!(written.contains("// CPU microcode (base)"), "{written}");
    assert!(written.contains("// one of: intel amd none"), "{written}");
    assert!(
        written.contains("ucode \"\""),
        "required, left blank:\n{written}"
    );
    // Optional ones are written commented out, showing the default.
    assert!(written.contains("// ddc off"), "{written}");

    // It refuses to overwrite.
    let (out, ok) = run(&["init", "newbox", "--layers", "desk"]);
    assert!(!ok, "{out}");
    assert!(out.contains("already exists"), "{out}");

    // An unknown layer says which ones exist.
    let (out, ok) = run(&["init", "other", "--layers", "nosuch"]);
    assert!(!ok, "{out}");
    assert!(out.contains("base, desk"), "{out}");

    fs::remove_dir_all(&dir).ok();
}
