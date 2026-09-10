//! A gépfeloldás integrációs tesztjei az examples/minimal configon.
//!
//! A fixture szándékosan NEM a szerző személyes configja: stabilnak és
//! hardverfüggetlennek kell lennie, különben a tesztek egy külső
//! hozzájárulónál elhasalnak.

use std::path::Path;
use std::process::Command;

fn lami(args: &[&str]) -> (String, bool) {
    let exe = env!("CARGO_BIN_EXE_lami");
    let out = Command::new(exe)
        .args(["--config-dir", "examples/minimal"])
        .args(args)
        .output()
        .expect("a lami futtatható");
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
fn a_fixture_letezik() {
    assert!(Path::new("examples/minimal/hosts/frodo.kdl").is_file());
}

#[test]
fn a_needs_fuggosegek_feloldodnak() {
    // A rice needs gui, a gui needs core -- a sam layers listajaban
    // viszont csak core, tools, gui van.
    let (out, ok) = lami(&["--host", "sam", "show"]);
    assert!(ok, "{out}");
    assert!(out.contains("core"), "{out}");
    assert!(out.contains("gui"), "{out}");
    assert!(!out.contains("rice"), "sam nem kaphat rice reteget:\n{out}");
}

#[test]
fn a_retegkapu_mukodik() {
    // Ez a teszt letezesenek oka: a korabbi chezmoi-alapu setupban a
    // rice-fajlok minden gepre felkerultek, mert a chezmoinak nincs
    // retegfogalma. Itt nem szabad, hogy megtortenjen.
    let (sam, _) = lami(&["--host", "sam", "why", "caelestia-shell"]);
    assert!(sam.contains("nincs deklaralva") || sam.contains("nincs deklarálva"), "{sam}");

    let (frodo, _) = lami(&["--host", "frodo", "why", "caelestia-shell"]);
    assert!(frodo.contains("rice"), "frodo megkapja a rice-t:\n{frodo}");
}

#[test]
fn a_gpu_feltetel_szetvalasztja_a_gepeket() {
    let (frodo, _) = lami(&["--host", "frodo", "why", "intel-media-driver"]);
    assert!(frodo.contains("gpu=intel"), "{frodo}");

    let (sam, _) = lami(&["--host", "sam", "why", "intel-media-driver"]);
    assert!(sam.contains("nincs dekl"), "sam NVIDIA-s, nem kaphat iHD-t:\n{sam}");

    let (sam_nv, _) = lami(&["--host", "sam", "why", "nvidia-open"]);
    assert!(sam_nv.contains("gpu=nvidia"), "{sam_nv}");
}

#[test]
fn az_on_off_kapcsolo_mukodik() {
    // A KDL v2-ben a puszta `true` mar nem szabad szo (`#true` kell), a
    // `ddc #true` viszont csunyabb, mint a `ddc on`.
    //
    // Az `off` azert letezik, mert ONMAGAT DOKUMENTALJA: a sor elhagyasa is
    // kikapcsolna, de abbol nem derul ki, hogy merlegelted-e.
    let (frodo, _) = lami(&["--host", "frodo", "why", "ddcutil"]);
    assert!(frodo.contains("ddc=on"), "{frodo}");

    let (sam, _) = lami(&["--host", "sam", "why", "ddcutil"]);
    assert!(sam.contains("nincs dekl"), "sam-en ddc off van:\n{sam}");

    // A show a configgal egyezo alakban irja ki, nem true/false-kent.
    let (show, _) = lami(&["--host", "sam", "show"]);
    assert!(show.contains("ddc            off"), "{show}");
}

#[test]
fn ismeretlen_gep_hibaval_all_meg() {
    // Nincs default profil: egy elgepelt hostname vagy egy friss VM ne
    // kaphasson csendben rossz konfiguraciot.
    let (out, ok) = lami(&["--host", "nemletezik", "show"]);
    assert!(!ok, "hibaval kell leallnia:\n{out}");
    assert!(out.contains("nemletezik"), "{out}");
    assert!(out.contains("frodo"), "sorolja fel az ismert gepeket:\n{out}");
}
