#![allow(dead_code)] // a config modell reszei a kovetkezo merfoldkohoz kellenek

mod cli;
mod config;
mod error;
mod pacman;

use std::path::PathBuf;

use clap::Parser;
use miette::Result;

use crate::cli::{Cli, Command};
use crate::config::Config;
use crate::error::Error;

/// A config könyvtára: `--config-dir`, `LAMI_CONFIG_DIR`, végül `$XDG_CONFIG_HOME/lami`.
///
/// A home-ot NEM a `$HOME`-ból vesszük: sudo alatt az a hívó home-ja lehet
/// (`env_keep` / `always_set_home`), és a tool rootként fog futni.
fn config_dir(explicit: Option<PathBuf>) -> Result<PathBuf, Error> {
    if let Some(p) = explicit {
        return Ok(p);
    }
    if let Ok(x) = std::env::var("XDG_CONFIG_HOME") {
        if !x.is_empty() {
            return Ok(PathBuf::from(x).join("lami"));
        }
    }
    let home = real_home()?;
    Ok(home.join(".config").join("lami"))
}

/// A tényleges felhasználó home-ja a passwd-ből.
fn real_home() -> Result<PathBuf, Error> {
    // sudo alatt a SUDO_UID az eredeti felhasználó; egyébként a sajátunk.
    let uid: u32 = std::env::var("SUDO_UID")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| unsafe { libc::getuid() });

    let pw = unsafe { libc::getpwuid(uid) };
    if pw.is_null() {
        return Err(Error::Other(format!(
            "nem találom a(z) {uid} uid-hoz tartozó felhasználót a passwd-ben"
        )));
    }
    let dir = unsafe { std::ffi::CStr::from_ptr((*pw).pw_dir) };
    Ok(PathBuf::from(
        dir.to_str()
            .map_err(|_| Error::Other("a home útvonala nem érvényes UTF-8".into()))?,
    ))
}

fn hostname() -> String {
    std::fs::read_to_string("/etc/hostname")
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let dir = config_dir(cli.config_dir.clone())?;

    if !dir.is_dir() {
        return Err(Error::Other(format!(
            "a config könyvtár nem létezik: {}\n\
             Hozd létre, vagy add meg: lami --config-dir <út>",
            dir.display()
        ))
        .into());
    }

    let cfg = Config::load(&dir)?;

    match cli.command {
        Command::List => cmd_list(&cfg),
        Command::Show => cmd_show(&cfg, cli.host.unwrap_or_else(hostname))?,
        Command::Why { target } => cmd_why(&cfg, cli.host.unwrap_or_else(hostname), &target)?,
        Command::Check => cmd_check(&cfg, cli.host.unwrap_or_else(hostname))?,
    }
    Ok(())
}

fn cmd_list(cfg: &Config) {
    println!("config: {}\n", cfg.dir.display());

    println!("gépek:");
    if cfg.hosts.is_empty() {
        println!("  (egy sincs)");
    }
    for h in cfg.hosts.values() {
        let desc = h.description.as_deref().unwrap_or("");
        println!("  {:<12} {}", h.name, desc);
    }

    println!("\nrétegek:");
    if cfg.layers.is_empty() {
        println!("  (egy sincs)");
    }
    for l in cfg.layers.values() {
        let desc = l.description.as_deref().unwrap_or("");
        println!("  {:<12} {}", l.name, desc);
    }
}

fn cmd_show(cfg: &Config, host: String) -> Result<(), Error> {
    let r = cfg.resolve(&host)?;

    println!("gép: {}", r.host.name);
    if let Some(d) = &r.host.description {
        println!("     {d}");
    }
    println!("     {}", r.host.origin.display());

    println!("\nparaméterek:");
    for (k, v) in &r.host.params {
        println!("  {k:<14} {v}");
    }

    println!("\nrétegek (feloldva, függőségi sorrendben):");
    for l in &r.layers {
        let explicit = if r.host.layers.contains(&l.name) {
            ""
        } else {
            "  (függőségként)"
        };
        println!("  {:<12} {}{}", l.name, l.path.display(), explicit);
    }

    println!("\nerőforrások erre a gépre:");
    println!("  csomag         {}", r.packages().len());
    println!("  szolgáltatás   {}", r.services().len());
    Ok(())
}

fn cmd_why(cfg: &Config, host: String, target: &str) -> Result<(), Error> {
    let r = cfg.resolve(&host)?;

    let mut found = false;
    for (kind, items) in [
        ("csomag", r.packages()),
        ("szolgáltatás", r.services()),
    ] {
        for (layer, decl) in items {
            if decl.name != target {
                continue;
            }
            found = true;
            println!("{}  ({kind})", decl.name);
            println!("  deklarálva:  {}", decl.origin);
            print!("  azért kapod: a(z) '{}' réteg", layer.name);
            if r.host.layers.contains(&layer.name) {
                println!(" szerepel {} layers listájában", r.host.name);
            } else {
                println!(" a függőségi láncban van");
            }
            if let Some(c) = &decl.condition {
                let val = r.host.param(&c.key).map(|v| v.to_string()).unwrap_or_default();
                println!("  feltétel:    {} (a gépen: {} = {})", c, c.key, val);
            }
            println!();
        }
    }

    if !found {
        println!("'{target}' nincs deklarálva {} számára.", r.host.name);
        println!("\nEllenőrizd a `lami show` kimenetét, vagy lehet, hogy más gép kapja csak.");
    }
    Ok(())
}

/// A config ellenőrzése a rendszer ellen.
///
/// Ez pótolja azt, amit a külön `aur` blokk megszűnésével elvesztettünk: mivel
/// a config nem mondja meg, mi jön az AUR-ból, ELLENŐRIZNI kell, hogy van-e
/// AUR helper -- különben az apply csak a telepítésnél derítené ki.
fn cmd_check(cfg: &Config, host: String) -> Result<(), Error> {
    let r = cfg.resolve(&host)?;
    println!("gép: {}\n", r.host.name);

    if !pacman::available() {
        println!("A pacman nem elérhető, a csomagellenőrzés kimarad.");
        println!("(A lami Arch Linuxra készült; a config szerkezete így is ellenőrizve.)");
        return Ok(());
    }

    let declared: Vec<&str> = r.packages().iter().map(|(_, d)| d.name.as_str()).collect();
    let sync = pacman::sync_packages()?;

    let (from_repo, unknown): (Vec<&str>, Vec<&str>) =
        declared.iter().partition(|p| sync.contains(**p));

    println!("csomagok:");
    println!("  repóból      {}", from_repo.len());
    println!("  nem a repóból {}", unknown.len());

    if unknown.is_empty() {
        println!("\n✓ minden csomag elérhető a beállított repókból.");
        return Ok(());
    }

    println!();
    match pacman::aur_helper() {
        Some(helper) => {
            println!("Ezeket a(z) '{helper}' hozza az AUR-ból:");
            for p in &unknown {
                println!("  {p}");
            }
            println!(
                "\nMegjegyzés: ami elgépelés, az is ide kerül -- a lami nem kérdezi le\n\
                 az AUR-t hálózat nélkül. A telepítéskor a(z) {helper} fog szólni."
            );
        }
        None => {
            let list = unknown.join(", ");
            return Err(Error::Other(format!(
                "{} csomag nem érhető el a beállított repókból, és NINCS AUR helper telepítve:\n\
                 \n  {}\n\
                 \nHa AUR-os csomagok: telepíts egy helpert (paru, yay, pikaur, aura).\n\
                 A forrásból épített paru ajánlott -- a paru-bin a pacman ABI-váltásakor\n\
                 használhatatlanná válik, épp amikor javítanál vele:\n\
                 \n  git clone https://aur.archlinux.org/paru.git && cd paru && makepkg -si\n\
                 \nHa elgépelés: javítsd a rétegfájlban. A `lami why <nev>` megmondja, hol van.",
                unknown.len(),
                list
            )));
        }
    }
    Ok(())
}
