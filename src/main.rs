#![allow(dead_code)] // parts of the config model are for the next milestone

mod apply;
mod capture;
mod cli;
mod color;
mod config;
mod diff;
mod error;
mod groups;
mod migrate;
mod pacman;
mod perms;
mod render;
mod repo;
mod secret;
mod state;
mod systemd;
mod write;

use std::fs;
use std::path::{Path, PathBuf};

use clap::Parser;
use miette::Result;

use crate::cli::{Cli, Command};
use crate::config::Config;
use crate::error::Error;

/// Where the config repo lives when nothing says otherwise.
///
/// The home directory is NOT taken from `$HOME`: under sudo that may still be
/// the caller's home (`env_keep` / `always_set_home`), and this tool will run
/// as root.
fn default_config_dir(home: &Path) -> PathBuf {
    if let Ok(x) = std::env::var("XDG_CONFIG_HOME") {
        if !x.is_empty() {
            return PathBuf::from(x).join("lami");
        }
    }
    home.join(".config").join("lami")
}

/// Find the config repo, in order of how explicit the answer is:
///
///   1. `--config-dir` (or `LAMI_CONFIG_DIR`)
///   2. `path` in ~/.config/lami.kdl
///   3. ~/.config/lami
///
/// If it is missing but a repo URL is known -- from `--repo` or from the same
/// pointer file -- clone it. That is what lets a machine with nothing on it
/// run `lami --repo <url> apply`.
fn resolve_config_dir(cli: &Cli, home: &Path) -> Result<PathBuf, Error> {
    let pointer = repo::Pointer::load(home)?;
    let dir = cli
        .config_dir
        .clone()
        .or_else(|| pointer.path.clone())
        .unwrap_or_else(|| default_config_dir(home));

    if dir.is_dir() {
        return Ok(dir);
    }

    match cli.repo.clone().or(pointer.repo) {
        Some(url) => {
            repo::refuse_root("lami --repo")?;
            println!("{}", color::bold("config repo"));
            repo::clone(&url, &dir)?;
            println!("  {}\n", dir.display());
            Ok(dir)
        }
        None => Err(Error::Other(format!(
            "no config repo at {}\n\
             \n  lami clone <git-url>            clone it and remember where\n\
             \n  lami --repo <git-url> <cmd>    use one without recording it\n\
             \n  lami --config-dir <path> <cmd>  point at a directory you already have",
            dir.display()
        ))),
    }
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
        return Err(Error::Other(format!("no passwd entry for uid {uid}")));
    }
    let dir = unsafe { std::ffi::CStr::from_ptr((*pw).pw_dir) };
    Ok(PathBuf::from(dir.to_str().map_err(|_| {
        Error::Other("home directory path is not valid UTF-8".into())
    })?))
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
    color::init(cli.no_color);
    let home = real_home()?;

    // `clone` is the one command that runs before there is a config to load.
    if let Command::Clone { url, path } = &cli.command {
        return cmd_clone(url, path.clone(), &home).map_err(Into::into);
    }

    let dir = resolve_config_dir(&cli, &home)?;

    // Syncing the repo deliberately does not parse it. A config that does not
    // load is exactly when you most want to pull the fix, or push the broken
    // state to look at it elsewhere.
    match &cli.command {
        Command::Pull => return cmd_pull(&dir).map_err(Into::into),
        Command::Push { message } => return cmd_push(&dir, message.as_deref()).map_err(Into::into),
        _ => {}
    }

    let cfg = Config::load(&dir, &home)?;

    match cli.command {
        Command::Clone { .. } => unreachable!("handled before the config is loaded"),
        Command::Pull | Command::Push { .. } => {
            unreachable!("handled before the config is loaded")
        }
        Command::Init { name, layers, like } => cmd_init(&cfg, &name, &layers, like.as_deref())?,
        Command::List => cmd_list(&cfg),
        Command::Show => cmd_show(&cfg, cli.host.unwrap_or_else(hostname))?,
        Command::Why { target } => cmd_why(&cfg, cli.host.unwrap_or_else(hostname), &target)?,
        Command::Check => cmd_check(&cfg, cli.host.unwrap_or_else(hostname))?,
        Command::Apply { dry_run } => cmd_apply(&cfg, cli.host.unwrap_or_else(hostname), dry_run)?,
        Command::Capture {
            package,
            all,
            file,
            layer,
            dry_run,
        } => cmd_capture(
            &cfg,
            cli.host.unwrap_or_else(hostname),
            &package,
            all,
            file.as_deref(),
            layer.as_deref(),
            dry_run,
        )?,
        Command::Prune {
            dry_run,
            force,
            yes,
        } => cmd_prune(&cfg, cli.host.unwrap_or_else(hostname), dry_run, force, yes)?,
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

    println!("{}", color::bold("hosts:"));
    if cfg.hosts.is_empty() {
        println!("  (none)");
    }
    for h in cfg.hosts.values() {
        let desc = h.description.as_deref().unwrap_or("");
        println!("  {:<12} {}", h.name, desc);
    }

    println!("\n{}", color::bold("layers:"));
    if cfg.layers.is_empty() {
        println!("  (none)");
    }
    for l in cfg.layers.values() {
        let desc = l.description.as_deref().unwrap_or("");
        println!("  {:<12} {}", l.name, desc);
    }
}

/// Write a host file for a new machine.
///
/// The value is not the file -- it is that the layers are asked what they need
/// to know, so the new host file arrives already listing every parameter that
/// has to be answered, with the description the layer gave it. Copying an
/// existing host file instead means inheriting its answers along with any
/// parameter that has since stopped mattering.
fn cmd_init(cfg: &Config, name: &str, layers: &[String], like: Option<&str>) -> Result<(), Error> {
    let file = cfg.dir.join("hosts").join(format!("{name}.kdl"));
    if file.exists() {
        return Err(Error::Other(format!(
            "{} already exists.\n\
             Remove it first if you mean to start over.",
            file.display()
        )));
    }

    // Expand `needs`, keeping dependency order, so the parameters come out in
    // the order the layers were built in rather than alphabetically.
    let mut ordered: Vec<String> = Vec::new();
    for l in layers {
        expand_for_init(cfg, l, &mut ordered)?;
    }
    let resolved: Vec<&config::Layer> = ordered
        .iter()
        .map(|n| cfg.layers.get(n).expect("checked while expanding"))
        .collect();

    let borrowed: Vec<&str> = layers.iter().map(String::as_str).collect();
    let previous = match like {
        Some(h) => Some(cfg.resolve(h)?),
        None => None,
    };

    let mut required: Vec<(&str, &config::ParamDecl)> = Vec::new();
    let mut optional: Vec<(&str, &config::ParamDecl)> = Vec::new();
    let mut seen: std::collections::BTreeSet<&str> = Default::default();
    for l in &resolved {
        for p in &l.params {
            if !seen.insert(p.name.as_str()) {
                continue;
            }
            if p.default.is_some() {
                optional.push((&l.name, p));
            } else {
                required.push((&l.name, p));
            }
        }
    }

    let mut out = String::new();
    out.push_str(&format!(
        "description \"TODO: what this machine is\"\n\n\
         // The layers this host gets. Their dependencies are added\n\
         // automatically, so `needs` never has to be repeated here.\n\
         layers {}\n",
        borrowed
            .iter()
            .map(|l| format!("\"{l}\""))
            .collect::<Vec<_>>()
            .join(" ")
    ));

    if !required.is_empty() {
        out.push_str(
            "\n// Required. Every one of these has to be answered before\n\
             // `lami diff` will run: there is no default profile here.\n",
        );
        for (layer, p) in &required {
            out.push_str(&format!("\n// {} ({layer})\n", p.description));
            if !p.one_of.is_empty() {
                out.push_str(&format!("// one of: {}\n", p.one_of.join(" ")));
            }
            let existing = previous
                .as_ref()
                .and_then(|r| r.host.params.get(p.name.as_str()));
            out.push_str(&param_line(&p.name, existing, p.list));
        }
    }

    if !optional.is_empty() {
        out.push_str(
            "\n// Optional: these have defaults, shown here commented out.\n\
             // Uncomment a line only to change it -- a line you leave out\n\
             // means the layer's default, which is the same thing.\n",
        );
        for (layer, p) in &optional {
            out.push_str(&format!("\n// {} ({layer})\n", p.description));
            if !p.one_of.is_empty() {
                out.push_str(&format!("// one of: {}\n", p.one_of.join(" ")));
            }
            let d = p.default.as_ref().expect("optional means it has one");
            out.push_str(&format!(
                "// {}\n",
                param_line(&p.name, Some(d), p.list).trim_end()
            ));
        }
    }

    std::fs::create_dir_all(file.parent().expect("hosts/ has a parent")).map_err(|source| {
        Error::Io {
            path: file.parent().unwrap().to_path_buf(),
            source,
        }
    })?;
    std::fs::write(&file, &out).map_err(|source| Error::Io {
        path: file.clone(),
        source,
    })?;

    println!("{}", color::bold(&format!("wrote {}", file.display())));
    println!(
        "  {}",
        color::dim(&format!(
            "layers: {} ({} after `needs`)",
            borrowed.join(", "),
            resolved.len()
        ))
    );
    let copied = |p: &config::ParamDecl| {
        previous
            .as_ref()
            .is_some_and(|r| r.host.params.contains_key(p.name.as_str()))
    };
    let blank: Vec<&(&str, &config::ParamDecl)> =
        required.iter().filter(|(_, p)| !copied(p)).collect();

    let describe = |layer: &str, p: &config::ParamDecl| {
        let allowed = if p.one_of.is_empty() {
            String::new()
        } else {
            format!("  [{}]", p.one_of.join(" "))
        };
        format!(
            "  {:<14} {}{}",
            p.name,
            color::dim(&format!("{layer}: {}", p.description)),
            color::dim(&allowed)
        )
    };

    if let Some(from) = like {
        let taken = required.len() - blank.len();
        if taken > 0 {
            println!(
                "  {}",
                color::dim(&format!(
                    "{taken} answer(s) copied from {from} -- check them"
                ))
            );
        }
    }

    if blank.is_empty() {
        println!("\nNext: lami diff --host {name}");
    } else {
        println!("\n{}", color::bold("to fill in:"));
        for (layer, p) in &blank {
            println!("{}", describe(layer, p));
        }
        println!("\nThen: lami diff --host {name}");
    }
    Ok(())
}

/// One `name value` line, in the spelling the parser expects back.
fn param_line(name: &str, value: Option<&config::Value>, list: bool) -> String {
    match value {
        Some(config::Value::List(items)) => {
            let body: String = items.iter().map(|i| format!("    \"{i}\"\n")).collect();
            format!("{name} {{\n{body}}}\n")
        }
        // A list parameter written as a bare string would be read as a string:
        // KDL cannot tell one from a list of one, so the block form is the
        // only correct empty value.
        None if list => format!("{name} {{\n    \"\"\n}}\n"),
        Some(config::Value::Int(i)) => format!("{name} {i}\n"),
        Some(config::Value::Bool(b)) => {
            format!("{name} {}\n", if *b { "on" } else { "off" })
        }
        Some(v) => format!("{name} \"{v}\"\n"),
        None => format!("{name} \"\"\n"),
    }
}

/// Layer expansion for `init`, which has no host to blame an unknown name on.
fn expand_for_init(cfg: &Config, name: &str, out: &mut Vec<String>) -> Result<(), Error> {
    if out.iter().any(|n| n == name) {
        return Ok(());
    }
    let layer = cfg.layers.get(name).ok_or_else(|| {
        Error::Other(format!(
            "no layer called '{name}'.\nKnown layers: {}",
            cfg.layers.keys().cloned().collect::<Vec<_>>().join(", ")
        ))
    })?;
    for dep in &layer.needs {
        expand_for_init(cfg, dep, out)?;
    }
    out.push(name.to_string());
    Ok(())
}

/// Clone the config repo and record where it went.
fn cmd_clone(url: &str, path: Option<PathBuf>, home: &Path) -> Result<(), Error> {
    repo::refuse_root("lami clone")?;
    let dest = path.unwrap_or_else(|| default_config_dir(home));

    println!("{}", color::bold("config repo"));
    repo::clone(url, &dest)?;

    let ptr = repo::Pointer {
        repo: Some(url.to_string()),
        path: Some(dest.clone()),
    };
    let file = ptr.save(home)?;

    println!("  {}", dest.display());
    println!(
        "  {}",
        color::dim(&format!("recorded in {}", file.display()))
    );

    // Say straight away whether this machine is described, because that is
    // the next thing that can go wrong and it costs nothing to check.
    match Config::load(&dest, home) {
        Ok(cfg) => {
            let h = hostname();
            if cfg.hosts.contains_key(&h) {
                println!("\n  this host ({h}) is described. Next:\n");
                println!("    lami diff");
                println!("    sudo lami apply");
            } else {
                let known: Vec<&str> = cfg.hosts.keys().map(String::as_str).collect();
                println!(
                    "\n{}",
                    color::changed(&format!("  this host ({h}) is not in hosts/ yet"))
                );
                println!("  known hosts: {}", known.join(", "));
                println!("\n    $EDITOR {}/hosts/{h}.kdl", dest.display());
            }
        }
        Err(e) => {
            println!(
                "\n{}",
                color::changed("  cloned, but the config does not parse:")
            );
            println!("  {e}");
        }
    }
    Ok(())
}

/// Fast-forward the config repo.
fn cmd_pull(dir: &Path) -> Result<(), Error> {
    repo::refuse_root("lami pull")?;
    println!("{}", color::bold(&format!("pull {}", dir.display())));
    repo::pull(dir)?;
    println!("\nRun `lami diff` to see what the new config would change here.");
    Ok(())
}

/// Commit and push the config repo.
fn cmd_push(dir: &Path, message: Option<&str>) -> Result<(), Error> {
    repo::refuse_root("lami push")?;
    println!("{}", color::bold(&format!("push {}", dir.display())));
    repo::push(dir, message, &hostname())?;
    Ok(())
}

fn cmd_show(cfg: &Config, host: String) -> Result<(), Error> {
    let r = cfg.resolve(&host)?;

    println!("{}", color::bold(&format!("host: {}", r.host.name)));
    if let Some(d) = &r.host.description {
        println!("     {d}");
    }
    println!("     {}", r.host.origin.display());

    if let Some(sum) = repo::summary(&cfg.dir) {
        println!("\n{}", color::bold("config repo:"));
        println!("  {}", cfg.dir.display());
        println!("  {}", color::dim(&sum));
    }

    println!("\n{}", color::bold("parameters:"));
    let wide = r
        .params
        .keys()
        .map(|k| k.len())
        .max()
        .unwrap_or(0)
        .clamp(8, 20);
    for (k, v) in &r.params {
        if k == "hostname" {
            continue;
        }
        let value = v.to_string();
        let note = match r.declared.get(k) {
            Some((layer, decl)) => {
                let from_default = !r.host.params.contains_key(k);
                color::dim(&format!(
                    "{}{layer}: {}",
                    if from_default { "default, " } else { "" },
                    decl.description
                ))
            }
            // Not an error: a template may read it, or a layer this host does
            // not enable may declare it. But it should not be invisible.
            None => color::changed("no enabled layer declares this"),
        };
        println!("  {k:<wide$}  {value:<18}  {note}");
    }

    println!(
        "\n{}",
        color::bold("layers (resolved, in dependency order):")
    );
    for l in &r.layers {
        let explicit = if r.host.layers.contains(&l.name) {
            ""
        } else {
            "  (via needs)"
        };
        println!("  {:<12} {}{}", l.name, l.path.display(), explicit);
    }

    println!("\n{}", color::bold("resources for this host:"));
    println!("  packages   {}", r.packages().len());
    println!("  groups     {}", r.groups().len());
    println!("  services   {}", r.services().len());
    let m = r.migrations().len();
    if m > 0 {
        println!("  migrations {m}");
    }
    Ok(())
}

fn cmd_why(cfg: &Config, host: String, target: &str) -> Result<(), Error> {
    let r = cfg.resolve(&host)?;

    let mut found = false;

    for (layer, m) in r.migrations() {
        if m.name != target {
            continue;
        }
        found = true;
        let done = state::load().migrations;
        println!("{}  {}", color::bold(&m.name), color::dim("(migration)"));
        println!("  declared:   {}", m.origin);
        println!("  applies:    layer '{}'", layer.name);
        println!("  script:     {}", m.script.display());
        println!("  because:    {}", m.because);
        match done.get(&m.name) {
            Some(recorded) => {
                let now = crate::migrate::checksum(&m.script).unwrap_or_default();
                if now == *recorded {
                    println!("  status:     already run on this machine");
                } else {
                    println!(
                        "  status:     ran on this machine, but the script has been edited since"
                    );
                }
            }
            None => println!("  status:     has not run on this machine"),
        }
        println!();
    }
    for (kind, items) in [("package", r.packages()), ("group", r.groups())] {
        for (layer, decl) in items {
            if decl.name != target {
                continue;
            }
            found = true;
            println!(
                "{}  {}",
                color::bold(&decl.name),
                color::dim(&format!("({kind})"))
            );
            println!("  declared:   {}", decl.origin);
            print!("  applies:    layer '{}'", layer.name);
            if r.host.layers.contains(&layer.name) {
                println!(" is in {}'s layers list", r.host.name);
            } else {
                println!(" is pulled in via needs");
            }
            if let Some(c) = &decl.condition {
                let val = r
                    .params
                    .get(&c.key)
                    .map(|v| v.to_string())
                    .unwrap_or_default();
                println!("  condition:  {} (this host: {} = {})", c, c.key, val);
            }
            println!();
        }
    }

    for (layer, d) in r.services() {
        if d.name != target && crate::systemd::qualify(&d.name) != target {
            continue;
        }
        found = true;
        let scope = match d.scope {
            crate::config::Scope::System => "system",
            crate::config::Scope::User => "user",
        };
        println!(
            "{}  {}",
            color::bold(&systemd::qualify(&d.name)),
            color::dim(&format!("({scope} unit)"))
        );
        println!("  declared:   {}", d.origin);
        println!("  applies:    layer '{}'", layer.name);
        println!("  wanted:     {}", d.state);
        if d.restart_on_change {
            println!("  restart:    when lami rewrites its unit file");
        }
        if let Some(c) = &d.condition {
            println!("  condition:  {c}");
        }
        println!();
    }

    let user = real_user()?;
    let home = real_home()?;
    let forms = path_forms(target, &home);
    for (layer, f) in r.files() {
        if !forms.iter().any(|p| *p == f.path) {
            continue;
        }
        found = true;
        let pm = perms::with_overrides(
            perms::secret_aware(f, &user),
            f.owner.as_deref(),
            f.group.as_deref(),
            f.mode,
        );
        println!("{}  {}", color::bold(&f.path), color::dim("(file)"));
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
        if !h.watch.iter().any(|w| forms.iter().any(|p| p == w)) {
            continue;
        }
        found = true;
        println!(
            "{}  {}",
            color::bold(target),
            color::dim("(watched by a hook)")
        );
        println!("  declared:   {}", h.origin);
        println!("  applies:    layer '{}'", layer.name);
        println!("  runs:       {}", h.run);
        println!();
    }

    if !found {
        // "Is this file managed?" is a question worth answering properly,
        // because the useful reply is never just "no".
        if target.contains('/') {
            println!(
                "{}",
                color::bold(&format!("{target}  (not managed by lami)"))
            );
            let exists = Path::new(&forms[forms.len() - 1]).exists() || Path::new(target).exists();
            if exists {
                println!("  the file exists, but no layer this host enables declares it");
                println!(
                    "\n  {}",
                    color::dim("lami render --list      every path that IS managed")
                );
                println!(
                    "  {}",
                    color::dim("lami capture --file <path>   only works once a layer declares it")
                );
            } else {
                println!("  no layer declares it, and there is no such file on this machine");
                println!(
                    "\n  {}",
                    color::dim("lami render --list      every path that IS managed")
                );
            }
        } else {
            println!("'{target}' is not declared for {}.", r.host.name);
            println!("\nCheck `lami show`, or it may only apply to another host.");
        }
    }
    Ok(())
}

/// The forms a managed path can be typed in.
///
/// The config writes a home path as `~/.config/fish/config.fish`, but the
/// shell expands `~` long before lami sees it, and tab completion produces
/// the absolute form. Asking about a file you can see on disk has to work.
fn path_forms(target: &str, home: &Path) -> Vec<String> {
    let mut out = vec![target.to_string()];
    if let Some(rest) = target.strip_prefix("~/") {
        out.push(home.join(rest).display().to_string());
    } else if let Ok(rest) = Path::new(target).strip_prefix(home) {
        out.push(format!("~/{}", rest.display()));
    }
    out
}

/// Check the config against the system.
///
/// This makes up for what we lost when the separate `aur` block went away:
/// since the config no longer states which packages come from the AUR, we have
/// to CHECK that a helper is present -- otherwise apply would only find out at
/// install time.
fn cmd_check(cfg: &Config, host: String) -> Result<(), Error> {
    let r = cfg.resolve(&host)?;
    println!("{}\n", color::bold(&format!("host: {}", r.host.name)));

    // A migration that has run and has since been edited. Not an error and
    // not something to re-run behind your back -- a one-off is a one-off --
    // but the repo and the machine now disagree about what actually happened,
    // and only this can tell you.
    let done = state::load().migrations;
    let edited: Vec<(&str, &str)> = r
        .migrations()
        .into_iter()
        .filter_map(|(_, m)| {
            let recorded = done.get(&m.name)?;
            let now = crate::migrate::checksum(&m.script).ok()?;
            (now != *recorded).then_some((m.name.as_str(), m.because.as_str()))
        })
        .collect();
    if !edited.is_empty() {
        println!("{}", color::changed("migrations edited since they ran:"));
        for (name, because) in &edited {
            println!("  {name}   {}", color::dim(because));
        }
        println!(
            "{}\n",
            color::dim(
                "  They will not run again: a one-off is keyed by its name, so renaming\n\
                 \x20 one is how you ask for it to happen a second time."
            )
        );
    }

    // Anything the host answers that nothing asked. Resolving already rejects
    // the opposite case -- a layer that needs something the host never set --
    // so this is the half that cannot be an error: the parameter may be read
    // by a template, or by a layer this host does not currently enable.
    let unread: Vec<&String> = r
        .host
        .params
        .keys()
        .filter(|k| !r.declared.contains_key(*k))
        .collect();
    if !unread.is_empty() {
        println!("{}", color::changed("parameters nothing reads:"));
        for k in &unread {
            println!("  {k}");
        }
        println!(
            "{}\n",
            color::dim(
                "  No enabled layer declares these. Either a layer should say it\n\
                 \x20 needs them in its `params` block, or the lines can go."
            )
        );
    }

    if !pacman::available() {
        println!("pacman is not available, skipping the package check.");
        println!("(lami targets Arch Linux; the config structure was still validated.)");
        return Ok(());
    }

    let declared: Vec<&str> = r.packages().iter().map(|(_, d)| d.name.as_str()).collect();
    let sync = pacman::sync_packages()?;

    let (from_repo, unknown): (Vec<&str>, Vec<&str>) =
        declared.iter().partition(|p| sync.contains(**p));

    println!("{}", color::bold("packages:"));
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
                perms::secret_aware(f, &user),
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
        let content = render::file(&r, layer, f, &cfg.settings, &home, &real_user()?)?;

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
                println!("{}", color::heading(&format!("=== {}", f.path)));
                println!(
                    "{}",
                    color::dim(&format!(
                        "    layer {}, declared at {}",
                        layer.name, f.origin
                    ))
                );
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
    let report = diff::compute(&r, &home, &real_user()?, &cfg.settings)?;

    println!("{}\n", color::bold(&format!("host: {}", r.host.name)));

    if report.changes.is_empty() {
        if !report.problems.is_empty() {
            println!("No changes to make -- but see below.");
        } else if report.undeclared_packages.is_empty() {
            println!("Nothing to do -- the machine matches the config.");
        } else {
            // Not the same sentence. `apply` never removes, so a package no
            // layer declares is nothing for it to do -- but saying "the
            // machine matches the config" would be claiming more than that.
            println!("Nothing to apply -- everything the config declares is in place.");
        }
    } else {
        diff::print(&report.changes);
        let n = report.changes.len();
        println!(
            "\n{}",
            color::dim(&format!(
                "{n} change{}. Nothing has been applied -- run `sudo lami apply` to do so.",
                if n == 1 { "" } else { "s" }
            ))
        );
    }

    diff::print_problems(&report.problems);

    // The other direction: installed here, declared nowhere. Never a change,
    // because apply does not remove -- but a new machine built from this
    // config would not have them, and that is worth one line.
    if !show_undeclared && !report.undeclared_packages.is_empty() {
        let n = report.undeclared_packages.len();
        println!(
            "{}",
            color::dim(&format!(
                "({n} installed package{} declared by no layer -- `lami capture` files them, \
                 `lami diff --undeclared` lists them)",
                if n == 1 { "" } else { "s" }
            ))
        );
    }

    // Kept to one line. Silently claiming a match for something we could not
    // read would be dishonest, but four lines of it on every single run is
    // noise -- and `sudo lami diff` both answers the question and makes the
    // notice disappear.
    if !report.skipped.is_empty() {
        let n = report.skipped.len();
        let needs_root = report
            .skipped
            .iter()
            .all(|s| s.contains("not readable as this user"));
        println!(
            "{}",
            color::dim(&if needs_root {
                format!(
                    "({n} file{} need root to compare; `sudo lami diff` includes them)",
                    if n == 1 { "" } else { "s" }
                )
            } else {
                format!(
                    "({n} item{} not compared: {})",
                    if n == 1 { "" } else { "s" },
                    report.skipped.join(", ")
                )
            })
        );
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

        // The same question for group membership, which is otherwise invisible
        // in both directions: apply does not remove it and prune only touches
        // what lami added, so a membership from some forgotten manual step
        // could sit there forever with nothing ever mentioning it.
        println!("\na member of, but declared by no layer:");
        if report.undeclared_groups.is_empty() {
            println!("  (none)");
        } else {
            for g in &report.undeclared_groups {
                println!("  {g}");
            }
            println!(
                "\n{} group(s), primary group excluded. Either a layer should declare\n\
                 them, or they are left over from a manual step and nothing needs them.",
                report.undeclared_groups.len()
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
    let uid = write::uid_of(&name).ok_or_else(|| Error::Other(format!("no such user: {name}")))?;
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

    println!("{}\n", color::bold(&format!("host: {}", r.host.name)));
    let n = apply::run_apply(&r, &actor, dry, &cfg.settings)?;
    if n > 0 && !dry {
        println!("\nApplied {n} change(s).");
    }
    Ok(())
}

/// Pull a change made on this machine back into the repo.
fn cmd_capture(
    cfg: &Config,
    host: String,
    packages: &[String],
    all: bool,
    file: Option<&str>,
    layer: Option<&str>,
    dry: bool,
) -> Result<(), Error> {
    let r = cfg.resolve(&host)?;

    if let Some(path) = file {
        let home = real_home()?;
        let forms = path_forms(path, &home);
        let (_, decl) = r
            .files()
            .into_iter()
            .find(|(_, f)| forms.iter().any(|p| *p == f.path))
            .ok_or_else(|| {
                Error::Other(format!(
                    "'{path}' is not a file managed for {}.\n\
                     Run `lami render --list` to see the managed paths.",
                    r.host.name
                ))
            })?;
        let live = render::target_path(decl, &home);
        let edit = capture::capture_file(decl, &live, dry)?;
        println!(
            "{}  {}  {}",
            edit.file.display(),
            color::dim("<-"),
            live.display()
        );
        println!("{}", colourise_summary(&edit.summary()));
        if dry {
            println!("\n--dry-run: nothing was written.");
        } else {
            println!("\nWritten. Review with `git diff` before committing.");
        }
        return Ok(());
    }

    // `--all` means every package the machine has that no layer declares.
    let packages: Vec<String> = if all {
        let report = diff::compute(&r, &real_home()?, &real_user()?, &cfg.settings)?;
        if report.undeclared_packages.is_empty() {
            println!("Nothing to capture -- every installed package is declared.");
            return Ok(());
        }
        report.undeclared_packages.clone()
    } else {
        packages.to_vec()
    };

    if packages.is_empty() {
        // A layer with nothing to put in it is the one combination that used
        // to do nothing at all and say nothing about it.
        if let Some(l) = layer {
            let report = diff::compute(&r, &real_home()?, &real_user()?, &cfg.settings)?;
            let names = report.undeclared_packages.join(",");
            return Err(Error::Other(if names.is_empty() {
                format!("--layer {l} names a layer, but not what to put in it.")
            } else {
                format!(
                    "--layer {l} names a layer, but not what to put in it.\n\
                     \n  lami capture --layer {l} --package {names}\n\
                     \nor, for everything this machine has that no layer declares:\n\
                     \n  lami capture --layer {l} --all"
                )
            }));
        }

        // No target named: show what is on offer.
        let report = diff::compute(&r, &real_home()?, &real_user()?, &cfg.settings)?;
        println!("{}\n", color::bold(&format!("host: {}", r.host.name)));

        let changed: Vec<&diff::Change> = report
            .changes
            .iter()
            .filter(|c| matches!(c, diff::Change::UpdateFile { .. }))
            .collect();

        if report.undeclared_packages.is_empty() && changed.is_empty() {
            println!("Nothing to capture -- the machine matches the config.");
            return Ok(());
        }

        if !changed.is_empty() {
            println!("managed files that differ on this machine:");
            for c in &changed {
                if let diff::Change::UpdateFile { path, .. } = c {
                    println!("  {path}");
                }
            }
            println!("\nPull one back with:");
            println!("  lami capture --file <path>");
            println!();
        }

        if report.undeclared_packages.is_empty() {
            return Ok(());
        }
        println!("explicitly installed but declared by no layer:");
        for p in &report.undeclared_packages {
            println!("  {p}");
        }
        let layers = r
            .layers
            .iter()
            .map(|l| l.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");

        // A command that can be pasted, with a name that is actually in the
        // list above it.
        println!("\nFile them into a layer with:");
        println!(
            "  lami capture --layer <layer> --package {}",
            report.undeclared_packages.join(",")
        );
        println!(
            "  lami capture --layer <layer> --all      {}",
            color::dim("(the same thing, for all of them)")
        );
        println!("\nlayers on this host: {layers}");
        return Ok(());
    }

    let Some(layer_name) = layer else {
        return Err(Error::Other(format!(
            "which layer should {} go in?\n\
             \n  lami capture --layer <layer> --package {}\n\
             \nlayers on this host: {}",
            packages.join(", "),
            packages.join(","),
            r.layers
                .iter()
                .map(|l| l.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )));
    };

    let target = cfg.layers.get(layer_name).ok_or_else(|| {
        Error::Other(format!(
            "no layer named '{layer_name}'.\nknown layers: {}",
            cfg.layers.keys().cloned().collect::<Vec<_>>().join(", ")
        ))
    })?;

    // One at a time, re-reading the file between each, so the second package
    // is inserted into the layer the first one just changed rather than into a
    // stale copy of it.
    println!("{}", target.path.display());
    for p in &packages {
        let edit = capture::add_package(&target.path, p, dry)?;
        println!("{}", colourise_summary(&edit.summary()));
    }
    if dry {
        println!("\n--dry-run: nothing was written.");
    } else {
        println!("\nWritten. Review with `git diff` before committing.");
    }
    Ok(())
}

/// Remove what lami used to manage but no longer declares.
fn cmd_prune(cfg: &Config, host: String, _dry: bool, force: bool, yes: bool) -> Result<(), Error> {
    let r = cfg.resolve(&host)?;
    let name = real_user()?;
    let home = real_home()?;
    let actor = apply::Actor {
        uid: write::uid_of(&name).unwrap_or(0),
        gid: 0,
        name,
        home,
    };

    let stale = apply::stale(&r, &actor);
    println!("{}\n", color::bold(&format!("host: {}", r.host.name)));

    if stale.is_empty() {
        println!("Nothing to prune -- everything lami manages is still declared.");
        return Ok(());
    }

    for p in &stale.packages {
        println!("  {} {p}", color::removed("- package"));
    }
    for g in &stale.groups {
        println!("  {} {g}", color::removed("- group  "));
    }
    for s in &stale.services {
        println!("  {} {s}", color::removed("- service"));
    }
    for s in &stale.user_services {
        println!(
            "  {} {s}  {}",
            color::removed("- service"),
            color::dim("(user)")
        );
    }
    for f in &stale.files {
        println!("  {} {f}", color::removed("- file   "));
    }
    println!(
        "\n{} item(s) lami used to manage and no longer declares.",
        stale.len()
    );

    // Dry run is the DEFAULT. Removal has to be asked for twice: once by
    // choosing this command at all, and once by saying --force. A tool that
    // deletes because you typed the wrong subcommand is not one to trust with
    // root.
    if !force {
        println!("\nNothing was removed. To go ahead:\n\n  sudo lami prune --force");
        return Ok(());
    }

    apply::require_root()?;

    if !yes {
        print!("\nRemove these {} item(s)? [y/N] ", stale.len());
        use std::io::Write as _;
        std::io::stdout().flush().ok();
        let mut answer = String::new();
        std::io::stdin().read_line(&mut answer).ok();
        if !matches!(answer.trim(), "y" | "Y") {
            println!("Cancelled.");
            return Ok(());
        }
    }

    println!();
    apply::run_prune(&stale, &actor)?;
    println!("\nPruned {} item(s).", stale.len());
    Ok(())
}

/// Colour a capture summary the way a diff reads: additions green, removals red.
fn colourise_summary(text: &str) -> String {
    text.lines()
        .map(|l| match l.chars().next() {
            Some('+') => color::added(l),
            Some('-') => color::removed(l),
            _ => l.to_string(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}
