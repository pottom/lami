//! A `lami check` tesztjei.
//!
//! Ez a parancs potolja azt, amit a kulon `aur` blokk megszunesevel
//! elvesztettunk: mivel a config nem mondja meg, mi jon az AUR-bol,
//! ELLENORIZNI kell, hogy van-e AUR helper.

use std::process::Command;

fn pacman_van() -> bool {
    Command::new("pacman")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn lami_path(path: Option<&str>, args: &[&str]) -> (String, bool) {
    let exe = env!("CARGO_BIN_EXE_lami");
    let mut cmd = Command::new(exe);
    cmd.args(["--config-dir", "examples/minimal"]).args(args);
    if let Some(p) = path {
        cmd.env("PATH", p);
    }
    let out = cmd.output().expect("a lami futtathato");
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
fn a_repobol_jovo_csomagokat_felismeri() {
    if !pacman_van() {
        eprintln!("nincs pacman, kihagyva");
        return;
    }
    // sam-nek nincs rice retege, tehat minden csomagja a repobol jon.
    let (out, ok) = lami_path(None, &["--host", "sam", "check"]);
    assert!(ok, "{out}");
    assert!(out.contains("nem a repóból 0"), "{out}");
}

#[test]
fn az_aur_csomagokat_elkuloniti() {
    if !pacman_van() {
        eprintln!("nincs pacman, kihagyva");
        return;
    }
    let (out, ok) = lami_path(None, &["--host", "frodo", "check"]);
    assert!(ok, "van AUR helper a gepen, tehat sikerulnie kell:\n{out}");
    assert!(out.contains("caelestia-shell"), "{out}");
}

#[test]
fn aur_helper_nelkul_hibaval_all_meg() {
    if !pacman_van() {
        eprintln!("nincs pacman, kihagyva");
        return;
    }
    // Csak a pacman legyen elerheto, AUR helper ne.
    let tmp = std::env::temp_dir().join(format!("lami-test-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    let link = tmp.join("pacman");
    let _ = std::fs::remove_file(&link);
    std::os::unix::fs::symlink("/usr/bin/pacman", &link).unwrap();

    let (out, ok) = lami_path(Some(tmp.to_str().unwrap()), &["--host", "frodo", "check"]);
    std::fs::remove_dir_all(&tmp).ok();

    assert!(!ok, "hibaval kell leallnia:\n{out}");
    assert!(out.contains("NINCS AUR helper"), "{out}");
    // A hibauzenet legyen cselekvesre keszteto, ne csak panasz.
    assert!(out.contains("makepkg -si"), "adjon megoldast is:\n{out}");
    assert!(out.contains("lami why"), "mutassa meg, hol javithato:\n{out}");
}
