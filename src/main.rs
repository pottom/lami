#![allow(dead_code)] // parts of the config model are for the next milestone

mod apply;
mod capture;
mod cli;
mod config;
mod diff;
mod error;
mod write;
mod pacman;
mod perms;
mod render;
mod systemd;

use std::fs;
use std::path::{Path, PathBuf};

use clap::Parser;
use miette::Result;

use crate::cli::{Cli, Command};
use crate::config::Config;
use crate::error::Error;

/// The config directory: `--config-dir`, `LAMI_CONFIG_DIR`, then
/// `$XDG_CONFIG_HOME/lami`.
///
/// The home directory is NOT taken from `$HOME`: under sudo that may still be
/// the caller's home (`env_keep` / `always_set_home`), and this tool will run
/// as root.
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

/// The real user's home directory, from passwd.
fn real_home() -> Result<PathBuf, Error> {
    // Under sudo, SUDO_UID is the original user; otherwise it is ours.
    let uid: u32 = std::env::var("SUDO_UID")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| unsafe { libc::getuid() });

    let pw = unsafe { libc::getpwuid(uid) };
    if pw.is_null() {
        return Err(Error::Other(format!(
            "no passwd entry for uid {uid}"
        )));
    }
    let dir = unsafe { std::ffi::CStr::from_ptr((*pw).pw_dir) };
    Ok(PathBuf::from(
        dir.to_str()
            .map_err(|_| Error::Other("home directory path is not valid UTF-8".into()))?,
    ))
}

/// The name of the user we are acting for.
///
/// Resolved through passwd, never from $USER: under sudo that would be the
/// caller's name and everything written into a home directory would end up
/// owned by root.
fn real_user() -> Result<String, Error> {
    let uid: u32 = std::env::var("SUDO_UID")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| unsafe { libc::getuid() });
    let pw = unsafe { libc::getpwuid(uid) };
    if pw.is_null() {
        return Err(Error::Other(format!("no passwd entry for uid {uid}")));
    }
    let name = unsafe { std::ffi::CStr::from_ptr((*pw).pw_name) };
    Ok(name.to_string_lossy().into_owned())
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
            "config directory does not exist: {}\n\
             Create it, or point at another one: lami --config-dir <path>",
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
        Command::Apply { dry_run } => {
            cmd_apply(&cfg, cli.host.unwrap_or_else(hostname), dry_run)?
        }
        Command::Capture {
            package,
            layer,
            dry_run,
        } => cmd_capture(
            &cfg,
            cli.host.unwrap_or_else(hostname),
            package.as_deref(),
            layer.as_deref(),
            dry_run,
        )?,
        Command::Diff { undeclared } => {
            cmd_diff(&cfg, cli.host.unwrap_or_else(hostname), undeclared)?
        }
        Command::Render { target, out, list } => cmd_render(
            &cfg,
            cli.host.unwrap_or_else(hostname),
            target.as_deref(),
            out.as_deref(),
            list,
        )?,
    }
    Ok(())
}

fn cmd_list(cfg: &Config) {
    println!("config: {}\n", cfg.dir.display());

    println!("hosts:");
    if cfg.hosts.is_empty() {
        println!("  (none)");
    }
    for h in cfg.hosts.values() {
        let desc = h.description.as_deref().unwrap_or("");
        println!("  {:<12} {}", h.name, desc);
    }

    println!("\nlayers:");
    if cfg.layers.is_empty() {
        println!("  (none)");
    }
    for l in cfg.layers.values() {
        let desc = l.description.as_deref().unwrap_or("");
        println!("  {:<12} {}", l.name, desc);
    }
}

fn cmd_show(cfg: &Config, host: String) -> Result<(), Error> {
    let r = cfg.resolve(&host)?;

    println!("host: {}", r.host.name);
    if let Some(d) = &r.host.description {
        println!("     {d}");
    }
    println!("     {}", r.host.origin.display());

    println!("\nparameters:");
    for (k, v) in &r.host.params {
        println!("  {k:<14} {v}");
    }

    println!("\nlayers (resolved, in dependency order):");
    for l in &r.layers {
        let explicit = if r.host.layers.contains(&l.name) {
            ""
        } else {
            "  (via needs)"
        };
        println!("  {:<12} {}{}", l.name, l.path.display(), explicit);
    }

    println!("\nresources for this host:");
    println!("  packages   {}", r.packages().len());
    println!("  services   {}", r.services().len());
    Ok(())
}

fn cmd_why(cfg: &Config, host: String, target: &str) -> Result<(), Error> {
    let r = cfg.resolve(&host)?;

    let mut found = false;
    for (kind, items) in [
        ("package", r.packages()),
        ("service", r.services()),
    ] {
        for (layer, decl) in items {
            if decl.name != target {
                continue;
            }
            found = true;
            println!("{}  ({kind})", decl.name);
            println!("  declared:   {}", decl.origin);
            print!("  applies:    layer '{}'", layer.name);
            if r.host.layers.contains(&layer.name) {
                println!(" is in {}'s layers list", r.host.name);
            } else {
                println!(" is pulled in via needs");
            }
            if let Some(c) = &decl.condition {
                let val = r.host.param(&c.key).map(|v| v.to_string()).unwrap_or_default();
                println!("  condition:  {} (this host: {} = {})", c, c.key, val);
            }
            println!();
        }
    }

    let user = real_user()?;
    for (layer, f) in r.files() {
        if f.path != target {
            continue;
        }
        found = true;
        let pm = perms::with_overrides(
            perms::infer(&f.path, &user),
            f.owner.as_deref(),
            f.group.as_deref(),
            f.mode,
        );
        println!("{}  (file)", f.path);
        println!("  declared:   {}", f.origin);
        println!("  applies:    layer '{}'", layer.name);
        match &f.source {
            crate::config::Source::From(p) => println!("  content:    {}", p.display()),
            crate::config::Source::Text(_) => println!("  content:    inline in the layer"),
        }
        println!(
            "  permissions: {:o} {}:{}  ({})",
            pm.mode, pm.owner, pm.group, pm.reason
        );
        if let Some(c) = &f.condition {
            println!("  condition:  {c}");
        }
        println!();
    }

    for (layer, h) in r.hooks() {
        if !h.watch.iter().any(|w| w == target) {
            continue;
        }
        found = true;
        println!("{}  (watched by a hook)", target);
        println!("  declared:   {}", h.origin);
        println!("  applies:    layer '{}'", layer.name);
        println!("  runs:       {}", h.run);
        println!();
    }

    if !found {
        println!("'{target}' is not declared for {}.", r.host.name);
        println!("\nCheck `lami show`, or it may only apply to another host.");
    }
    Ok(())
}

/// Check the config against the system.
///
/// This makes up for what we lost when the separate `aur` block went away:
/// since the config no longer states which packages come from the AUR, we have
/// to CHECK that a helper is present -- otherwise apply would only find out at
/// install time.
fn cmd_check(cfg: &Config, host: String) -> Result<(), Error> {
    let r = cfg.resolve(&host)?;
    println!("host: {}\n", r.host.name);

    if !pacman::available() {
        println!("pacman is not available, skipping the package check.");
        println!("(lami targets Arch Linux; the config structure was still validated.)");
        return Ok(());
    }

    let declared: Vec<&str> = r.packages().iter().map(|(_, d)| d.name.as_str()).collect();
    let sync = pacman::sync_packages()?;

    let (from_repo, unknown): (Vec<&str>, Vec<&str>) =
        declared.iter().partition(|p| sync.contains(**p));

    println!("packages:");
    println!("  from repos   {}", from_repo.len());
    println!("  not in repos {}", unknown.len());

    if unknown.is_empty() {
        println!("\nAll packages are available from the configured repositories.");
        return Ok(());
    }

    println!();
    match pacman::aur_helper() {
        Some(helper) => {
            println!("'{helper}' will fetch these from the AUR:");
            for p in &unknown {
                println!("  {p}");
            }
            println!(
                "\nNote: a typo lands in this list too -- lami does not query the AUR\n\
                 without network access. {helper} will tell you at install time."
            );
        }
        None => {
            let list = unknown.join(", ");
            return Err(Error::Other(format!(
                "{} package(s) are not available from the configured repositories,\n\
                 and NO AUR helper is installed:\n\
                 \n  {}\n\
                 \nIf they are AUR packages, install a helper (paru, yay, pikaur, aura).\n\
                 Building paru from source is recommended -- paru-bin breaks on a pacman\n\
                 ABI bump, exactly when you would need it to repair things:\n\
                 \n  git clone https://aur.archlinux.org/paru.git && cd paru && makepkg -si\n\
                 \nIf it is a typo, fix it in the layer file. `lami why <name>` shows where.",
                unknown.len(),
                list
            )));
        }
    }
    Ok(())
}

/// Render managed files: to the screen or into a directory, one or all.
///
/// This is deliberately available before `apply` exists. Seeing exactly what
/// would be written, before anything is written, is the cheapest way to catch
/// a template mistake.
fn cmd_render(
    cfg: &Config,
    host: String,
    target: Option<&str>,
    out: Option<&Path>,
    list_only: bool,
) -> Result<(), Error> {
    let r = cfg.resolve(&host)?;
    let home = real_home()?;

    let mut files = r.files();
    if let Some(t) = target {
        files.retain(|(_, f)| f.path == t);
        if files.is_empty() {
            return Err(Error::Other(format!(
                "'{t}' is not a file managed for {}.\n\
                 Run `lami render --list` to see the managed paths.",
                r.host.name
            )));
        }
    }

    if files.is_empty() {
        println!("No files are managed for {} yet.", r.host.name);
        return Ok(());
    }

    if list_only {
        let user = real_user()?;
        for (layer, f) in &files {
            let pm = perms::with_overrides(
                perms::infer(&f.path, &user),
                f.owner.as_deref(),
                f.group.as_deref(),
                f.mode,
            );
            println!(
                "{:<44} {:o} {}:{}  [{}]  {}",
                f.path, pm.mode, pm.owner, pm.group, layer.name, f.origin
            );
        }
        return Ok(());
    }

    // A single named target prints raw, so it can be piped into a diff.
    let raw = target.is_some() && out.is_none();

    for (layer, f) in &files {
        let content = render::file(&r, layer, f)?;

        match out {
            Some(dir) => {
                // Mirror the absolute path under the output directory, so the
                // tree can be compared with the live system directly.
                let rel = render::target_path(f, &home);
                let rel = rel.strip_prefix("/").unwrap_or(&rel);
                let dest = dir.join(rel);
                if let Some(parent) = dest.parent() {
                    fs::create_dir_all(parent).map_err(|source| Error::Io {
                        path: parent.to_path_buf(),
                        source,
                    })?;
                }
                fs::write(&dest, &content).map_err(|source| Error::Io {
                    path: dest.clone(),
                    source,
                })?;
                println!("{}", dest.display());
            }
            None if raw => print!("{content}"),
            None => {
                println!("\x1b[1m=== {} \x1b[0m", f.path);
                println!("    layer {}, declared at {}", layer.name, f.origin);
                println!();
                for line in content.lines() {
                    println!("    {line}");
                }
                println!();
            }
        }
    }
    Ok(())
}

/// Show what differs between the config and this machine.
fn cmd_diff(cfg: &Config, host: String, show_undeclared: bool) -> Result<(), Error> {
    let r = cfg.resolve(&host)?;
    let home = real_home()?;
    let report = diff::compute(&r, &home, &real_user()?)?;

    println!("host: {}\n", r.host.name);

    if report.changes.is_empty() {
        println!("Nothing to do -- the machine matches the config.");
    } else {
        for c in &report.changes {
            println!("  {}", c.line());
        }
        println!("\n{} change(s). Nothing has been applied.", report.changes.len());
    }

    if !report.skipped.is_empty() {
        println!("\nnot compared:");
        for s in &report.skipped {
            println!("  {s}");
        }
    }

    if show_undeclared {
        println!("\nexplicitly installed but declared by no layer:");
        if report.undeclared_packages.is_empty() {
            println!("  (none)");
        } else {
            for p in &report.undeclared_packages {
                println!("  {p}");
            }
            println!(
                "\n{} package(s). `apply` never removes these -- that is what\n\
                 `prune` is for, and `capture` will offer to file them into a layer.",
                report.undeclared_packages.len()
            );
        }
    }
    Ok(())
}

/// Bring the machine in line with the config.
fn cmd_apply(cfg: &Config, host: String, dry: bool) -> Result<(), Error> {
    if !dry {
        apply::require_root()?;
    }
    let r = cfg.resolve(&host)?;

    let name = real_user()?;
    let home = real_home()?;
    let uid = write::uid_of(&name)
        .ok_or_else(|| Error::Other(format!("no such user: {name}")))?;
    let gid = unsafe {
        let c = std::ffi::CString::new(name.clone()).unwrap();
        let pw = libc::getpwnam(c.as_ptr());
        if pw.is_null() {
            return Err(Error::Other(format!("no passwd entry for {name}")));
        }
        (*pw).pw_gid
    };

    let actor = apply::Actor {
        name,
        uid,
        gid,
        home,
    };

    println!("host: {}\n", r.host.name);
    let n = apply::run_apply(&r, &actor, dry)?;
    if n > 0 && !dry {
        println!("\nApplied {n} change(s).");
    }
    Ok(())
}

/// Pull a change made on this machine back into the repo.
fn cmd_capture(
    cfg: &Config,
    host: String,
    package: Option<&str>,
    layer: Option<&str>,
    dry: bool,
) -> Result<(), Error> {
    let r = cfg.resolve(&host)?;

    let Some(package) = package else {
        // No target named: show what is on offer.
        let report = diff::compute(&r, &real_home()?, &real_user()?)?;
        println!("host: {}\n", r.host.name);

        if report.undeclared_packages.is_empty() {
            println!("Nothing to capture -- every explicitly installed package is declared.");
            return Ok(());
        }
        println!("explicitly installed but declared by no layer:");
        for p in &report.undeclared_packages {
            println!("  {p}");
        }
        println!("\nFile one into a layer with:");
        println!("  lami capture --package <name> --layer <layer>");
        println!("\nlayers on this host: {}", 
            r.layers.iter().map(|l| l.name.as_str()).collect::<Vec<_>>().join(", "));
        return Ok(());
    };

    let Some(layer_name) = layer else {
        return Err(Error::Other(format!(
            "which layer should {package} go in?\n\
             \n  lami capture --package {package} --layer <layer>\n\
             \nlayers on this host: {}",
            r.layers.iter().map(|l| l.name.as_str()).collect::<Vec<_>>().join(", ")
        )));
    };

    let target = cfg.layers.get(layer_name).ok_or_else(|| {
        Error::Other(format!(
            "no layer named '{layer_name}'.\nknown layers: {}",
            cfg.layers.keys().cloned().collect::<Vec<_>>().join(", ")
        ))
    })?;

    let edit = capture::add_package(&target.path, package, dry)?;

    println!("{}", edit.file.display());
    println!("{}", edit.summary());
    if dry {
        println!("\n--dry-run: nothing was written.");
    } else {
        println!("\nWritten. Review with `git diff` before committing.");
    }
    Ok(())
}
